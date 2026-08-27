//! Deterministic restack inventory contracts and history classification.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

mod errors;
mod interaction;
mod interaction_update;
mod inventory;
mod plan;
mod snapshot;
#[cfg(test)]
mod tests;

pub use errors::{InventoryError, PlanError};
pub use interaction::{
    RestackInteraction, RestackInteractionAction, RestackInteractionEffect,
    RestackInteractionStage, SelectionError,
};
pub use inventory::{build_inventory_snapshot, orphaned_commit_ids};
pub use plan::{build_plan, canonical_merge_message, select_features};
pub use snapshot::build_snapshot;

pub const RESTACK_SCHEMA_VERSION: u8 = 2;

/// One commit needed to classify an environment history.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GraphCommit {
    pub id: String,
    pub tree: String,
    pub parents: Vec<String>,
    pub message: String,
}

/// One remote feature ref and the environment-only commits reachable from its captured tip.
///
/// `ancestors` holds the reachable commits that the environment contains but main does not;
/// commits already on main are never consulted and may be omitted.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FeatureRef {
    pub name: String,
    pub tip: String,
    pub ancestors: BTreeSet<String>,
}

/// Captured graph facts used to prove that an environment is restackable.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RestackGraph {
    pub remote: String,
    pub environment: String,
    pub environment_ref: String,
    pub environment_tip: String,
    pub main: String,
    pub main_ref: String,
    pub main_tip: String,
    pub environment_ancestors: BTreeSet<String>,
    pub main_ancestors: BTreeSet<String>,
    pub feature_refs: Vec<FeatureRef>,
    pub commits: BTreeMap<String, GraphCommit>,
}

/// A historical two-parent merge accepted for later rerere training.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HistoricalMerge {
    pub commit: String,
    pub first_parent: String,
    pub feature_parent: String,
    pub tree: String,
}

/// A surviving explicit feature, ordered by its first merge into the environment.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExplicitFeature {
    pub name: String,
    pub tip: String,
    pub historical_merges: Vec<HistoricalMerge>,
}

/// A captured remote branch name and tip.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BranchIdentity {
    pub name: String,
    pub tip: String,
}

/// An exact obsolete phase marker that a v1 restack deliberately drops.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DroppedMarker {
    pub commit: String,
    pub parent: String,
    pub tree: String,
}

/// One environment-unique non-merge commit and its explicit feature owners.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AttributedCommit {
    pub commit: String,
    pub branches: Vec<String>,
}

/// How the feature inventory of a snapshot was discovered.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum InventoryMode {
    /// Explicit first-parent merges proved every commit's owner.
    History,
    /// History was unreadable; membership comes from remote tip reachability.
    Reachability,
}

/// Why history mode was unavailable, kept verbatim from the failed proof.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UnsupportedHistory {
    pub kind: String,
    pub commit: Option<String>,
    pub feature_parent: Option<String>,
    pub branches: Vec<String>,
    pub parents: Option<usize>,
}

/// A branch whose tip another top-level feature already contains.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CarriedFeature {
    pub name: String,
    pub tip: String,
    pub carriers: Vec<String>,
}

/// A commit the rebuilt environment will not contain.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OrphanedCommit {
    pub commit: String,
    pub subject: String,
    pub author: String,
    pub date: String,
}

/// A complete, ordered proof that an environment can be reconstructed.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RestackSnapshot {
    pub remote: String,
    pub environment: String,
    pub environment_ref: String,
    pub environment_tip: String,
    pub main: String,
    pub main_ref: String,
    pub main_tip: String,
    pub features: Vec<ExplicitFeature>,
    pub graduated_features: Vec<BranchIdentity>,
    pub indirect_features: Vec<BranchIdentity>,
    pub dropped_markers: Vec<DroppedMarker>,
    pub attributed_commits: Vec<AttributedCommit>,
    pub inventory_mode: InventoryMode,
    pub unsupported_history: Option<UnsupportedHistory>,
    pub carried_features: Vec<CarriedFeature>,
    /// Environment-only work that no top-level feature reaches. Always empty
    /// in history mode, where the proof rejects such commits instead.
    pub unattributed_commits: Vec<String>,
}

/// The configured identity used for every reconstructed merge commit.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RestackAuthor {
    pub name: String,
    pub email: String,
}

/// Credential-redacted identities for the remote endpoints reviewed by a plan.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RemoteEndpointIdentity {
    pub fetch_sha256: String,
    pub push_sha256: String,
}

/// A validated partition of the explicit feature inventory.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RestackSelection {
    pub retained: Vec<BranchIdentity>,
    pub removed: Vec<BranchIdentity>,
}

/// How isolated reconstruction resolved one retained feature merge.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum MergeResolution {
    Clean,
    Reused,
    Manual,
}

/// One validated merge produced by isolated reconstruction.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MergeOutcome {
    pub branch: String,
    pub tip: String,
    pub commit: String,
    pub tree: String,
    pub resolution: MergeResolution,
}

/// The validated output of one isolated reconstruction.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reconstruction {
    pub merges: Vec<MergeOutcome>,
    pub final_tree: String,
    pub preview_commit: String,
}

/// An immutable clean-preview plan.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RestackPlan {
    pub snapshot: RestackSnapshot,
    pub remote_endpoints: RemoteEndpointIdentity,
    pub author: RestackAuthor,
    pub selection: RestackSelection,
    pub merges: Vec<MergeOutcome>,
    pub final_tree: String,
    pub preview_commit: String,
    pub orphaned_commits: Vec<OrphanedCommit>,
    pub digest: String,
}

impl RestackInteraction {
    #[must_use]
    pub const fn stage(&self) -> RestackInteractionStage {
        self.stage
    }

    #[must_use]
    pub const fn inventory_mode(&self) -> InventoryMode {
        self.snapshot.inventory_mode
    }

    #[must_use]
    pub const fn unsupported_history(&self) -> Option<&UnsupportedHistory> {
        self.snapshot.unsupported_history.as_ref()
    }

    #[must_use]
    pub const fn cursor(&self) -> usize {
        self.cursor
    }

    #[must_use]
    pub const fn review_scroll(&self) -> usize {
        self.review_scroll
    }

    #[must_use]
    pub const fn review_details(&self) -> bool {
        self.review_details
    }

    #[must_use]
    pub const fn snapshot(&self) -> &RestackSnapshot {
        &self.snapshot
    }
}
