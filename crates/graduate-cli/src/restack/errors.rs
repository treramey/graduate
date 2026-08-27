//! Machine error construction.

use std::fs;
use std::path::Path;

use graduate::restack::{InventoryError, PlanError, SelectionError};
use serde_json::json;

use super::machine_output::{machine_failure, machine_usage};
use crate::restack::session::SessionError;
use crate::shared::environment_git::RestackInspectionError;
use crate::shared::error::CliError;

pub(super) fn inspection_error(error: RestackInspectionError) -> CliError {
    match error {
        RestackInspectionError::Git(_) => machine_failure(
            "inspection_failed",
            "could not inspect the fetched environment history",
            json!({"stage": "history"}),
        ),
        RestackInspectionError::Unsupported { error, .. } => inventory_error(error),
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

pub(super) fn selection_error(error: SelectionError) -> CliError {
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

pub(super) fn plan_error(error: PlanError) -> CliError {
    let details = match error {
        PlanError::MergeCount { expected, actual } => {
            json!({"stage": "mergeCount", "expected": expected, "actual": actual})
        }
        PlanError::MergeIdentity { index, expected } => {
            json!({"stage": "mergeIdentity", "index": index, "expected": expected})
        }
        PlanError::OrphanedCommits {
            expected,
            actual,
            mismatch,
        } => {
            json!({"stage": "orphanedCommits", "expected": expected, "actual": actual, "mismatch": mismatch})
        }
    };
    machine_failure(
        "validation_failed",
        "isolated reconstruction did not match the selected plan",
        details,
    )
}

pub(super) fn isolated_setup_error() -> CliError {
    machine_failure(
        "isolated_setup_failed",
        "could not create the isolated restack work area",
        json!({}),
    )
}

pub(super) fn reconstruction_error(stage: &'static str) -> CliError {
    machine_failure(
        "reconstruction_failed",
        "Git could not complete isolated reconstruction",
        json!({"stage": stage}),
    )
}

pub(super) fn validation_error(stage: &'static str) -> CliError {
    machine_failure(
        "validation_failed",
        "isolated reconstruction failed validation",
        json!({"stage": stage}),
    )
}

pub(super) fn conflict_error(
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

pub(super) fn session_error(error: SessionError) -> CliError {
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
        SessionError::SchemaMismatch { found, expected } => machine_failure(
            "session_schema_mismatch",
            "the restack session was saved by a different Graduate release; start a new restack",
            json!({"found": found, "expected": expected}),
        ),
        SessionError::Unavailable => machine_failure(
            "session_unavailable",
            "the restack session store is unavailable",
            json!({}),
        ),
    }
}

pub(super) fn stale_session_error(reason: &'static str) -> CliError {
    machine_failure(
        "stale_session",
        "the restack session does not match this invocation",
        json!({"reason": reason}),
    )
}

pub(super) fn session_state_error(reason: &'static str) -> CliError {
    machine_failure(
        "invalid_session_state",
        "the restack work area is not in the expected resumable state",
        json!({"reason": reason}),
    )
}

pub(super) fn require_plain_directory(path: &Path) -> Result<(), CliError> {
    let metadata = fs::symlink_metadata(path).map_err(|_| session_state_error("layout"))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(session_state_error("layout"));
    }
    Ok(())
}

pub(super) fn require_plain_file(path: &Path) -> Result<(), CliError> {
    let metadata = fs::symlink_metadata(path).map_err(|_| session_state_error("layout"))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(session_state_error("layout"));
    }
    Ok(())
}
