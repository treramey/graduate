//! Isolated restack preview, interactive review, resume, and apply workflow.

use std::collections::BTreeMap;
use std::fs;
use std::io::{IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use graduate::restack::{
    build_plan, canonical_merge_message, select_features, BranchIdentity, InventoryError,
    MergeOutcome, MergeResolution, PlanError, Reconstruction, RemoteEndpointIdentity,
    RestackAuthor, RestackInteraction, RestackPlan, RestackSelection, RestackSnapshot,
    SelectionError, RESTACK_SCHEMA_VERSION,
};
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::cli::RestackArgs;
use crate::environment_git::{
    inspect_environment, restack_snapshot, validate_ref_component, RestackInspectionError,
};
use crate::error::{CliError, MachineError};
use crate::git_process;
use crate::restack_session::{
    SessionConflict, SessionDraft, SessionError, SessionHandle, SessionMetadata, SessionStatus,
    SessionStore,
};
use crate::restack_tui::{self, ConflictHandoff, ReviewDecision, SelectionDecision};
use crate::terminal::StderrTerminal;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct MachineParams {
    remove_branches: Vec<String>,
    #[serde(default)]
    plan_digest: Option<String>,
}

pub(crate) fn run(args: RestackArgs) -> Result<(), CliError> {
    if args.params.is_none() && args.resume.is_none() && !args.dry_run {
        return run_interactive(args);
    }
    validate_inputs(&args)?;
    let sessions = SessionStore::open().map_err(session_error)?;
    let source = std::env::current_dir().map_err(|_| {
        machine_failure(
            "repository_unavailable",
            "could not access the source repository",
            json!({"stage": "currentDirectory"}),
        )
    })?;
    if let Some(token) = args.resume.as_deref() {
        sessions.prepare_resume(token).map_err(session_error)?;
        if args.abort {
            return abort_session(&args, token, &source, &sessions);
        }
        if args.apply {
            return resume_apply(&args, token, &source, &sessions);
        }
        return resume_preview(&args, token, &source, &sessions);
    }
    sessions.purge_expired().map_err(session_error)?;
    preview(&args, &source, &sessions)
}

struct InteractiveDiscovery {
    remote: git_process::RestackRemote,
    repository_id: String,
    snapshot: RestackSnapshot,
    author: RestackAuthor,
    source_objects: Vec<u8>,
}

struct InteractivePrepared {
    isolated: IsolatedRepository,
    draft: SessionDraft,
    plan: RestackPlan,
}

struct InteractiveConflict {
    environment: String,
    branch: String,
    unresolved_paths: Vec<String>,
    resume_token: String,
    work_area: String,
}

enum InteractivePreparation {
    Complete(Box<InteractivePrepared>),
    Conflict(InteractiveConflict),
}

enum InteractiveOutcome {
    Cancelled(String),
    Published(Box<RestackPlan>),
    Conflict(InteractiveConflict),
}

fn run_interactive(args: RestackArgs) -> Result<(), CliError> {
    if args.apply || args.abort || args.dry_run {
        return Err(machine_usage(
            "invalid_usage",
            "interactive restack uses terminal confirmation instead of --apply or --abort",
            json!({}),
        ));
    }
    if !std::io::stdin().is_terminal() || !std::io::stderr().is_terminal() {
        return Err(machine_usage(
            "params_required",
            "a non-terminal restack requires --params or --resume",
            json!({"expected": {"removeBranches": []}}),
        ));
    }
    validate_inputs(&args)?;
    let sessions = SessionStore::open().map_err(session_error)?;
    sessions.purge_expired().map_err(session_error)?;
    let source = std::env::current_dir().map_err(|_| {
        machine_failure(
            "repository_unavailable",
            "could not access the source repository",
            json!({"stage": "currentDirectory"}),
        )
    })?;
    let mut terminal = StderrTerminal::new()?;
    let outcome = interactive_workflow(&args, &source, &sessions, &mut terminal);
    finish_interactive(outcome, || terminal.restore(), write_interactive_outcome)
}

/// Object cache shared by the environment, main, and feature history walks.
const INSPECTION_OBJECT_CACHE_BYTES: usize = 64 * 1024 * 1024;

fn finish_interactive(
    outcome: Result<InteractiveOutcome, CliError>,
    restore: impl FnOnce() -> std::io::Result<()>,
    write: impl FnOnce(InteractiveOutcome) -> Result<(), CliError>,
) -> Result<(), CliError> {
    restore()?;
    write(outcome.map_err(interactive_error)?)
}

fn write_interactive_outcome(outcome: InteractiveOutcome) -> Result<(), CliError> {
    match outcome {
        InteractiveOutcome::Cancelled(environment) => restack_tui::write_cancelled(&environment),
        InteractiveOutcome::Published(plan) => restack_tui::write_success(&plan),
        InteractiveOutcome::Conflict(conflict) => restack_tui::write_conflict(&ConflictHandoff {
            environment: &conflict.environment,
            branch: &conflict.branch,
            unresolved_paths: &conflict.unresolved_paths,
            resume_token: &conflict.resume_token,
            work_area: &conflict.work_area,
        }),
    }
}

fn interactive_workflow(
    args: &RestackArgs,
    source: &Path,
    sessions: &SessionStore,
    terminal: &mut StderrTerminal,
) -> Result<InteractiveOutcome, CliError> {
    restack_tui::draw_loading(terminal, "Fetching and inspecting the environment…")?;
    let discovery = discover_interactive(args, source)?;
    let mut interaction = RestackInteraction::new(discovery.snapshot.clone());
    loop {
        let selection = match restack_tui::choose_features(terminal, &mut interaction)? {
            SelectionDecision::Preview(selection) => selection,
            SelectionDecision::Cancel => {
                return Ok(InteractiveOutcome::Cancelled(args.environment.clone()));
            }
        };
        restack_tui::draw_loading(terminal, "Reconstructing the reviewed selection…")?;
        let prepared = match prepare_interactive(&discovery, selection, sessions)? {
            InteractivePreparation::Complete(prepared) => prepared,
            InteractivePreparation::Conflict(conflict) => {
                return Ok(InteractiveOutcome::Conflict(conflict));
            }
        };
        match restack_tui::review_plan(terminal, &mut interaction, &prepared.plan)? {
            ReviewDecision::Revise => {
                prepared.draft.discard().map_err(session_error)?;
            }
            ReviewDecision::Cancel => {
                prepared.draft.discard().map_err(session_error)?;
                return Ok(InteractiveOutcome::Cancelled(args.environment.clone()));
            }
            ReviewDecision::Publish => {
                restack_tui::draw_loading(terminal, "Revalidating and publishing under lease…")?;
                publish_interactive(
                    source,
                    &discovery.remote,
                    &prepared.isolated,
                    &prepared.plan,
                )?;
                prepared.draft.discard().map_err(session_error)?;
                return Ok(InteractiveOutcome::Published(Box::new(prepared.plan)));
            }
        }
    }
}

fn discover_interactive(
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
    Ok(InteractiveDiscovery {
        remote,
        repository_id: source_repository_identity(source)?,
        snapshot: restack_snapshot(&repository, &inspection).map_err(inspection_error)?,
        author: configured_author(source)?,
        source_objects: source_object_directory(source)?,
    })
}

fn prepare_interactive(
    discovery: &InteractiveDiscovery,
    selection: RestackSelection,
    sessions: &SessionStore,
) -> Result<InteractivePreparation, CliError> {
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
                Vec::new(),
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

fn publish_interactive(
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

fn interactive_error(error: CliError) -> CliError {
    match error {
        CliError::Machine(error) => CliError::Restack(error.detailed_message()),
        error => error,
    }
}

fn preview(args: &RestackArgs, source: &Path, sessions: &SessionStore) -> Result<(), CliError> {
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
    let selection = select_features(&snapshot, &params.remove_branches).map_err(selection_error)?;
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

fn resume_preview(
    args: &RestackArgs,
    token: &str,
    source: &Path,
    sessions: &SessionStore,
) -> Result<(), CliError> {
    let mut session = open_resumed_session(args, token, source, sessions)?;
    if session.metadata.status != SessionStatus::Conflicted {
        return Err(stale_session_error("sessionStatus"));
    }
    let feature = session
        .metadata
        .selection
        .retained
        .get(session.metadata.next_feature)
        .cloned()
        .ok_or_else(|| stale_session_error("featurePosition"))?;
    if session.metadata.expected_feature_tip.as_deref() != Some(feature.tip.as_str())
        || session.metadata.merges.len() != session.metadata.next_feature
    {
        return Err(stale_session_error("featurePosition"));
    }
    session.metadata.refresh().map_err(session_error)?;
    session.save().map_err(session_error)?;

    let source_objects = source_object_directory(source)?;
    let isolated = IsolatedRepository::open(session.repository(), &source_objects)?;
    let manual = isolated.complete_manual_merge(
        &session.metadata.expected_head,
        &session.metadata.expected_head_reflog,
        &feature,
        &session.metadata.snapshot.environment,
        &session.metadata.author,
    )?;
    session.metadata.merges.push(manual);
    let reconstruction = isolated.reconstruct(
        &session.metadata.snapshot.main_tip,
        &session.metadata.snapshot.environment,
        &session.metadata.selection.retained,
        &session.metadata.author,
        session.metadata.next_feature + 1,
        session.metadata.merges.clone(),
    )?;
    match reconstruction {
        ReconstructionResult::Conflict(conflict) => {
            session.metadata.merges = conflict.merges;
            session.metadata.next_feature = conflict.feature_index;
            session.metadata.expected_head = conflict.expected_head;
            session.metadata.expected_head_reflog = conflict.expected_head_reflog;
            session.metadata.expected_feature_tip = Some(conflict.feature.tip.clone());
            session.metadata.refresh().map_err(session_error)?;
            session.save().map_err(session_error)?;
            Err(conflict_error(
                &conflict.feature.name,
                conflict.unresolved_paths,
                token,
                &session.repository(),
                session.metadata.expires_at,
            ))
        }
        ReconstructionResult::Complete(reconstruction) => {
            let plan = build_plan(
                session.metadata.snapshot.clone(),
                session.metadata.remote_endpoints.clone(),
                session.metadata.author.clone(),
                session.metadata.selection.clone(),
                reconstruction,
                Vec::new(),
            )
            .map_err(plan_error)?;
            session.metadata.merges.clone_from(&plan.merges);
            session.metadata.next_feature = session.metadata.selection.retained.len();
            session
                .metadata
                .expected_head
                .clone_from(&plan.preview_commit);
            session.metadata.expected_head_reflog = isolated.head_reflog_digest()?;
            session.metadata.expected_feature_tip = None;
            session.metadata.status = SessionStatus::Sealed;
            session.metadata.final_tree = Some(plan.final_tree.clone());
            session.metadata.preview_commit = Some(plan.preview_commit.clone());
            session.metadata.plan_digest = Some(plan.digest.clone());
            session.metadata.refresh().map_err(session_error)?;
            session.save().map_err(session_error)?;
            write_plan(&plan)
        }
    }
}

fn resume_apply(
    args: &RestackArgs,
    token: &str,
    source: &Path,
    sessions: &SessionStore,
) -> Result<(), CliError> {
    let mut session = open_resumed_session(args, token, source, sessions)?;
    if session.metadata.status != SessionStatus::Sealed {
        return Err(stale_session_error("sessionStatus"));
    }
    session.metadata.refresh().map_err(session_error)?;
    session.save().map_err(session_error)?;

    let plan = sealed_session_plan(&session.metadata)?;
    let source_objects = source_object_directory(source)?;
    let isolated = IsolatedRepository::open(session.repository(), &source_objects)?;
    validate_sealed_repository(&isolated, &session.metadata, &plan)?;

    let remote_name = &session.metadata.snapshot.remote;
    let remote = git_process::resolve_restack_remote(remote_name, source).map_err(|_| {
        machine_failure(
            "remote_unavailable",
            "could not resolve one safe endpoint for the selected remote",
            json!({"remote": remote_name}),
        )
    })?;
    if remote.identity() != plan.remote_endpoints {
        return Err(machine_failure(
            "stale_plan",
            "the reviewed remote endpoint changed before publication",
            json!({"reason": "remoteEndpoint"}),
        ));
    }
    revalidate_plan(source, &remote, &plan)?;
    validate_sealed_repository(&isolated, &session.metadata, &plan)?;
    session.begin_publication().map_err(session_error)?;
    let publication = git_process::push_restack_commit(
        &remote,
        &isolated.root,
        &isolated.hooks,
        &isolated.global_config,
        &plan.preview_commit,
        &remote_environment_ref(&plan.snapshot.environment),
        &plan.snapshot.environment_tip,
    );
    if publication.is_err() {
        let environment_ref = remote_environment_ref(&plan.snapshot.environment);
        let refs = git_process::read_restack_remote_refs(
            &remote,
            source,
            std::slice::from_ref(&environment_ref),
            true,
        );
        match refs
            .ok()
            .and_then(|resolved| resolved.get(&environment_ref).cloned())
            .as_deref()
        {
            Some(oid) if oid == plan.snapshot.environment_tip => {
                session.restore_sealed().map_err(session_error)?;
                return Err(machine_failure(
                    "push_rejected",
                    "the remote rejected the exact leased environment update",
                    json!({"environment": plan.snapshot.environment}),
                ));
            }
            Some(oid) if oid == plan.preview_commit => {
                session.consume().map_err(session_error)?;
                return write_apply_result(&plan);
            }
            _ => {
                return Err(machine_failure(
                    "push_outcome_unknown",
                    "could not prove whether the leased environment update completed",
                    json!({"environment": plan.snapshot.environment}),
                ));
            }
        }
    }
    session.consume().map_err(session_error)?;
    write_apply_result(&plan)
}

fn abort_session(
    args: &RestackArgs,
    token: &str,
    source: &Path,
    sessions: &SessionStore,
) -> Result<(), CliError> {
    let session = open_resumed_session(args, token, source, sessions)?;
    if session.metadata.status == SessionStatus::Consumed {
        return Err(stale_session_error("sessionStatus"));
    }
    let environment = session.metadata.snapshot.environment.clone();
    session.consume().map_err(session_error)?;
    write_abort_result(&environment)
}

fn open_resumed_session(
    args: &RestackArgs,
    token: &str,
    source: &Path,
    sessions: &SessionStore,
) -> Result<SessionHandle, CliError> {
    let repository_id = source_repository_identity(source)?;
    let session = sessions.resume(token).map_err(session_error)?;
    if session.metadata.repository_id != repository_id {
        return Err(stale_session_error("repository"));
    }
    if session.metadata.snapshot.environment != args.environment {
        return Err(stale_session_error("environment"));
    }
    if args
        .remote
        .as_ref()
        .is_some_and(|remote| *remote != session.metadata.snapshot.remote)
    {
        return Err(stale_session_error("remote"));
    }
    if args
        .main
        .as_ref()
        .is_some_and(|main| *main != session.metadata.snapshot.main)
    {
        return Err(stale_session_error("main"));
    }
    Ok(session)
}

fn sealed_session_plan(metadata: &SessionMetadata) -> Result<RestackPlan, CliError> {
    let complete = metadata.next_feature == metadata.selection.retained.len()
        && metadata.merges.len() == metadata.selection.retained.len()
        && metadata.expected_feature_tip.is_none();
    let Some(final_tree) = metadata.final_tree.clone() else {
        return Err(session_state_error("sealedPlan"));
    };
    let Some(preview_commit) = metadata.preview_commit.clone() else {
        return Err(session_state_error("sealedPlan"));
    };
    let Some(saved_digest) = metadata.plan_digest.as_deref() else {
        return Err(session_state_error("sealedPlan"));
    };
    if !complete || metadata.expected_head != preview_commit {
        return Err(session_state_error("sealedPlan"));
    }
    let plan = build_plan(
        metadata.snapshot.clone(),
        metadata.remote_endpoints.clone(),
        metadata.author.clone(),
        metadata.selection.clone(),
        Reconstruction {
            merges: metadata.merges.clone(),
            final_tree,
            preview_commit,
        },
        Vec::new(),
    )
    .map_err(plan_error)?;
    if plan.digest != saved_digest {
        return Err(session_state_error("sealedPlan"));
    }
    Ok(plan)
}

fn validate_sealed_repository(
    isolated: &IsolatedRepository,
    metadata: &SessionMetadata,
    plan: &RestackPlan,
) -> Result<(), CliError> {
    if isolated.head_reflog_digest()? != metadata.expected_head_reflog {
        return Err(session_state_error("sealedResult"));
    }
    isolated.validate_publication_plan(plan)
}

fn validate_inputs(args: &RestackArgs) -> Result<(), CliError> {
    if args.abort && args.resume.is_none() {
        return Err(machine_usage(
            "invalid_usage",
            "--abort requires --resume",
            json!({}),
        ));
    }
    for (label, value) in [
        ("environment", args.environment.as_str()),
        ("remote", args.remote.as_deref().unwrap_or("origin")),
    ] {
        validate_ref_component(label, value).map_err(|_| {
            machine_usage(
                "invalid_ref",
                "a restack ref name is not valid",
                json!({"field": label}),
            )
        })?;
    }
    if let Some(main) = &args.main {
        validate_ref_component("main", main).map_err(|_| {
            machine_usage(
                "invalid_ref",
                "a restack ref name is not valid",
                json!({"field": "main"}),
            )
        })?;
    }
    Ok(())
}

fn parse_params(params: Option<&str>, dry_run: bool) -> Result<MachineParams, CliError> {
    let Some(params) = params else {
        if dry_run {
            return Ok(MachineParams {
                remove_branches: Vec::new(),
                plan_digest: None,
            });
        }
        return Err(machine_usage(
            "params_required",
            "a machine restack preview requires --params",
            json!({"expected": {"removeBranches": []}}),
        ));
    };
    let parsed: MachineParams = serde_json::from_str(params).map_err(|_| {
        machine_usage(
            "invalid_params",
            "--params must match the schema-v1 restack machine parameters",
            json!({"expected": {"removeBranches": ["feature/BRANCH"], "planDigest": "apply only"}}),
        )
    })?;
    for (index, branch) in parsed.remove_branches.iter().enumerate() {
        validate_ref_component("removeBranches entry", branch).map_err(|_| {
            machine_usage(
                "invalid_params",
                "removeBranches contains an invalid Git branch name",
                json!({"index": index}),
            )
        })?;
    }
    Ok(parsed)
}

fn validate_apply_params(apply: bool, params: &MachineParams) -> Result<(), CliError> {
    match (apply, params.plan_digest.as_deref()) {
        (false, None) => Ok(()),
        (false, Some(_)) => Err(machine_usage(
            "invalid_params",
            "planDigest is accepted only with --apply",
            json!({"field": "planDigest"}),
        )),
        (true, Some(digest)) if valid_plan_digest(digest) => Ok(()),
        (true, _) => Err(machine_usage(
            "plan_digest_required",
            "--apply requires the lowercase SHA-256 planDigest from a preview",
            json!({"field": "planDigest"}),
        )),
    }
}

fn valid_plan_digest(digest: &str) -> bool {
    digest.len() == 64
        && digest
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

fn configured_author(source: &Path) -> Result<RestackAuthor, CliError> {
    let name = source_config(source, "user.name")?;
    let email = source_config(source, "user.email")?;
    if !valid_identity_value(&name) || !valid_identity_value(&email) {
        return Err(machine_failure(
            "missing_identity",
            "Git user.name and user.email must be configured",
            json!({}),
        ));
    }
    Ok(RestackAuthor { name, email })
}

fn source_config(source: &Path, key: &str) -> Result<String, CliError> {
    let output = source_git(source)
        .args(["config", "--get", key])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .map_err(|_| {
            machine_failure(
                "git_unavailable",
                "could not run Git for restack preflight",
                json!({"stage": "identity"}),
            )
        })?;
    if !output.status.success() {
        return Err(machine_failure(
            "missing_identity",
            "Git user.name and user.email must be configured",
            json!({}),
        ));
    }
    let value = String::from_utf8(output.stdout).map_err(|_| {
        machine_failure(
            "invalid_identity",
            "the configured Git identity is not valid UTF-8",
            json!({}),
        )
    })?;
    Ok(value.trim_end_matches(['\r', '\n']).to_owned())
}

fn valid_identity_value(value: &str) -> bool {
    !value.trim().is_empty() && !value.chars().any(char::is_control)
}

fn source_object_directory(source: &Path) -> Result<Vec<u8>, CliError> {
    let output = source_git(source)
        .args([
            "rev-parse",
            "--path-format=absolute",
            "--git-path",
            "objects",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .map_err(|_| {
            machine_failure(
                "git_unavailable",
                "could not run Git for restack preflight",
                json!({"stage": "objectStore"}),
            )
        })?;
    if !output.status.success() {
        return Err(machine_failure(
            "object_store_unavailable",
            "could not locate the source repository object store",
            json!({}),
        ));
    }
    let mut path = output.stdout;
    while matches!(path.last(), Some(b'\r' | b'\n')) {
        path.pop();
    }
    if path.is_empty() || path.contains(&b'\n') || path.contains(&b'\r') {
        return Err(machine_failure(
            "object_store_unavailable",
            "could not locate the source repository object store",
            json!({}),
        ));
    }
    Ok(path)
}

fn source_repository_identity(source: &Path) -> Result<String, CliError> {
    let output = source_git(source)
        .args(["rev-parse", "--path-format=absolute", "--git-common-dir"])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .map_err(|_| {
            machine_failure(
                "repository_unavailable",
                "could not identify the source repository",
                json!({}),
            )
        })?;
    if !output.status.success() {
        return Err(machine_failure(
            "repository_not_found",
            "the current directory is not inside a Git repository",
            json!({}),
        ));
    }
    let path = String::from_utf8(output.stdout).map_err(|_| {
        machine_failure(
            "repository_unavailable",
            "the source repository path is not valid UTF-8",
            json!({}),
        )
    })?;
    let canonical = fs::canonicalize(path.trim_end_matches(['\r', '\n'])).map_err(|_| {
        machine_failure(
            "repository_unavailable",
            "could not identify the source repository",
            json!({}),
        )
    })?;
    canonical.to_str().map(str::to_owned).ok_or_else(|| {
        machine_failure(
            "repository_unavailable",
            "the source repository path is not valid UTF-8",
            json!({}),
        )
    })
}

fn source_git(source: &Path) -> Command {
    let mut command = Command::new("git");
    clear_repository_location_environment(&mut command);
    command.current_dir(source);
    command
}

struct ReconstructionConflict {
    merges: Vec<MergeOutcome>,
    feature_index: usize,
    expected_head: String,
    expected_head_reflog: String,
    feature: BranchIdentity,
    unresolved_paths: Vec<String>,
}

enum ReconstructionResult {
    Complete(Reconstruction),
    Conflict(ReconstructionConflict),
}

struct FreshRestack<'a> {
    isolated: &'a IsolatedRepository,
    repository_id: String,
    snapshot: RestackSnapshot,
    remote_endpoints: RemoteEndpointIdentity,
    author: RestackAuthor,
    selection: graduate::restack::RestackSelection,
    apply_digest: Option<&'a str>,
    source: &'a Path,
    remote: &'a git_process::RestackRemote,
}

struct IsolatedRepository {
    root: PathBuf,
    hooks: PathBuf,
    global_config: PathBuf,
}

impl IsolatedRepository {
    fn create(root: &Path, source_objects: &[u8]) -> Result<Self, CliError> {
        let root = root.to_path_buf();
        let session = root.parent().ok_or_else(isolated_setup_error)?;
        let hooks = session.join("hooks");
        let global_config = session.join("global.gitconfig");
        fs::create_dir(&root).map_err(|_| isolated_setup_error())?;
        fs::create_dir(&hooks).map_err(|_| isolated_setup_error())?;
        fs::write(&global_config, []).map_err(|_| isolated_setup_error())?;

        let isolated = Self {
            root,
            hooks,
            global_config,
        };
        isolated.run_success(["init", "--quiet"], "initialize")?;
        fs::write(isolated.root.join(".git/config"), []).map_err(|_| isolated_setup_error())?;
        let alternates = isolated.root.join(".git/objects/info/alternates");
        let mut contents = source_objects.to_vec();
        contents.push(b'\n');
        fs::write(alternates, contents).map_err(|_| isolated_setup_error())?;
        Ok(isolated)
    }

    fn open(root: PathBuf, source_objects: &[u8]) -> Result<Self, CliError> {
        let session = root.parent().ok_or_else(isolated_setup_error)?;
        let isolated = Self {
            hooks: session.join("hooks"),
            global_config: session.join("global.gitconfig"),
            root,
        };
        isolated.validate_control_files(source_objects)?;
        Ok(isolated)
    }

    fn reconstruct(
        &self,
        main_tip: &str,
        environment: &str,
        retained: &[BranchIdentity],
        author: &RestackAuthor,
        start_index: usize,
        mut merges: Vec<MergeOutcome>,
    ) -> Result<ReconstructionResult, CliError> {
        let mut previous = if start_index == 0 {
            self.run_success(
                ["checkout", "--detach", "--quiet", main_tip, "--"],
                "checkoutBase",
            )?;
            main_tip.to_owned()
        } else {
            self.read_text(["rev-parse", "HEAD"], "continuationHead")?
        };
        for (feature_index, feature) in retained.iter().enumerate().skip(start_index) {
            let mut merge_command = self.command();
            merge_command
                .args([
                    "merge",
                    "--no-ff",
                    "--no-commit",
                    "--no-edit",
                    "--no-gpg-sign",
                    feature.tip.as_str(),
                ])
                .env("GIT_AUTHOR_NAME", &author.name)
                .env("GIT_AUTHOR_EMAIL", &author.email)
                .env("GIT_COMMITTER_NAME", &author.name)
                .env("GIT_COMMITTER_EMAIL", &author.email)
                .stdout(Stdio::null())
                .stderr(Stdio::null());
            let merge = merge_command
                .output()
                .map_err(|_| reconstruction_error("merge"))?;
            let resolution = if merge.status.success() {
                MergeResolution::Clean
            } else {
                let conflicted = self.unresolved_paths()?;
                if conflicted.is_empty() {
                    return Err(reconstruction_error("merge"));
                }
                self.run_success(["rerere"], "rerereReplay")?;
                let remaining = self.rerere_remaining()?;
                let resolved = conflicted
                    .iter()
                    .filter(|path| !remaining.contains(path))
                    .cloned()
                    .collect::<Vec<_>>();
                self.stage_paths(&resolved)?;
                if !remaining.is_empty() {
                    return Ok(ReconstructionResult::Conflict(ReconstructionConflict {
                        merges,
                        feature_index,
                        expected_head: previous,
                        expected_head_reflog: self.head_reflog_digest()?,
                        feature: feature.clone(),
                        unresolved_paths: remaining,
                    }));
                }
                MergeResolution::Reused
            };
            self.validate_index()?;
            let tree = self.read_text(["write-tree"], "writeTree")?;
            let message = canonical_merge_message(&feature.name, environment);
            let commit = self.commit_tree(&tree, &previous, &feature.tip, &message, author)?;
            self.run_success(["reset", "--hard", "--quiet", &commit], "resetResult")?;
            self.validate_commit(&commit, &previous, &feature.tip, &message, author)?;
            self.validate_clean_state(&previous, &commit)?;
            previous.clone_from(&commit);
            merges.push(MergeOutcome {
                branch: feature.name.clone(),
                tip: feature.tip.clone(),
                commit,
                tree,
                resolution,
            });
        }
        self.validate_clean_state(main_tip, &previous)?;
        let final_tree = self.read_text(["rev-parse", "HEAD^{tree}"], "finalTree")?;
        if let Some(last) = merges.last() {
            if last.tree != final_tree {
                return Err(validation_error("finalTree"));
            }
        } else {
            let base_tree =
                self.read_text(["rev-parse", &format!("{main_tip}^{{tree}}")], "baseTree")?;
            if base_tree != final_tree || previous != main_tip {
                return Err(validation_error("finalTree"));
            }
        }
        Ok(ReconstructionResult::Complete(Reconstruction {
            merges,
            final_tree,
            preview_commit: previous,
        }))
    }

    fn train_resolutions(
        &self,
        snapshot: &RestackSnapshot,
        retained: &[BranchIdentity],
        author: &RestackAuthor,
    ) -> Result<(), CliError> {
        for feature in &snapshot.features {
            if !retained
                .iter()
                .any(|retained| retained.name == feature.name)
            {
                continue;
            }
            for historical in &feature.historical_merges {
                self.run_success(
                    [
                        "checkout",
                        "--detach",
                        "--quiet",
                        &historical.first_parent,
                        "--",
                    ],
                    "trainingCheckout",
                )?;
                let merge = self
                    .command()
                    .args([
                        "merge",
                        "--no-ff",
                        "--no-commit",
                        "--no-edit",
                        "--no-gpg-sign",
                        &historical.feature_parent,
                    ])
                    .env("GIT_AUTHOR_NAME", &author.name)
                    .env("GIT_AUTHOR_EMAIL", &author.email)
                    .env("GIT_COMMITTER_NAME", &author.name)
                    .env("GIT_COMMITTER_EMAIL", &author.email)
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .output()
                    .map_err(|_| reconstruction_error("trainingMerge"))?;
                if merge.status.success() {
                    self.run_success(
                        ["reset", "--hard", "--quiet", &historical.first_parent],
                        "trainingReset",
                    )?;
                    continue;
                }
                if self.unresolved_paths()?.is_empty() {
                    return Err(reconstruction_error("trainingMerge"));
                }
                self.run_success(["rerere"], "trainingPreimage")?;
                self.run_success(
                    ["checkout", "--quiet", &historical.commit, "--", "."],
                    "trainingResolution",
                )?;
                let accepted_tree = self.read_text(["write-tree"], "trainingTree")?;
                if accepted_tree != historical.tree {
                    return Err(validation_error("trainingTree"));
                }
                self.run_success(["rerere"], "trainingPostimage")?;
                self.run_success(
                    ["reset", "--hard", "--quiet", &historical.first_parent],
                    "trainingReset",
                )?;
            }
        }
        Ok(())
    }

    fn complete_manual_merge(
        &self,
        expected_head: &str,
        expected_head_reflog: &str,
        feature: &BranchIdentity,
        environment: &str,
        author: &RestackAuthor,
    ) -> Result<MergeOutcome, CliError> {
        let head = self.read_text(["rev-parse", "HEAD"], "resumeHead")?;
        if head != expected_head {
            return Err(session_state_error("agentCommit"));
        }
        if self.head_reflog_digest()? != expected_head_reflog {
            return Err(session_state_error("agentCommit"));
        }
        let merge_head = self.read_text(["rev-parse", "MERGE_HEAD"], "resumeMergeHead")?;
        if merge_head != feature.tip {
            return Err(session_state_error("mergeParent"));
        }
        if !self.unresolved_paths()?.is_empty() {
            return Err(session_state_error("resolutionNotStaged"));
        }
        let unstaged = self.read_bytes(["diff", "--name-only", "-z"], "unstagedState")?;
        let untracked = self.read_bytes(
            ["ls-files", "--others", "--exclude-standard", "-z"],
            "untrackedState",
        )?;
        if !unstaged.is_empty() || !untracked.is_empty() {
            return Err(session_state_error("resolutionNotStaged"));
        }
        self.run_success(["rerere"], "recordResolution")?;
        if !self.rerere_remaining()?.is_empty() {
            return Err(session_state_error("resolutionNotStaged"));
        }
        self.validate_index()?;
        let tree = self.read_text(["write-tree"], "writeTree")?;
        let message = canonical_merge_message(&feature.name, environment);
        let commit = self.commit_tree(&tree, expected_head, &feature.tip, &message, author)?;
        self.run_success(["reset", "--hard", "--quiet", &commit], "resetResult")?;
        self.validate_commit(&commit, expected_head, &feature.tip, &message, author)?;
        self.validate_clean_state(expected_head, &commit)?;
        Ok(MergeOutcome {
            branch: feature.name.clone(),
            tip: feature.tip.clone(),
            commit,
            tree,
            resolution: MergeResolution::Manual,
        })
    }

    fn rerere_remaining(&self) -> Result<Vec<String>, CliError> {
        let output = self.read_text(["rerere", "remaining"], "rerereRemaining")?;
        Ok(output
            .lines()
            .filter(|path| !path.is_empty())
            .map(str::to_owned)
            .collect())
    }

    fn head_reflog_digest(&self) -> Result<String, CliError> {
        let reflog = self.read_bytes(
            ["reflog", "show", "HEAD", "--format=%H%x00%gD"],
            "headReflog",
        )?;
        Ok(format!("{:x}", Sha256::digest(reflog)))
    }

    fn stage_paths(&self, paths: &[String]) -> Result<(), CliError> {
        if paths.is_empty() {
            return Ok(());
        }
        let output = self
            .command()
            .arg("add")
            .arg("--")
            .args(paths)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .output()
            .map_err(|_| reconstruction_error("stageRerere"))?;
        if output.status.success() {
            Ok(())
        } else {
            Err(reconstruction_error("stageRerere"))
        }
    }

    fn commit_tree(
        &self,
        tree: &str,
        first_parent: &str,
        feature_parent: &str,
        message: &str,
        author: &RestackAuthor,
    ) -> Result<String, CliError> {
        let mut command = self.command();
        command
            .args([
                "commit-tree",
                tree,
                "-p",
                first_parent,
                "-p",
                feature_parent,
                "-m",
                message,
            ])
            .env("GIT_AUTHOR_NAME", &author.name)
            .env("GIT_AUTHOR_EMAIL", &author.email)
            .env("GIT_COMMITTER_NAME", &author.name)
            .env("GIT_COMMITTER_EMAIL", &author.email)
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        read_success_text(command.output(), "commitTree")
    }

    fn validate_index(&self) -> Result<(), CliError> {
        let unmerged = self.read_bytes(["ls-files", "-u", "-z"], "unmergedIndex")?;
        if !unmerged.is_empty() {
            return Err(validation_error("unmergedIndex"));
        }
        self.run_success(["diff", "--cached", "--check"], "stagedDiffCheck")?;
        self.run_success(["diff", "--check"], "worktreeDiffCheck")
    }

    fn validate_commit(
        &self,
        commit: &str,
        first_parent: &str,
        feature_parent: &str,
        message: &str,
        author: &RestackAuthor,
    ) -> Result<(), CliError> {
        let parents = self.read_text(["rev-list", "--parents", "-n", "1", commit], "parents")?;
        let expected = format!("{commit} {first_parent} {feature_parent}");
        if parents != expected {
            return Err(validation_error("parents"));
        }
        let actual_message = self.read_text(["show", "-s", "--format=%B", commit], "message")?;
        if actual_message != message {
            return Err(validation_error("message"));
        }
        let identity = self.read_bytes(
            ["show", "-s", "--format=%an%x00%ae%x00%cn%x00%ce", commit],
            "identity",
        )?;
        let expected_identity = format!(
            "{}\0{}\0{}\0{}\n",
            author.name, author.email, author.name, author.email
        );
        if identity != expected_identity.as_bytes() {
            return Err(validation_error("identity"));
        }
        let raw = self.read_bytes(["cat-file", "commit", commit], "signature")?;
        let headers = raw
            .split(|byte| *byte == b'\n')
            .take_while(|line| !line.is_empty());
        if headers.into_iter().any(|line| line.starts_with(b"gpgsig ")) {
            return Err(validation_error("signature"));
        }
        Ok(())
    }

    fn validate_clean_state(&self, previous: &str, commit: &str) -> Result<(), CliError> {
        let status = self.read_bytes(["status", "--porcelain=v1", "-z"], "status")?;
        if !status.is_empty() {
            return Err(validation_error("indexState"));
        }
        self.run_success(
            ["diff-tree", "--check", previous, commit],
            "resultDiffCheck",
        )
    }

    fn validate_publication_plan(&self, plan: &RestackPlan) -> Result<(), CliError> {
        let head = self.read_text(["rev-parse", "HEAD"], "publicationHead")?;
        let tree = self.read_text(["rev-parse", "HEAD^{tree}"], "publicationTree")?;
        if head != plan.preview_commit || tree != plan.final_tree {
            return Err(validation_error("publicationResult"));
        }
        let mut first_parent = plan.snapshot.main_tip.as_str();
        for (merge, feature) in plan.merges.iter().zip(&plan.selection.retained) {
            let message = canonical_merge_message(&feature.name, &plan.snapshot.environment);
            self.validate_commit(
                &merge.commit,
                first_parent,
                &feature.tip,
                &message,
                &plan.author,
            )?;
            first_parent = &merge.commit;
        }
        self.validate_clean_state(&plan.snapshot.main_tip, &plan.preview_commit)
    }

    fn unresolved_paths(&self) -> Result<Vec<String>, CliError> {
        let bytes = self.read_bytes(
            ["diff", "--name-only", "--diff-filter=U", "-z"],
            "conflictPaths",
        )?;
        bytes
            .split(|byte| *byte == 0)
            .filter(|path| !path.is_empty())
            .map(|path| {
                String::from_utf8(path.to_vec()).map_err(|_| {
                    machine_failure(
                        "invalid_path_encoding",
                        "an unresolved path is not valid UTF-8",
                        json!({}),
                    )
                })
            })
            .collect()
    }

    fn validate_control_files(&self, source_objects: &[u8]) -> Result<(), CliError> {
        require_plain_directory(&self.root)?;
        require_plain_directory(&self.hooks)?;
        if fs::read_dir(&self.hooks)
            .map_err(|_| session_state_error("hooks"))?
            .next()
            .is_some()
        {
            return Err(session_state_error("hooks"));
        }
        for path in [&self.global_config, &self.root.join(".git/config")] {
            require_plain_file(path)?;
            if !fs::read(path)
                .map_err(|_| session_state_error("configuration"))?
                .is_empty()
            {
                return Err(session_state_error("configuration"));
            }
        }
        let alternates = self.root.join(".git/objects/info/alternates");
        require_plain_file(&alternates)?;
        let mut expected = source_objects.to_vec();
        expected.push(b'\n');
        if fs::read(alternates).map_err(|_| session_state_error("objectStore"))? != expected {
            return Err(session_state_error("objectStore"));
        }
        Ok(())
    }

    fn read_text<const N: usize>(
        &self,
        arguments: [&str; N],
        stage: &'static str,
    ) -> Result<String, CliError> {
        let bytes = self.read_bytes(arguments, stage)?;
        let text = String::from_utf8(bytes).map_err(|_| validation_error(stage))?;
        Ok(text.trim_end_matches(['\r', '\n']).to_owned())
    }

    fn read_bytes<const N: usize>(
        &self,
        arguments: [&str; N],
        stage: &'static str,
    ) -> Result<Vec<u8>, CliError> {
        let mut command = self.command();
        command
            .args(arguments)
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        let output = command.output().map_err(|_| reconstruction_error(stage))?;
        if output.status.success() {
            Ok(output.stdout)
        } else {
            Err(validation_error(stage))
        }
    }

    fn run_success<const N: usize>(
        &self,
        arguments: [&str; N],
        stage: &'static str,
    ) -> Result<(), CliError> {
        let output = self
            .command()
            .args(arguments)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .output()
            .map_err(|_| reconstruction_error(stage))?;
        if output.status.success() {
            Ok(())
        } else {
            Err(reconstruction_error(stage))
        }
    }

    fn command(&self) -> Command {
        let mut command = Command::new("git");
        clear_isolated_environment(&mut command);
        command
            .current_dir(&self.root)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", &self.global_config)
            .env("GIT_CONFIG_COUNT", "7")
            .env("GIT_CONFIG_KEY_0", "core.hooksPath")
            .env("GIT_CONFIG_VALUE_0", &self.hooks)
            .env("GIT_CONFIG_KEY_1", "core.fsmonitor")
            .env("GIT_CONFIG_VALUE_1", "false")
            .env("GIT_CONFIG_KEY_2", "commit.gpgSign")
            .env("GIT_CONFIG_VALUE_2", "false")
            .env("GIT_CONFIG_KEY_3", "tag.gpgSign")
            .env("GIT_CONFIG_VALUE_3", "false")
            .env("GIT_CONFIG_KEY_4", "rerere.enabled")
            .env("GIT_CONFIG_VALUE_4", "true")
            .env("GIT_CONFIG_KEY_5", "rerere.autoupdate")
            .env("GIT_CONFIG_VALUE_5", "false")
            .env("GIT_CONFIG_KEY_6", "core.autocrlf")
            .env("GIT_CONFIG_VALUE_6", "false")
            .env("GIT_MERGE_AUTOEDIT", "no")
            .env("GIT_TERMINAL_PROMPT", "0");
        command
    }
}

fn clear_repository_location_environment(command: &mut Command) {
    for variable in [
        "GIT_ALTERNATE_OBJECT_DIRECTORIES",
        "GIT_COMMON_DIR",
        "GIT_CONFIG_COUNT",
        "GIT_CONFIG_PARAMETERS",
        "GIT_DIR",
        "GIT_GRAFT_FILE",
        "GIT_INDEX_FILE",
        "GIT_OBJECT_DIRECTORY",
        "GIT_PREFIX",
        "GIT_QUARANTINE_PATH",
        "GIT_SHALLOW_FILE",
        "GIT_WORK_TREE",
    ] {
        command.env_remove(variable);
    }
}

fn clear_isolated_environment(command: &mut Command) {
    clear_repository_location_environment(command);
    for variable in [
        "GIT_AUTHOR_DATE",
        "GIT_AUTHOR_EMAIL",
        "GIT_AUTHOR_NAME",
        "GIT_COMMITTER_DATE",
        "GIT_COMMITTER_EMAIL",
        "GIT_COMMITTER_NAME",
        "GIT_CONFIG",
        "GIT_CONFIG_COUNT",
        "GIT_CONFIG_GLOBAL",
        "GIT_CONFIG_NOSYSTEM",
        "GIT_CONFIG_PARAMETERS",
        "GIT_CONFIG_SYSTEM",
        "GIT_EXEC_PATH",
    ] {
        command.env_remove(variable);
    }
}

fn read_success_text(
    result: std::io::Result<Output>,
    stage: &'static str,
) -> Result<String, CliError> {
    let output = result.map_err(|_| reconstruction_error(stage))?;
    if !output.status.success() {
        return Err(reconstruction_error(stage));
    }
    let text = String::from_utf8(output.stdout).map_err(|_| validation_error(stage))?;
    Ok(text.trim_end_matches(['\r', '\n']).to_owned())
}

fn inspection_error(error: RestackInspectionError) -> CliError {
    match error {
        RestackInspectionError::Git(_) => machine_failure(
            "inspection_failed",
            "could not inspect the fetched environment history",
            json!({"stage": "history"}),
        ),
        RestackInspectionError::Unsupported(error) => inventory_error(error),
    }
}

fn inventory_error(error: InventoryError) -> CliError {
    let details = match error {
        InventoryError::MissingCommit { commit } => {
            json!({"kind": "missingCommit", "commit": commit})
        }
        InventoryError::DirectCommit { commit } => {
            json!({"kind": "directCommit", "commit": commit})
        }
        InventoryError::FastForwardHistory { commit, branches } => {
            json!({"kind": "fastForwardHistory", "commit": commit, "branches": branches})
        }
        InventoryError::OctopusMerge {
            merge_commit,
            parents,
        } => {
            json!({"kind": "octopusMerge", "mergeCommit": merge_commit, "parents": parents})
        }
        InventoryError::DeletedFeatureRef {
            merge_commit,
            feature_parent,
        } => {
            json!({"kind": "deletedFeatureRef", "mergeCommit": merge_commit, "featureParent": feature_parent})
        }
        InventoryError::AmbiguousFeatureRefs {
            merge_commit,
            feature_parent,
            branches,
        } => {
            json!({"kind": "ambiguousFeatureRefs", "mergeCommit": merge_commit, "featureParent": feature_parent, "branches": branches})
        }
    };
    machine_failure(
        "unsupported_history",
        "the environment history cannot be reconstructed without guessing",
        details,
    )
}

fn selection_error(error: SelectionError) -> CliError {
    let (kind, branch, dependents) = match error {
        SelectionError::Duplicate { branch } => ("duplicate", branch, Vec::new()),
        SelectionError::Graduated { branch } => ("graduated", branch, Vec::new()),
        SelectionError::IndirectOnly { branch } => ("indirectOnly", branch, Vec::new()),
        SelectionError::Unknown { branch } => ("unknown", branch, Vec::new()),
        SelectionError::RetainedDependency { branch, dependents } => {
            ("retainedDependency", branch, dependents)
        }
    };
    machine_usage(
        "invalid_removal",
        "removeBranches contains a feature that cannot be removed",
        json!({"kind": kind, "branch": branch, "dependents": dependents}),
    )
}

fn plan_error(error: PlanError) -> CliError {
    let details = match error {
        PlanError::MergeCount { expected, actual } => {
            json!({"stage": "mergeCount", "expected": expected, "actual": actual})
        }
        PlanError::MergeIdentity { index, expected } => {
            json!({"stage": "mergeIdentity", "index": index, "expected": expected})
        }
        PlanError::OrphanedCommits { expected, actual } => {
            json!({"stage": "orphanedCommits", "expected": expected, "actual": actual})
        }
    };
    machine_failure(
        "validation_failed",
        "isolated reconstruction did not match the selected plan",
        details,
    )
}

fn authorize_plan(plan: &RestackPlan, requested_digest: Option<&str>) -> Result<(), CliError> {
    if requested_digest == Some(plan.digest.as_str()) {
        Ok(())
    } else {
        Err(machine_failure(
            "stale_plan",
            "the freshly reconstructed plan does not match the reviewed digest",
            json!({"reason": "planDigest"}),
        ))
    }
}

fn revalidate_plan(
    source: &Path,
    remote: &git_process::RestackRemote,
    plan: &RestackPlan,
) -> Result<(), CliError> {
    if configured_author(source)? != plan.author {
        return Err(machine_failure(
            "stale_plan",
            "the configured Git identity changed before publication",
            json!({"reason": "identity"}),
        ));
    }
    let expected = expected_remote_refs(plan);
    let refs = expected.keys().cloned().collect::<Vec<_>>();
    validate_remote_refs(
        git_process::read_restack_remote_refs(remote, source, &refs, false),
        &expected,
        "fetch",
    )?;
    if remote.has_distinct_push_endpoint() {
        validate_remote_refs(
            git_process::read_restack_remote_refs(remote, source, &refs, true),
            &expected,
            "push",
        )?;
    }
    Ok(())
}

fn expected_remote_refs(plan: &RestackPlan) -> BTreeMap<String, String> {
    let mut refs = BTreeMap::new();
    refs.insert(
        remote_environment_ref(&plan.snapshot.environment),
        plan.snapshot.environment_tip.clone(),
    );
    refs.insert(
        remote_environment_ref(&plan.snapshot.main),
        plan.snapshot.main_tip.clone(),
    );
    for feature in plan
        .selection
        .retained
        .iter()
        .chain(&plan.selection.removed)
    {
        refs.insert(remote_environment_ref(&feature.name), feature.tip.clone());
    }
    refs
}

fn validate_remote_refs(
    actual: Result<BTreeMap<String, String>, CliError>,
    expected: &BTreeMap<String, String>,
    endpoint: &'static str,
) -> Result<(), CliError> {
    let actual = actual.map_err(|_| {
        machine_failure(
            "remote_revalidation_failed",
            "could not re-read the remote refs before publication",
            json!({"endpoint": endpoint}),
        )
    })?;
    for (reference, expected_oid) in expected {
        match actual.get(reference) {
            Some(actual_oid) if actual_oid == expected_oid => {}
            Some(_) => {
                return Err(machine_failure(
                    "stale_plan",
                    "a reviewed remote input moved before publication",
                    json!({"reason": "movedRef", "ref": reference, "endpoint": endpoint}),
                ));
            }
            None => {
                return Err(machine_failure(
                    "stale_plan",
                    "a reviewed remote input was deleted before publication",
                    json!({"reason": "deletedRef", "ref": reference, "endpoint": endpoint}),
                ));
            }
        }
    }
    Ok(())
}

fn remote_environment_ref(branch: &str) -> String {
    format!("refs/heads/{branch}")
}

fn write_plan(plan: &RestackPlan) -> Result<(), CliError> {
    let value = plan_json(plan);
    let output = serde_json::to_string(&value).map_err(|_| {
        machine_failure(
            "serialization_failed",
            "could not serialize the restack plan",
            json!({}),
        )
    })?;
    writeln!(std::io::stdout().lock(), "{output}").map_err(|_| {
        machine_failure(
            "output_failed",
            "could not write the restack plan to stdout",
            json!({}),
        )
    })
}

fn write_apply_result(plan: &RestackPlan) -> Result<(), CliError> {
    let branches = |branches: &[BranchIdentity]| {
        branches
            .iter()
            .map(|branch| json!({"name": branch.name, "tip": branch.tip}))
            .collect::<Vec<_>>()
    };
    let mut clean = 0_u64;
    let mut rerere = 0_u64;
    let mut manual = 0_u64;
    for merge in &plan.merges {
        match merge.resolution {
            MergeResolution::Clean => clean += 1,
            MergeResolution::Reused => rerere += 1,
            MergeResolution::Manual => manual += 1,
        }
    }
    let value = json!({
        "kind": "restackResult",
        "schemaVersion": RESTACK_SCHEMA_VERSION,
        "remote": plan.snapshot.remote,
        "environment": {
            "name": plan.snapshot.environment,
            "ref": remote_environment_ref(&plan.snapshot.environment),
            "oldOid": plan.snapshot.environment_tip,
            "newOid": plan.preview_commit,
        },
        "tree": plan.final_tree,
        "planDigest": plan.digest,
        "mergedBranches": branches(&plan.selection.retained),
        "removedBranches": branches(&plan.selection.removed),
        "resolutionCounts": {
            "clean": clean,
            "rerere": rerere,
            "manual": manual,
        },
        "pushed": true,
        "effects": {
            "sourceCheckoutChanged": false,
            "localRefsChanged": false,
            "personalRerereChanged": false,
            "commitSigning": "unsigned",
        },
    });
    let output = serde_json::to_string(&value).map_err(|_| {
        machine_failure(
            "serialization_failed",
            "could not serialize the restack apply result",
            json!({}),
        )
    })?;
    writeln!(std::io::stdout().lock(), "{output}").map_err(|_| {
        machine_failure(
            "output_failed",
            "could not write the restack apply result to stdout",
            json!({}),
        )
    })
}

fn write_abort_result(environment: &str) -> Result<(), CliError> {
    let value = json!({
        "kind": "restackAbortResult",
        "schemaVersion": RESTACK_SCHEMA_VERSION,
        "environment": environment,
        "aborted": true,
        "effects": {
            "sourceCheckoutChanged": false,
            "localRefsChanged": false,
            "remoteRefsChanged": false,
            "personalRerereChanged": false,
        },
    });
    let output = serde_json::to_string(&value).map_err(|_| {
        machine_failure(
            "serialization_failed",
            "could not serialize the restack abort result",
            json!({}),
        )
    })?;
    writeln!(std::io::stdout().lock(), "{output}").map_err(|_| {
        machine_failure(
            "output_failed",
            "could not write the restack abort result to stdout",
            json!({}),
        )
    })
}

fn plan_json(plan: &RestackPlan) -> Value {
    let branches = |branches: &[graduate::restack::BranchIdentity]| {
        branches
            .iter()
            .map(|branch| json!({"name": branch.name, "tip": branch.tip}))
            .collect::<Vec<_>>()
    };
    let mut first_parent = plan.snapshot.main_tip.as_str();
    let merges = plan
        .merges
        .iter()
        .map(|merge| {
            let outcome = match merge.resolution {
                MergeResolution::Clean => "clean",
                MergeResolution::Reused => "rerere",
                MergeResolution::Manual => "manual",
            };
            let value = json!({
                "branch": merge.branch,
                "tip": merge.tip,
                "outcome": outcome,
                "commit": merge.commit,
                "tree": merge.tree,
                "firstParent": first_parent,
                "featureParent": merge.tip,
                "message": canonical_merge_message(&merge.branch, &plan.snapshot.environment),
            });
            first_parent = &merge.commit;
            value
        })
        .collect::<Vec<_>>();
    json!({
        "kind": "restackPlan",
        "schemaVersion": RESTACK_SCHEMA_VERSION,
        "remote": plan.snapshot.remote,
        "remoteEndpoints": {
            "fetchSha256": plan.remote_endpoints.fetch_sha256,
            "pushSha256": plan.remote_endpoints.push_sha256,
        },
        "environment": {
            "name": plan.snapshot.environment,
            "ref": plan.snapshot.environment_ref,
            "oid": plan.snapshot.environment_tip,
        },
        "base": {
            "name": plan.snapshot.main,
            "ref": plan.snapshot.main_ref,
            "oid": plan.snapshot.main_tip,
        },
        "author": {"name": plan.author.name, "email": plan.author.email},
        "retainedBranches": branches(&plan.selection.retained),
        "removedBranches": branches(&plan.selection.removed),
        "droppedMarkers": plan.snapshot.dropped_markers.iter().map(|marker| json!({
            "commit": marker.commit,
            "parent": marker.parent,
            "tree": marker.tree,
        })).collect::<Vec<_>>(),
        "merges": merges,
        "finalTree": plan.final_tree,
        "previewCommit": plan.preview_commit,
        "planDigest": plan.digest,
        "effects": {
            "fetchedRemoteTrackingRefs": true,
            "pushed": false,
            "sourceCheckoutChanged": false,
            "localRefsChanged": false,
            "personalRerereChanged": false,
            "commitSigning": "unsigned",
        },
    })
}

fn isolated_setup_error() -> CliError {
    machine_failure(
        "isolated_setup_failed",
        "could not create the isolated restack work area",
        json!({}),
    )
}

fn reconstruction_error(stage: &'static str) -> CliError {
    machine_failure(
        "reconstruction_failed",
        "Git could not complete isolated reconstruction",
        json!({"stage": stage}),
    )
}

fn validation_error(stage: &'static str) -> CliError {
    machine_failure(
        "validation_failed",
        "isolated reconstruction failed validation",
        json!({"stage": stage}),
    )
}

fn conflict_error(
    branch: &str,
    unresolved_paths: Vec<String>,
    token: &str,
    work_area: &Path,
    expires_at: u64,
) -> CliError {
    let Some(work_area) = work_area.to_str() else {
        return machine_failure(
            "session_unavailable",
            "the restack work area path is not valid UTF-8",
            json!({}),
        );
    };
    machine_failure(
        "reconstruction_conflict",
        "the restack preview has unresolved conflicts",
        json!({
            "branch": branch,
            "unresolvedPaths": unresolved_paths,
            "resumeToken": token,
            "workArea": work_area,
            "expiresAt": expires_at,
        }),
    )
}

fn session_error(error: SessionError) -> CliError {
    match error {
        SessionError::InvalidToken => machine_failure(
            "invalid_session",
            "the restack continuation token is not valid",
            json!({"reason": "token"}),
        ),
        SessionError::Missing => machine_failure(
            "invalid_session",
            "the restack session does not exist",
            json!({"reason": "missing"}),
        ),
        SessionError::Locked => machine_failure(
            "session_locked",
            "the restack session is already in use",
            json!({}),
        ),
        SessionError::Tampered => machine_failure(
            "invalid_session",
            "the restack session failed integrity validation",
            json!({"reason": "tampered"}),
        ),
        SessionError::Expired => machine_failure(
            "expired_session",
            "the restack session has expired",
            json!({}),
        ),
        SessionError::Unavailable => machine_failure(
            "session_unavailable",
            "the restack session store is unavailable",
            json!({}),
        ),
    }
}

fn stale_session_error(reason: &'static str) -> CliError {
    machine_failure(
        "stale_session",
        "the restack session does not match this invocation",
        json!({"reason": reason}),
    )
}

fn session_state_error(reason: &'static str) -> CliError {
    machine_failure(
        "invalid_session_state",
        "the restack work area is not in the expected resumable state",
        json!({"reason": reason}),
    )
}

fn require_plain_directory(path: &Path) -> Result<(), CliError> {
    let metadata = fs::symlink_metadata(path).map_err(|_| session_state_error("layout"))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(session_state_error("layout"));
    }
    Ok(())
}

fn require_plain_file(path: &Path) -> Result<(), CliError> {
    let metadata = fs::symlink_metadata(path).map_err(|_| session_state_error("layout"))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(session_state_error("layout"));
    }
    Ok(())
}

fn machine_usage(code: &'static str, message: &'static str, details: Value) -> CliError {
    MachineError::usage(code, message, details).into()
}

fn machine_failure(code: &'static str, message: &'static str, details: Value) -> CliError {
    MachineError::failure(code, message, details).into()
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    use super::*;

    #[test]
    fn interactive_completion_restores_before_ordinary_output(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let terminal = Rc::new(RefCell::new(Terminal::new(TestBackend::new(40, 10))?));
        terminal.borrow_mut().hide_cursor()?;
        let restored = Rc::new(RefCell::new(false));
        let restore_terminal = Rc::clone(&terminal);
        let restore_state = Rc::clone(&restored);
        let write_state = Rc::clone(&restored);

        finish_interactive(
            Ok(InteractiveOutcome::Cancelled("qa".to_owned())),
            move || {
                let _ = restore_terminal.borrow_mut().show_cursor();
                *restore_state.borrow_mut() = true;
                Ok(())
            },
            move |_| {
                assert!(*write_state.borrow());
                Ok(())
            },
        )?;
        Ok(())
    }

    #[test]
    fn interactive_failure_still_restores_and_skips_ordinary_output(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let terminal = Rc::new(RefCell::new(Terminal::new(TestBackend::new(40, 10))?));
        terminal.borrow_mut().hide_cursor()?;
        let restored = Rc::new(RefCell::new(false));
        let wrote = Rc::new(RefCell::new(false));
        let restore_terminal = Rc::clone(&terminal);
        let restore_state = Rc::clone(&restored);
        let write_state = Rc::clone(&wrote);

        let result = finish_interactive(
            Err(machine_failure("fetch_failed", "fetch failed", json!({}))),
            move || {
                let _ = restore_terminal.borrow_mut().show_cursor();
                *restore_state.borrow_mut() = true;
                Ok(())
            },
            move |_| {
                *write_state.borrow_mut() = true;
                Ok(())
            },
        );

        assert!(matches!(result, Err(CliError::Restack(message)) if message == "fetch failed"));
        assert!(*restored.borrow());
        assert!(!*wrote.borrow());
        Ok(())
    }

    #[test]
    fn interactive_error_keeps_structured_details() {
        let error = interactive_error(machine_failure(
            "unsupported_history",
            "the environment history cannot be reconstructed without guessing",
            json!({"kind": "ambiguousFeatureRefs", "mergeCommit": "886faef"}),
        ));
        assert_eq!(
            error.to_string(),
            "restack failed: the environment history cannot be reconstructed without guessing ({\"kind\":\"ambiguousFeatureRefs\",\"mergeCommit\":\"886faef\"})"
        );
    }
}
