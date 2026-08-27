//! Machine JSON plan, apply, and abort output.

use std::io::Write;

use graduate::restack::{
    canonical_merge_message, BranchIdentity, InventoryMode, MergeResolution, RestackPlan,
    RESTACK_SCHEMA_VERSION,
};
use serde_json::{json, Value};

use super::plan_validation::remote_environment_ref;
use crate::error::{CliError, MachineError};

pub(super) fn write_plan(plan: &RestackPlan) -> Result<(), CliError> {
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

pub(super) fn write_apply_result(plan: &RestackPlan) -> Result<(), CliError> {
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

pub(super) fn write_abort_result(environment: &str) -> Result<(), CliError> {
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

pub(super) fn plan_json(plan: &RestackPlan) -> Value {
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
        "inventory": {
            "mode": match plan.snapshot.inventory_mode {
                InventoryMode::History => "history",
                InventoryMode::Reachability => "reachability",
            },
            "reason": plan.snapshot.unsupported_history.as_ref().map(|reason| json!({
                "kind": reason.kind,
                "commit": reason.commit,
                "featureParent": reason.feature_parent,
                "branches": reason.branches,
                "parents": reason.parents,
            })),
        },
        "carriedBranches": plan.snapshot.carried_features.iter().map(|carried| json!({
            "name": carried.name,
            "tip": carried.tip,
            "carriers": carried.carriers,
        })).collect::<Vec<_>>(),
        "orphanedCommits": plan.orphaned_commits.iter().map(|commit| json!({
            "commit": commit.commit,
            "subject": commit.subject,
            "author": commit.author,
            "date": commit.date,
        })).collect::<Vec<_>>(),
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
            "reusedResolutions": plan.snapshot.inventory_mode == InventoryMode::History,
            "commitSigning": "unsigned",
        },
    })
}

pub(super) fn machine_usage(code: &'static str, message: &'static str, details: Value) -> CliError {
    MachineError::usage(code, message, details).into()
}

pub(super) fn machine_failure(
    code: &'static str,
    message: &'static str,
    details: Value,
) -> CliError {
    MachineError::failure(code, message, details).into()
}
