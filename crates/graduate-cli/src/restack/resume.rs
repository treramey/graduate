//! Resumed session preview, apply, and abort.
//!
//! Sealed-session validation and publication live in `sealed.rs`.

use std::path::Path;

use graduate::restack::{build_plan, RestackPlan};

use super::errors::{conflict_error, plan_error, session_error, stale_session_error};
use super::isolated::{IsolatedRepository, ReconstructionResult};
use super::machine_output::{write_abort_result, write_apply_result, write_plan};
use super::sealed::{publish_sealed, reopen_sealed};
use super::source::{source_object_directory, source_repository_identity};
use crate::cli::RestackArgs;
use crate::restack::session::{SessionHandle, SessionStatus, SessionStore};
use crate::shared::error::CliError;

/// Result of validating a staged resolution and continuing the reconstruction.
pub(super) enum ResumedPreview {
    /// A later feature conflicted; the session stays resumable under the same token.
    Conflict {
        session: Box<SessionHandle>,
        branch: String,
        unresolved_paths: Vec<String>,
    },
    /// Every feature merged; the session is sealed around this plan.
    Sealed {
        session: Box<SessionHandle>,
        plan: Box<RestackPlan>,
    },
}

pub(super) fn resume_preview(
    args: &RestackArgs,
    token: &str,
    source: &Path,
    sessions: &SessionStore,
) -> Result<(), CliError> {
    match continue_session(args, token, source, sessions)? {
        ResumedPreview::Conflict {
            session,
            branch,
            unresolved_paths,
        } => Err(conflict_error(
            &branch,
            unresolved_paths,
            token,
            &session.repository(),
            session.metadata.expires_at,
        )),
        ResumedPreview::Sealed { plan, .. } => write_plan(&plan),
    }
}

/// Complete the staged manual merge and reconstruct the remaining features.
pub(super) fn continue_session(
    args: &RestackArgs,
    token: &str,
    source: &Path,
    sessions: &SessionStore,
) -> Result<ResumedPreview, CliError> {
    let mut session = open_resumed_session(args, token, source, sessions)?;
    if session.metadata.status == SessionStatus::Sealed {
        return reopen_sealed(session, source);
    }
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
            Ok(ResumedPreview::Conflict {
                session: Box::new(session),
                branch: conflict.feature.name,
                unresolved_paths: conflict.unresolved_paths,
            })
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
            Ok(ResumedPreview::Sealed {
                session: Box::new(session),
                plan: Box::new(plan),
            })
        }
    }
}

pub(super) fn resume_apply(
    args: &RestackArgs,
    token: &str,
    source: &Path,
    sessions: &SessionStore,
) -> Result<(), CliError> {
    let session = open_resumed_session(args, token, source, sessions)?;
    let plan = publish_sealed(source, session)?;
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

pub(super) fn open_resumed_session(
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
