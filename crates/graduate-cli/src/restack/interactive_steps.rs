//! Interactive discovery, preparation, and publication steps.

use std::collections::BTreeMap;
use std::path::Path;

use graduate::restack::{
    build_inventory_snapshot, build_plan, orphaned_commit_ids, BranchIdentity, InventoryError,
    OrphanedCommit, RestackPlan, RestackSelection, RestackSnapshot,
};
use serde_json::json;

use super::errors::{inspection_error, plan_error, session_error};
use super::interactive::{
    InteractiveConflict, InteractiveDiscovery, InteractivePreparation, InteractivePrepared,
};
use super::isolated::{IsolatedRepository, ReconstructionResult};
use super::machine_output::machine_failure;
use super::plan_validation::{remote_environment_ref, revalidate_plan};
use super::source::{configured_author, source_object_directory, source_repository_identity};
use super::INSPECTION_OBJECT_CACHE_BYTES;
use crate::cli::RestackArgs;
use crate::environment_git::{
    commit_rows, inspect_environment, restack_snapshot, tip_timestamps, RestackInspectionError,
};
use crate::error::CliError;
use crate::git_process;
use crate::restack_session::{SessionConflict, SessionMetadata, SessionStore};

pub(super) fn discover_interactive(
    args: &RestackArgs,
    source: &Path,
) -> Result<InteractiveDiscovery, CliError> {
    let remote_name = args.remote.as_deref().unwrap_or("origin");
    let remote = git_process::resolve_restack_remote(remote_name, source).map_err(|_| {
        machine_failure(
            "remote_unavailable",
            "could not resolve one safe endpoint for the selected remote",
            json!({"remote": remote_name}),
        )
    })?;
    git_process::fetch_restack_remote(&remote, remote_name, source, true).map_err(|_| {
        machine_failure(
            "fetch_failed",
            "could not fetch the selected remote",
            json!({"remote": remote_name}),
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
    let inspection = inspect_environment(
        &repository,
        remote_name,
        &args.environment,
        args.main.as_deref(),
    )
    .map_err(|_| {
        machine_failure(
            "inspection_failed",
            "could not inspect the fetched environment refs",
            json!({"stage": "refs"}),
        )
    })?;
    let (snapshot, commit_rows) = match restack_snapshot(&repository, &inspection) {
        Ok(snapshot) => (snapshot, BTreeMap::new()),
        Err(RestackInspectionError::Unsupported { error, graph }) => {
            inventory_fallback(&repository, error, &graph)?
        }
        Err(error) => return Err(inspection_error(error)),
    };
    Ok(InteractiveDiscovery {
        remote,
        repository_id: source_repository_identity(source)?,
        snapshot,
        commit_rows,
        author: configured_author(source)?,
        source_objects: source_object_directory(source)?,
    })
}

/// Build the reachability inventory and the rows for every commit it might drop.
pub(super) fn inventory_fallback(
    repository: &gix::Repository,
    error: InventoryError,
    graph: &graduate::restack::RestackGraph,
) -> Result<(RestackSnapshot, BTreeMap<String, OrphanedCommit>), CliError> {
    let timestamps = tip_timestamps(repository, graph).map_err(|error| {
        machine_failure(
            "inspection_failed",
            "could not read the remote feature tips",
            json!({"stage": "tips", "error": error.to_string()}),
        )
    })?;
    let snapshot = build_inventory_snapshot(graph, error.into(), &timestamps);
    let rows = commit_rows(
        repository,
        snapshot
            .attributed_commits
            .iter()
            .map(|commit| &commit.commit)
            .chain(&snapshot.unattributed_commits),
    )
    .map_err(|error| {
        machine_failure(
            "inspection_failed",
            "could not read the environment's unique commits",
            json!({"stage": "orphans", "error": error.to_string()}),
        )
    })?;
    Ok((snapshot, rows))
}

/// Rows for the commits the reviewed selection drops.
pub(super) fn orphan_rows(
    snapshot: &RestackSnapshot,
    commit_rows: &BTreeMap<String, OrphanedCommit>,
    retained: &[BranchIdentity],
) -> Result<Vec<OrphanedCommit>, CliError> {
    orphaned_commit_ids(snapshot, retained)
        .iter()
        .map(|id| {
            commit_rows.get(id).cloned().ok_or_else(|| {
                machine_failure(
                    "inspection_failed",
                    "an orphaned commit was not captured during inspection",
                    json!({"stage": "orphans", "commit": id}),
                )
            })
        })
        .collect()
}

pub(super) fn prepare_interactive(
    discovery: &InteractiveDiscovery,
    selection: RestackSelection,
    sessions: &SessionStore,
) -> Result<InteractivePreparation, CliError> {
    let orphaned_commits = orphan_rows(
        &discovery.snapshot,
        &discovery.commit_rows,
        &selection.retained,
    )?;
    let mut draft = sessions.begin().map_err(session_error)?;
    let isolated = IsolatedRepository::create(&draft.repository(), &discovery.source_objects)?;
    isolated.train_resolutions(&discovery.snapshot, &selection.retained, &discovery.author)?;
    let reconstruction = isolated.reconstruct(
        &discovery.snapshot.main_tip,
        &discovery.snapshot.environment,
        &selection.retained,
        &discovery.author,
        0,
        Vec::new(),
    )?;
    match reconstruction {
        ReconstructionResult::Complete(reconstruction) => {
            let plan = build_plan(
                discovery.snapshot.clone(),
                discovery.remote.identity(),
                discovery.author.clone(),
                selection,
                reconstruction,
                orphaned_commits,
            )
            .map_err(plan_error)?;
            Ok(InteractivePreparation::Complete(Box::new(
                InteractivePrepared {
                    isolated,
                    draft,
                    plan,
                },
            )))
        }
        ReconstructionResult::Conflict(conflict) => {
            let metadata = SessionMetadata::conflicted(
                discovery.repository_id.clone(),
                discovery.snapshot.clone(),
                discovery.remote.identity(),
                discovery.author.clone(),
                selection,
                orphaned_commits,
                SessionConflict {
                    merges: conflict.merges,
                    next_feature: conflict.feature_index,
                    expected_head: conflict.expected_head,
                    expected_head_reflog: conflict.expected_head_reflog,
                    expected_feature_tip: conflict.feature.tip.clone(),
                },
            )
            .map_err(session_error)?;
            let repository = draft.repository();
            let work_area = repository.to_str().map(str::to_owned).ok_or_else(|| {
                machine_failure(
                    "session_unavailable",
                    "the restack work area path is not valid UTF-8",
                    json!({}),
                )
            })?;
            let resume_token = draft.token();
            draft.save(&metadata).map_err(session_error)?;
            Ok(InteractivePreparation::Conflict(InteractiveConflict {
                environment: discovery.snapshot.environment.clone(),
                branch: conflict.feature.name,
                unresolved_paths: conflict.unresolved_paths,
                resume_token,
                work_area,
            }))
        }
    }
}

pub(super) fn publish_interactive(
    source: &Path,
    remote: &git_process::RestackRemote,
    isolated: &IsolatedRepository,
    plan: &RestackPlan,
) -> Result<(), CliError> {
    revalidate_plan(source, remote, plan)?;
    isolated.validate_publication_plan(plan)?;
    git_process::push_restack_commit(
        remote,
        &isolated.root,
        &isolated.hooks,
        &isolated.global_config,
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
    })
}
