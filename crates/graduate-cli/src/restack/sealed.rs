//! Sealed restack sessions: plan reconstruction, validation, and leased publication.

use std::path::Path;

use graduate::restack::{build_plan, Reconstruction, RestackPlan};
use serde_json::json;

use super::errors::{plan_error, session_error, session_state_error, stale_session_error};
use super::isolated::IsolatedRepository;
use super::machine_output::machine_failure;
use super::plan_validation::{remote_environment_ref, revalidate_plan};
use super::resume::ResumedPreview;
use super::source::source_object_directory;
use crate::restack::session::{SessionHandle, SessionMetadata, SessionStatus};
use crate::shared::error::CliError;
use crate::shared::git_process;

/// Return the plan of an already sealed session without changing it, so a
/// preview can be reviewed again before `--apply` or `--abort`.
pub(super) fn reopen_sealed(
    mut session: SessionHandle,
    source: &Path,
) -> Result<ResumedPreview, CliError> {
    let plan = sealed_session_plan(&session.metadata)?;
    let source_objects = source_object_directory(source)?;
    let isolated = IsolatedRepository::open(session.repository(), &source_objects)?;
    validate_sealed_repository(&isolated, &session.metadata, &plan)?;
    session.metadata.refresh().map_err(session_error)?;
    session.save().map_err(session_error)?;
    Ok(ResumedPreview::Sealed {
        session: Box::new(session),
        plan: Box::new(plan),
    })
}

/// Revalidate a sealed session and publish its plan under an exact lease.
///
/// Successful publication consumes the session. A rejected push restores it
/// for a validated retry; an unprovable push leaves it untouched.
pub(super) fn publish_sealed(
    source: &Path,
    mut session: SessionHandle,
) -> Result<RestackPlan, CliError> {
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
        return match refs
            .ok()
            .and_then(|resolved| resolved.get(&environment_ref).cloned())
            .as_deref()
        {
            Some(oid) if oid == plan.snapshot.environment_tip => {
                session.restore_sealed().map_err(session_error)?;
                Err(machine_failure(
                    "push_rejected",
                    "the remote rejected the exact leased environment update",
                    json!({"environment": plan.snapshot.environment}),
                ))
            }
            Some(oid) if oid == plan.preview_commit => {
                session.consume().map_err(session_error)?;
                Ok(plan)
            }
            _ => Err(machine_failure(
                "push_outcome_unknown",
                "could not prove whether the leased environment update completed",
                json!({"environment": plan.snapshot.environment}),
            )),
        };
    }
    session.consume().map_err(session_error)?;
    Ok(plan)
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
