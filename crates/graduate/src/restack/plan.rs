//! Removal selection, plan construction, and digests.

use std::collections::BTreeSet;

use sha2::{Digest, Sha256};

use super::errors::PlanError;
use super::interaction::SelectionError;
use super::inventory::orphaned_commit_ids;
use super::{
    BranchIdentity, InventoryMode, OrphanedCommit, Reconstruction, RemoteEndpointIdentity,
    RestackAuthor, RestackPlan, RestackSelection, RestackSnapshot, RESTACK_SCHEMA_VERSION,
};

/// Validate requested removals without silently changing their meaning.
pub fn select_features(
    snapshot: &RestackSnapshot,
    remove_branches: &[String],
) -> Result<RestackSelection, SelectionError> {
    let mut requested = BTreeSet::new();
    for branch in remove_branches {
        if !requested.insert(branch.as_str()) {
            return Err(SelectionError::Duplicate {
                branch: branch.clone(),
            });
        }
        if snapshot
            .features
            .iter()
            .any(|feature| feature.name == *branch)
        {
            continue;
        }
        if snapshot
            .graduated_features
            .iter()
            .any(|feature| feature.name == *branch)
        {
            return Err(SelectionError::Graduated {
                branch: branch.clone(),
            });
        }
        if snapshot
            .indirect_features
            .iter()
            .any(|feature| feature.name == *branch)
        {
            return Err(SelectionError::IndirectOnly {
                branch: branch.clone(),
            });
        }
        return Err(SelectionError::Unknown {
            branch: branch.clone(),
        });
    }

    for branch in remove_branches {
        let dependents = snapshot
            .attributed_commits
            .iter()
            .filter(|commit| commit.branches.iter().any(|owner| owner == branch))
            .flat_map(|commit| commit.branches.iter())
            .filter(|owner| owner.as_str() != branch && !requested.contains(owner.as_str()))
            .cloned()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        if !dependents.is_empty() {
            return Err(SelectionError::RetainedDependency {
                branch: branch.clone(),
                dependents,
            });
        }
    }

    let (removed, retained) = snapshot
        .features
        .iter()
        .map(|feature| BranchIdentity {
            name: feature.name.clone(),
            tip: feature.tip.clone(),
        })
        .partition(|feature| requested.contains(feature.name.as_str()));
    Ok(RestackSelection { retained, removed })
}

/// Bind validated reconstruction output to the captured inputs.
pub fn build_plan(
    snapshot: RestackSnapshot,
    remote_endpoints: RemoteEndpointIdentity,
    author: RestackAuthor,
    selection: RestackSelection,
    reconstruction: Reconstruction,
    orphaned_commits: Vec<OrphanedCommit>,
) -> Result<RestackPlan, PlanError> {
    let Reconstruction {
        merges,
        final_tree,
        preview_commit,
    } = reconstruction;
    if merges.len() != selection.retained.len() {
        return Err(PlanError::MergeCount {
            expected: selection.retained.len(),
            actual: merges.len(),
        });
    }
    for (index, (outcome, expected)) in merges.iter().zip(&selection.retained).enumerate() {
        if outcome.branch != expected.name || outcome.tip != expected.tip {
            return Err(PlanError::MergeIdentity {
                index,
                expected: expected.name.clone(),
            });
        }
    }
    let expected_orphans = orphaned_commit_ids(&snapshot, &selection.retained);
    let mut listed_orphans = orphaned_commits
        .iter()
        .map(|commit| commit.commit.clone())
        .collect::<Vec<_>>();
    listed_orphans.sort();
    if listed_orphans != expected_orphans {
        let mismatch = expected_orphans
            .iter()
            .find(|id| !listed_orphans.contains(id))
            .or_else(|| {
                listed_orphans
                    .iter()
                    .find(|id| !expected_orphans.contains(id))
            })
            .cloned()
            .unwrap_or_default();
        return Err(PlanError::OrphanedCommits {
            expected: expected_orphans.len(),
            actual: listed_orphans.len(),
            mismatch,
        });
    }
    let digest = plan_digest(
        &snapshot,
        &remote_endpoints,
        &author,
        &selection,
        &final_tree,
        &expected_orphans,
    );
    Ok(RestackPlan {
        snapshot,
        remote_endpoints,
        author,
        selection,
        merges,
        final_tree,
        preview_commit,
        orphaned_commits,
        digest,
    })
}

/// The canonical message for an explicit reconstructed merge.
pub fn canonical_merge_message(feature: &str, environment: &str) -> String {
    format!("Merge branch '{feature}' into {environment}")
}

fn plan_digest(
    snapshot: &RestackSnapshot,
    remote_endpoints: &RemoteEndpointIdentity,
    author: &RestackAuthor,
    selection: &RestackSelection,
    final_tree: &str,
    orphaned_commits: &[String],
) -> String {
    let mut digest = Sha256::new();
    digest_field(&mut digest, "schema", &RESTACK_SCHEMA_VERSION.to_string());
    digest_field(&mut digest, "remote", &snapshot.remote);
    digest_field(
        &mut digest,
        "remote_fetch_sha256",
        &remote_endpoints.fetch_sha256,
    );
    digest_field(
        &mut digest,
        "remote_push_sha256",
        &remote_endpoints.push_sha256,
    );
    digest_field(&mut digest, "environment", &snapshot.environment);
    digest_field(&mut digest, "environment_ref", &snapshot.environment_ref);
    digest_field(&mut digest, "environment_tip", &snapshot.environment_tip);
    digest_field(&mut digest, "main", &snapshot.main);
    digest_field(&mut digest, "main_ref", &snapshot.main_ref);
    digest_field(&mut digest, "main_tip", &snapshot.main_tip);
    digest_field(&mut digest, "author_name", &author.name);
    digest_field(&mut digest, "author_email", &author.email);
    for feature in &snapshot.features {
        digest_field(&mut digest, "feature_name", &feature.name);
        digest_field(&mut digest, "feature_tip", &feature.tip);
    }
    for feature in &selection.removed {
        digest_field(&mut digest, "removed_name", &feature.name);
        digest_field(&mut digest, "removed_tip", &feature.tip);
    }
    digest_field(&mut digest, "final_tree", final_tree);
    digest_field(
        &mut digest,
        "inventory_mode",
        match snapshot.inventory_mode {
            InventoryMode::History => "history",
            InventoryMode::Reachability => "reachability",
        },
    );
    for commit in orphaned_commits {
        digest_field(&mut digest, "orphaned_commit", commit);
    }
    format!("{:x}", digest.finalize())
}

fn digest_field(digest: &mut Sha256, label: &str, value: &str) {
    digest.update((label.len() as u64).to_be_bytes());
    digest.update(label.as_bytes());
    digest.update((value.len() as u64).to_be_bytes());
    digest.update(value.as_bytes());
}
