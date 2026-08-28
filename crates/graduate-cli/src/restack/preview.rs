//! Machine preview and conflict preservation.

use std::path::Path;

use graduate::restack::{build_plan, select_features, RestackSnapshot};
use serde_json::json;

use super::errors::{conflict_error, inspection_error, plan_error, selection_error, session_error};
use super::isolated::{FreshRestack, IsolatedRepository, ReconstructionResult};
use super::machine_output::{machine_failure, write_apply_result, write_plan};
use super::plan_validation::{authorize_plan, remote_environment_ref, revalidate_plan};
use super::source::{configured_author, source_object_directory, source_repository_identity};
use super::{parse_params, validate_apply_params, INSPECTION_OBJECT_CACHE_BYTES};
use crate::cli::RestackArgs;
use crate::restack::session::{SessionConflict, SessionDraft, SessionMetadata, SessionStore};
use crate::shared::environment_git::{inspect_environment, restack_snapshot};
use crate::shared::error::CliError;
use crate::shared::git_process;

pub(super) fn preview(
    args: &RestackArgs,
    source: &Path,
    sessions: &SessionStore,
) -> Result<(), CliError> {
    let params = parse_params(args.params.as_deref(), args.dry_run)?;
    let remote = args.remote.as_deref().unwrap_or("origin");
    validate_apply_params(args.apply, &params)?;
    let remote_endpoint = git_process::resolve_restack_remote(remote, source).map_err(|_| {
        machine_failure(
            "remote_unavailable",
            "could not resolve one safe endpoint for the selected remote",
            json!({"remote": remote}),
        )
    })?;

    git_process::fetch_restack_remote(&remote_endpoint, remote, source, false).map_err(|_| {
        machine_failure(
            "fetch_failed",
            "could not fetch the selected remote",
            json!({"remote": remote}),
        )
    })?;

    let mut repository = gix::discover(source).map_err(|_| {
        machine_failure(
            "repository_not_found",
            "the current directory is not inside a Git repository",
            json!({}),
        )
    })?;
    repository.object_cache_size_if_unset(INSPECTION_OBJECT_CACHE_BYTES);
    let inspection =
        inspect_environment(&repository, remote, &args.environment, args.main.as_deref()).map_err(
            |_| {
                machine_failure(
                    "inspection_failed",
                    "could not inspect the fetched environment refs",
                    json!({"stage": "refs"}),
                )
            },
        )?;
    let snapshot = restack_snapshot(&repository, &inspection).map_err(inspection_error)?;
    let removals = with_tainted_removals(&snapshot, &params.remove_branches);
    let selection = select_features(&snapshot, &removals).map_err(selection_error)?;
    let author = configured_author(source)?;
    let repository_id = source_repository_identity(source)?;
    let source_objects = source_object_directory(source)?;
    let draft = sessions.begin().map_err(session_error)?;
    let isolated = IsolatedRepository::create(&draft.repository(), &source_objects)?;
    isolated.train_resolutions(&snapshot, &selection.retained, &author)?;
    let reconstruction = isolated.reconstruct(
        &snapshot.main_tip,
        &snapshot.environment,
        &selection.retained,
        &author,
        0,
        Vec::new(),
    )?;
    finish_or_preserve(
        reconstruction,
        draft,
        FreshRestack {
            isolated: &isolated,
            repository_id,
            snapshot,
            remote_endpoints: remote_endpoint.identity(),
            author,
            selection,
            apply_digest: if args.apply {
                params.plan_digest.as_deref()
            } else {
                None
            },
            source,
            remote: &remote_endpoint,
        },
    )
}

fn finish_or_preserve(
    reconstruction: ReconstructionResult,
    mut draft: SessionDraft,
    fresh: FreshRestack<'_>,
) -> Result<(), CliError> {
    match reconstruction {
        ReconstructionResult::Complete(reconstruction) => {
            let plan = build_plan(
                fresh.snapshot,
                fresh.remote_endpoints,
                fresh.author,
                fresh.selection,
                reconstruction,
                Vec::new(),
            )
            .map_err(plan_error)?;
            if let Some(digest) = fresh.apply_digest {
                authorize_plan(&plan, Some(digest))?;
                revalidate_plan(fresh.source, fresh.remote, &plan)?;
                fresh.isolated.validate_publication_plan(&plan)?;
                git_process::push_restack_commit(
                    fresh.remote,
                    &fresh.isolated.root,
                    &fresh.isolated.hooks,
                    &fresh.isolated.global_config,
                    &plan.preview_commit,
                    &remote_environment_ref(&plan.snapshot.environment),
                    &plan.snapshot.environment_tip,
                )
                .map_err(|_| {
                    machine_failure(
                        "push_rejected",
                        "the remote rejected the exact leased environment update",
                        json!({"environment": plan.snapshot.environment}),
                    )
                })?;
                draft.discard().map_err(session_error)?;
                return write_apply_result(&plan);
            }
            draft.discard().map_err(session_error)?;
            write_plan(&plan)
        }
        ReconstructionResult::Conflict(conflict) => {
            let metadata = SessionMetadata::conflicted(
                fresh.repository_id,
                fresh.snapshot,
                fresh.remote_endpoints,
                fresh.author,
                fresh.selection,
                Vec::new(),
                SessionConflict {
                    merges: conflict.merges,
                    next_feature: conflict.feature_index,
                    expected_head: conflict.expected_head,
                    expected_head_reflog: conflict.expected_head_reflog,
                    expected_feature_tip: conflict.feature.tip.clone(),
                },
            )
            .map_err(session_error)?;
            draft.save(&metadata).map_err(session_error)?;
            Err(conflict_error(
                &conflict.feature.name,
                conflict.unresolved_paths,
                &draft.token(),
                &draft.repository(),
                metadata.expires_at,
            ))
        }
    }
}

/// Every tainted feature is removed by default; explicit removals are kept
/// in order and never duplicated.
fn with_tainted_removals(snapshot: &RestackSnapshot, remove_branches: &[String]) -> Vec<String> {
    let mut removals = remove_branches.to_vec();
    for tainted in &snapshot.tainted_features {
        if !removals.contains(&tainted.name) {
            removals.push(tainted.name.clone());
        }
    }
    removals
}
