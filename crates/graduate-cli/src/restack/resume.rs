//! Resumed session preview, apply, and abort.

use std::path::Path;

use graduate::restack::{build_plan, Reconstruction, RestackPlan};
use serde_json::json;

use super::errors::{
    conflict_error, plan_error, session_error, session_state_error, stale_session_error,
};
use super::isolated::{IsolatedRepository, ReconstructionResult};
use super::machine_output::{machine_failure, write_abort_result, write_apply_result, write_plan};
use super::plan_validation::{remote_environment_ref, revalidate_plan};
use super::source::{source_object_directory, source_repository_identity};
use crate::cli::RestackArgs;
use crate::error::CliError;
use crate::git_process;
use crate::restack_session::{SessionHandle, SessionMetadata, SessionStatus, SessionStore};

pub(super) fn resume_preview(
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
                session.metadata.orphaned_commits.clone(),
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

pub(super) fn resume_apply(
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

pub(super) fn abort_session(
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

pub(super) fn sealed_session_plan(metadata: &SessionMetadata) -> Result<RestackPlan, CliError> {
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
        metadata.orphaned_commits.clone(),
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
