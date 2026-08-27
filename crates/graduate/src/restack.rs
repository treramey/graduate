//! Deterministic restack inventory contracts and history classification.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

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

/// Evidence that a requested feature removal is not safe or meaningful.
#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum SelectionError {
    #[error("feature `{branch}` was requested for removal more than once")]
    Duplicate { branch: String },
    #[error("feature `{branch}` has already graduated to main")]
    Graduated { branch: String },
    #[error("feature `{branch}` is only indirectly reachable from the environment")]
    IndirectOnly { branch: String },
    #[error("feature `{branch}` is not in the explicit environment inventory")]
    Unknown { branch: String },
    #[error("feature `{branch}` cannot be removed because its commits remain reachable through {dependents:?}")]
    RetainedDependency {
        branch: String,
        dependents: Vec<String>,
    },
}

/// One screen in the deterministic interactive restack review flow.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RestackInteractionStage {
    UnsupportedHistory,
    Selection,
    Review,
    Confirmation,
}

/// A terminal-independent action in the interactive restack review flow.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RestackInteractionAction {
    MoveUp,
    MoveDown,
    MovePageUp,
    MovePageDown,
    MoveFirst,
    MoveLast,
    MoveTo(usize),
    Toggle,
    KeepAll,
    RemoveAll,
    ToggleDetails,
    AcceptInventoryFallback,
    Continue,
    Back,
    Confirm,
    Cancel,
}

/// An effect requested by an interactive restack state transition.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RestackInteractionEffect {
    None,
    Preview(RestackSelection),
    Revise,
    Publish,
    Cancel,
    Rejected(SelectionError),
}

/// Deterministic feature selection, review, and confirmation state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RestackInteraction {
    snapshot: RestackSnapshot,
    retained: Vec<bool>,
    cursor: usize,
    review_scroll: usize,
    review_details: bool,
    stage: RestackInteractionStage,
}

impl RestackInteraction {
    /// Start with every explicit feature retained in discovered merge order.
    #[must_use]
    pub fn new(snapshot: RestackSnapshot) -> Self {
        Self {
            retained: vec![true; snapshot.features.len()],
            snapshot,
            cursor: 0,
            review_scroll: 0,
            review_details: false,
            stage: RestackInteractionStage::Selection,
        }
    }

    /// Start on the unsupported-history screen with a reachability inventory.
    ///
    /// Every top-level feature is retained; the checklist is reached only by
    /// an explicit `AcceptInventoryFallback`.
    #[must_use]
    pub fn from_inventory(snapshot: RestackSnapshot) -> Self {
        Self {
            stage: RestackInteractionStage::UnsupportedHistory,
            ..Self::new(snapshot)
        }
    }

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
    pub fn carried_features(&self) -> &[CarriedFeature] {
        &self.snapshot.carried_features
    }

    /// Commits the rebuild would drop for the current retained set.
    #[must_use]
    pub fn orphaned_commit_count(&self) -> usize {
        orphaned_commit_ids(&self.snapshot, &self.retained_identities()).len()
    }

    fn retained_identities(&self) -> Vec<BranchIdentity> {
        self.snapshot
            .features
            .iter()
            .zip(&self.retained)
            .filter(|(_, retained)| **retained)
            .map(|(feature, _)| branch_identity_of(feature))
            .collect()
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
    pub fn is_retained(&self, index: usize) -> bool {
        self.retained.get(index).copied().unwrap_or(false)
    }

    /// Return retained features that prevent removing the feature at `index`.
    #[must_use]
    pub fn retained_dependents(&self, index: usize) -> Vec<String> {
        let Some(feature) = self.snapshot.features.get(index) else {
            return Vec::new();
        };
        self.snapshot
            .attributed_commits
            .iter()
            .filter(|commit| commit.branches.iter().any(|owner| owner == &feature.name))
            .flat_map(|commit| commit.branches.iter())
            .filter(|owner| owner.as_str() != feature.name)
            .filter(|owner| {
                self.snapshot
                    .features
                    .iter()
                    .position(|candidate| candidate.name == **owner)
                    .is_some_and(|dependent| self.is_retained(dependent))
            })
            .cloned()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect()
    }

    #[must_use]
    pub const fn snapshot(&self) -> &RestackSnapshot {
        &self.snapshot
    }

    /// Mark the selected reconstruction as ready for review.
    pub fn review_ready(&mut self) {
        self.stage = RestackInteractionStage::Review;
    }

    /// Apply one action and return any requested workflow effect.
    pub fn update(&mut self, action: RestackInteractionAction) -> RestackInteractionEffect {
        match action {
            RestackInteractionAction::Cancel => RestackInteractionEffect::Cancel,
            RestackInteractionAction::MoveUp
                if self.stage == RestackInteractionStage::Selection =>
            {
                self.cursor = self.cursor.saturating_sub(1);
                RestackInteractionEffect::None
            }
            RestackInteractionAction::MoveDown
                if self.stage == RestackInteractionStage::Selection =>
            {
                self.cursor = self
                    .cursor
                    .saturating_add(1)
                    .min(self.snapshot.features.len().saturating_sub(1));
                RestackInteractionEffect::None
            }
            RestackInteractionAction::MovePageUp
                if self.stage == RestackInteractionStage::Selection =>
            {
                self.cursor = self.cursor.saturating_sub(10);
                RestackInteractionEffect::None
            }
            RestackInteractionAction::MovePageDown
                if self.stage == RestackInteractionStage::Selection =>
            {
                self.cursor = self
                    .cursor
                    .saturating_add(10)
                    .min(self.snapshot.features.len().saturating_sub(1));
                RestackInteractionEffect::None
            }
            RestackInteractionAction::MoveFirst
                if self.stage == RestackInteractionStage::Selection =>
            {
                self.cursor = 0;
                RestackInteractionEffect::None
            }
            RestackInteractionAction::MoveLast
                if self.stage == RestackInteractionStage::Selection =>
            {
                self.cursor = self.snapshot.features.len().saturating_sub(1);
                RestackInteractionEffect::None
            }
            RestackInteractionAction::MoveTo(index)
                if self.stage == RestackInteractionStage::Selection =>
            {
                self.cursor = index.min(self.snapshot.features.len().saturating_sub(1));
                RestackInteractionEffect::None
            }
            RestackInteractionAction::MoveUp if self.stage == RestackInteractionStage::Review => {
                self.review_scroll = self.review_scroll.saturating_sub(1);
                RestackInteractionEffect::None
            }
            RestackInteractionAction::MoveDown if self.stage == RestackInteractionStage::Review => {
                self.review_scroll = self.review_scroll.saturating_add(1);
                RestackInteractionEffect::None
            }
            RestackInteractionAction::MovePageUp
                if self.stage == RestackInteractionStage::Review =>
            {
                self.review_scroll = self.review_scroll.saturating_sub(10);
                RestackInteractionEffect::None
            }
            RestackInteractionAction::MovePageDown
                if self.stage == RestackInteractionStage::Review =>
            {
                self.review_scroll = self.review_scroll.saturating_add(10);
                RestackInteractionEffect::None
            }
            RestackInteractionAction::MoveFirst
                if self.stage == RestackInteractionStage::Review =>
            {
                self.review_scroll = 0;
                RestackInteractionEffect::None
            }
            RestackInteractionAction::MoveLast if self.stage == RestackInteractionStage::Review => {
                self.review_scroll = usize::MAX;
                RestackInteractionEffect::None
            }
            RestackInteractionAction::Toggle
                if self.stage == RestackInteractionStage::Selection =>
            {
                self.toggle_current()
            }
            RestackInteractionAction::KeepAll
                if self.stage == RestackInteractionStage::Selection =>
            {
                self.retained.fill(true);
                RestackInteractionEffect::None
            }
            RestackInteractionAction::RemoveAll
                if self.stage == RestackInteractionStage::Selection =>
            {
                self.retained.fill(false);
                RestackInteractionEffect::None
            }
            RestackInteractionAction::ToggleDetails
                if self.stage == RestackInteractionStage::Review =>
            {
                self.review_details = !self.review_details;
                self.review_scroll = 0;
                RestackInteractionEffect::None
            }
            RestackInteractionAction::AcceptInventoryFallback
                if self.stage == RestackInteractionStage::UnsupportedHistory =>
            {
                self.stage = RestackInteractionStage::Selection;
                RestackInteractionEffect::None
            }
            RestackInteractionAction::Continue
                if self.stage == RestackInteractionStage::Selection =>
            {
                match self.selection() {
                    Ok(selection) => RestackInteractionEffect::Preview(selection),
                    Err(error) => RestackInteractionEffect::Rejected(error),
                }
            }
            RestackInteractionAction::Continue if self.stage == RestackInteractionStage::Review => {
                self.stage = RestackInteractionStage::Confirmation;
                RestackInteractionEffect::None
            }
            RestackInteractionAction::Back if self.stage == RestackInteractionStage::Review => {
                self.stage = RestackInteractionStage::Selection;
                RestackInteractionEffect::Revise
            }
            RestackInteractionAction::Back
                if self.stage == RestackInteractionStage::Confirmation =>
            {
                self.stage = RestackInteractionStage::Review;
                RestackInteractionEffect::None
            }
            RestackInteractionAction::Confirm
                if self.stage == RestackInteractionStage::Confirmation =>
            {
                RestackInteractionEffect::Publish
            }
            RestackInteractionAction::MoveUp
            | RestackInteractionAction::MoveDown
            | RestackInteractionAction::MovePageUp
            | RestackInteractionAction::MovePageDown
            | RestackInteractionAction::MoveFirst
            | RestackInteractionAction::MoveLast
            | RestackInteractionAction::MoveTo(_)
            | RestackInteractionAction::Toggle
            | RestackInteractionAction::KeepAll
            | RestackInteractionAction::RemoveAll
            | RestackInteractionAction::ToggleDetails
            | RestackInteractionAction::AcceptInventoryFallback
            | RestackInteractionAction::Continue
            | RestackInteractionAction::Back
            | RestackInteractionAction::Confirm => RestackInteractionEffect::None,
        }
    }

    fn toggle_current(&mut self) -> RestackInteractionEffect {
        let Some(retained) = self.retained.get_mut(self.cursor) else {
            return RestackInteractionEffect::None;
        };
        *retained = !*retained;
        match self.selection() {
            Ok(_) => RestackInteractionEffect::None,
            Err(error) => {
                if let Some(retained) = self.retained.get_mut(self.cursor) {
                    *retained = !*retained;
                }
                RestackInteractionEffect::Rejected(error)
            }
        }
    }

    fn selection(&self) -> Result<RestackSelection, SelectionError> {
        let removals = self
            .snapshot
            .features
            .iter()
            .zip(&self.retained)
            .filter(|(_, retained)| !**retained)
            .map(|(feature, _)| feature.name.clone())
            .collect::<Vec<_>>();
        select_features(&self.snapshot, &removals)
    }
}

/// Evidence that reconstruction output does not match the selected plan.
#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum PlanError {
    #[error("reconstruction produced {actual} merge outcomes for {expected} retained features")]
    MergeCount { expected: usize, actual: usize },
    #[error("reconstruction outcome {index} does not match retained feature `{expected}`")]
    MergeIdentity { index: usize, expected: String },
    #[error("plan lists {actual} orphaned commits but the retained selection orphans {expected}; first mismatch {mismatch}")]
    OrphanedCommits {
        expected: usize,
        actual: usize,
        mismatch: String,
    },
}

/// Evidence that an environment history cannot be reconstructed safely.
#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum InventoryError {
    #[error("commit {commit} is missing from the inspected graph")]
    MissingCommit { commit: String },
    #[error("environment commit {commit} is direct work with no explicit feature merge")]
    DirectCommit { commit: String },
    #[error("environment history was fast-forwarded through {commit} from {branches:?}")]
    FastForwardHistory {
        commit: String,
        branches: Vec<String>,
    },
    #[error("environment merge {merge_commit} has {parents} parents; only two-parent merges are supported")]
    OctopusMerge {
        merge_commit: String,
        parents: usize,
    },
    #[error("merge {merge_commit} has feature parent {feature_parent}, but no surviving remote feature ref identifies it")]
    DeletedFeatureRef {
        merge_commit: String,
        feature_parent: String,
    },
    #[error("merge {merge_commit} feature parent {feature_parent} maps to multiple remote refs: {branches:?}")]
    AmbiguousFeatureRefs {
        merge_commit: String,
        feature_parent: String,
        branches: Vec<String>,
    },
}

/// Prove reconstructability and produce the canonical feature inventory.
pub fn build_snapshot(graph: &RestackGraph) -> Result<RestackSnapshot, InventoryError> {
    let mut first_parent = Vec::new();
    let mut current = graph.environment_tip.clone();
    while !graph.main_ancestors.contains(&current) {
        let commit = graph
            .commits
            .get(&current)
            .ok_or_else(|| InventoryError::MissingCommit {
                commit: current.clone(),
            })?;
        first_parent.push(current.clone());
        let Some(parent) = commit.parents.first() else {
            return Err(InventoryError::DirectCommit {
                commit: commit.id.clone(),
            });
        };
        current.clone_from(parent);
    }
    first_parent.reverse();

    let surviving = graph
        .feature_refs
        .iter()
        .filter(|feature| !graph.main_ancestors.contains(&feature.tip))
        .collect::<Vec<_>>();
    let mut features: Vec<ExplicitFeature> = Vec::new();
    let mut dropped_markers = Vec::new();
    let mut first_parent_non_merges = BTreeSet::new();

    for id in first_parent {
        let commit = graph
            .commits
            .get(&id)
            .ok_or_else(|| InventoryError::MissingCommit { commit: id.clone() })?;
        match commit.parents.as_slice() {
            [parent] => {
                if is_dropped_marker(graph, commit, parent) {
                    dropped_markers.push(DroppedMarker {
                        commit: commit.id.clone(),
                        parent: parent.clone(),
                        tree: commit.tree.clone(),
                    });
                } else {
                    first_parent_non_merges.insert(commit.id.clone());
                }
            }
            [first_parent, feature_parent] => {
                if graph.main_ancestors.contains(feature_parent) {
                    continue;
                }
                let matches = matching_features(&surviving, feature_parent);
                let feature = match matches.as_slice() {
                    [] => {
                        return Err(InventoryError::DeletedFeatureRef {
                            merge_commit: commit.id.clone(),
                            feature_parent: feature_parent.clone(),
                        });
                    }
                    [feature] => *feature,
                    _ => {
                        return Err(InventoryError::AmbiguousFeatureRefs {
                            merge_commit: commit.id.clone(),
                            feature_parent: feature_parent.clone(),
                            branches: matches.iter().map(|feature| feature.name.clone()).collect(),
                        });
                    }
                };
                let historical = HistoricalMerge {
                    commit: commit.id.clone(),
                    first_parent: first_parent.clone(),
                    feature_parent: feature_parent.clone(),
                    tree: commit.tree.clone(),
                };
                if let Some(existing) = features
                    .iter_mut()
                    .find(|existing| existing.name == feature.name)
                {
                    existing.historical_merges.push(historical);
                } else {
                    features.push(ExplicitFeature {
                        name: feature.name.clone(),
                        tip: feature.tip.clone(),
                        historical_merges: vec![historical],
                    });
                }
            }
            parents if parents.len() > 2 => {
                return Err(InventoryError::OctopusMerge {
                    merge_commit: commit.id.clone(),
                    parents: parents.len(),
                });
            }
            [] => {
                return Err(InventoryError::DirectCommit {
                    commit: commit.id.clone(),
                });
            }
            _ => {}
        }
    }

    let mut attributed_commits = Vec::new();
    for id in graph
        .environment_ancestors
        .difference(&graph.main_ancestors)
    {
        let commit = graph
            .commits
            .get(id)
            .ok_or_else(|| InventoryError::MissingCommit { commit: id.clone() })?;
        if commit.parents.len() > 1 || dropped_markers.iter().any(|marker| marker.commit == *id) {
            continue;
        }
        let branches = features
            .iter()
            .filter(|feature| {
                feature.historical_merges.iter().any(|historical| {
                    graph
                        .feature_refs
                        .iter()
                        .find(|candidate| candidate.name == feature.name)
                        .is_some_and(|candidate| candidate.ancestors.contains(id))
                        && graph
                            .environment_ancestors
                            .contains(&historical.feature_parent)
                })
            })
            .map(|feature| feature.name.clone())
            .collect::<Vec<_>>();
        if branches.is_empty() {
            if first_parent_non_merges.contains(id) {
                let containing = matching_features(&surviving, id)
                    .iter()
                    .map(|feature| feature.name.clone())
                    .collect::<Vec<_>>();
                if !containing.is_empty() {
                    return Err(InventoryError::FastForwardHistory {
                        commit: id.clone(),
                        branches: containing,
                    });
                }
            }
            return Err(InventoryError::DirectCommit { commit: id.clone() });
        }
        attributed_commits.push(AttributedCommit {
            commit: id.clone(),
            branches,
        });
    }

    let explicit_names = features
        .iter()
        .map(|feature| feature.name.as_str())
        .collect::<BTreeSet<_>>();
    let graduated_features = graph
        .feature_refs
        .iter()
        .filter(|feature| graph.main_ancestors.contains(&feature.tip))
        .map(branch_identity)
        .collect();
    let indirect_features = graph
        .feature_refs
        .iter()
        .filter(|feature| {
            !graph.main_ancestors.contains(&feature.tip)
                && !explicit_names.contains(feature.name.as_str())
                && feature.ancestors.iter().any(|commit| {
                    graph.environment_ancestors.contains(commit)
                        && !graph.main_ancestors.contains(commit)
                })
        })
        .map(branch_identity)
        .collect();

    Ok(RestackSnapshot {
        remote: graph.remote.clone(),
        environment: graph.environment.clone(),
        environment_ref: graph.environment_ref.clone(),
        environment_tip: graph.environment_tip.clone(),
        main: graph.main.clone(),
        main_ref: graph.main_ref.clone(),
        main_tip: graph.main_tip.clone(),
        features,
        graduated_features,
        indirect_features,
        dropped_markers,
        attributed_commits,
        inventory_mode: InventoryMode::History,
        unsupported_history: None,
        carried_features: Vec::new(),
        unattributed_commits: Vec::new(),
    })
}

impl From<InventoryError> for UnsupportedHistory {
    fn from(error: InventoryError) -> Self {
        match error {
            InventoryError::MissingCommit { commit } => Self {
                kind: "missingCommit".to_owned(),
                commit: Some(commit),
                feature_parent: None,
                branches: Vec::new(),
                parents: None,
            },
            InventoryError::DirectCommit { commit } => Self {
                kind: "directCommit".to_owned(),
                commit: Some(commit),
                feature_parent: None,
                branches: Vec::new(),
                parents: None,
            },
            InventoryError::FastForwardHistory { commit, branches } => Self {
                kind: "fastForwardHistory".to_owned(),
                commit: Some(commit),
                feature_parent: None,
                branches,
                parents: None,
            },
            InventoryError::OctopusMerge {
                merge_commit,
                parents,
            } => Self {
                kind: "octopusMerge".to_owned(),
                commit: Some(merge_commit),
                feature_parent: None,
                branches: Vec::new(),
                parents: Some(parents),
            },
            InventoryError::DeletedFeatureRef {
                merge_commit,
                feature_parent,
            } => Self {
                kind: "deletedFeatureRef".to_owned(),
                commit: Some(merge_commit),
                feature_parent: Some(feature_parent),
                branches: Vec::new(),
                parents: None,
            },
            InventoryError::AmbiguousFeatureRefs {
                merge_commit,
                feature_parent,
                branches,
            } => Self {
                kind: "ambiguousFeatureRefs".to_owned(),
                commit: Some(merge_commit),
                feature_parent: Some(feature_parent),
                branches,
                parents: None,
            },
        }
    }
}

/// Build the reachability inventory after the history proof failed.
///
/// Membership comes from remote tips reachable from the environment but not
/// from main. A candidate whose tip another candidate already reaches is
/// carried and never merged on its own; two refs at the same tip keep the
/// alphabetically first as the carrier. Top-level features are ordered by tip
/// author time, oldest first, then by name; a tip without a timestamp sorts
/// after every dated tip.
#[must_use]
pub fn build_inventory_snapshot(
    graph: &RestackGraph,
    reason: UnsupportedHistory,
    tip_timestamps: &BTreeMap<String, i64>,
) -> RestackSnapshot {
    let candidates = graph
        .feature_refs
        .iter()
        .filter(|feature| {
            graph.environment_ancestors.contains(&feature.tip)
                && !graph.main_ancestors.contains(&feature.tip)
        })
        .collect::<Vec<_>>();
    let carriers_of = |feature: &FeatureRef| -> Vec<String> {
        candidates
            .iter()
            .filter(|other| other.name != feature.name)
            .filter(|other| other.ancestors.contains(&feature.tip))
            .filter(|other| other.tip != feature.tip || other.name < feature.name)
            .map(|other| other.name.clone())
            .collect()
    };
    let mut carried_features = Vec::new();
    let mut top_level = Vec::new();
    for feature in &candidates {
        let carriers = carriers_of(feature);
        if carriers.is_empty() {
            top_level.push(*feature);
        } else {
            carried_features.push(CarriedFeature {
                name: feature.name.clone(),
                tip: feature.tip.clone(),
                carriers,
            });
        }
    }
    // A carrier that is itself carried never becomes a merge, so it cannot be
    // what brings a branch in; keep only the top-level merges as carriers.
    for carried in &mut carried_features {
        carried
            .carriers
            .retain(|carrier| top_level.iter().any(|feature| feature.name == *carrier));
    }
    top_level.sort_by(|left, right| {
        let left_time = tip_timestamps.get(&left.tip);
        let right_time = tip_timestamps.get(&right.tip);
        match (left_time, right_time) {
            (Some(left_time), Some(right_time)) => left_time
                .cmp(right_time)
                .then_with(|| left.name.cmp(&right.name)),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => left.name.cmp(&right.name),
        }
    });

    let unique = graph
        .environment_ancestors
        .difference(&graph.main_ancestors)
        .collect::<Vec<_>>();
    let dropped_markers = unique
        .iter()
        .filter_map(|id| {
            let commit = graph.commits.get(*id)?;
            let [parent] = commit.parents.as_slice() else {
                return None;
            };
            is_dropped_marker(graph, commit, parent).then(|| DroppedMarker {
                commit: commit.id.clone(),
                parent: parent.clone(),
                tree: commit.tree.clone(),
            })
        })
        .collect::<Vec<_>>();
    let mut attributed_commits = Vec::new();
    let mut unattributed_commits = Vec::new();
    for id in unique
        .iter()
        .filter(|id| {
            graph
                .commits
                .get(**id)
                .is_some_and(|commit| commit.parents.len() == 1)
        })
        .filter(|id| dropped_markers.iter().all(|marker| marker.commit != ***id))
    {
        let branches = top_level
            .iter()
            .filter(|feature| feature.ancestors.contains(*id))
            .map(|feature| feature.name.clone())
            .collect::<Vec<_>>();
        if branches.is_empty() {
            unattributed_commits.push((*id).clone());
        } else {
            attributed_commits.push(AttributedCommit {
                commit: (*id).clone(),
                branches,
            });
        }
    }

    RestackSnapshot {
        remote: graph.remote.clone(),
        environment: graph.environment.clone(),
        environment_ref: graph.environment_ref.clone(),
        environment_tip: graph.environment_tip.clone(),
        main: graph.main.clone(),
        main_ref: graph.main_ref.clone(),
        main_tip: graph.main_tip.clone(),
        features: top_level
            .iter()
            .map(|feature| ExplicitFeature {
                name: feature.name.clone(),
                tip: feature.tip.clone(),
                historical_merges: Vec::new(),
            })
            .collect(),
        graduated_features: graph
            .feature_refs
            .iter()
            .filter(|feature| graph.main_ancestors.contains(&feature.tip))
            .map(branch_identity)
            .collect(),
        indirect_features: carried_features
            .iter()
            .map(|carried| BranchIdentity {
                name: carried.name.clone(),
                tip: carried.tip.clone(),
            })
            .collect(),
        dropped_markers,
        attributed_commits,
        inventory_mode: InventoryMode::Reachability,
        unsupported_history: Some(reason),
        carried_features,
        unattributed_commits,
    }
}

/// Commits the rebuilt environment will drop for one retained set.
///
/// In history mode removals are explicit and the proof already attributed
/// every commit, so nothing is orphaned. In reachability mode every
/// environment-only commit that no retained top-level feature reaches is
/// orphaned, whether nothing ever reached it or its only owners were removed.
#[must_use]
pub fn orphaned_commit_ids(snapshot: &RestackSnapshot, retained: &[BranchIdentity]) -> Vec<String> {
    if snapshot.inventory_mode == InventoryMode::History {
        return Vec::new();
    }
    let mut orphans = snapshot
        .attributed_commits
        .iter()
        .filter(|commit| {
            !commit
                .branches
                .iter()
                .any(|owner| retained.iter().any(|kept| kept.name == *owner))
        })
        .map(|commit| commit.commit.clone())
        .chain(snapshot.unattributed_commits.iter().cloned())
        .collect::<Vec<_>>();
    orphans.sort();
    orphans.dedup();
    orphans
}

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

fn branch_identity_of(feature: &ExplicitFeature) -> BranchIdentity {
    BranchIdentity {
        name: feature.name.clone(),
        tip: feature.tip.clone(),
    }
}

fn branch_identity(feature: &FeatureRef) -> BranchIdentity {
    BranchIdentity {
        name: feature.name.clone(),
        tip: feature.tip.clone(),
    }
}

fn matching_features<'a>(features: &[&'a FeatureRef], commit: &str) -> Vec<&'a FeatureRef> {
    let mut matches = features
        .iter()
        .copied()
        .filter(|feature| feature.ancestors.contains(commit))
        .collect::<Vec<_>>();
    let exact = matches
        .iter()
        .copied()
        .filter(|feature| feature.tip == commit)
        .collect::<Vec<_>>();
    if !exact.is_empty() {
        matches = exact;
    }
    matches.sort_by(|left, right| left.name.cmp(&right.name));
    matches
}

fn is_dropped_marker(graph: &RestackGraph, commit: &GraphCommit, parent: &str) -> bool {
    commit.message.trim_end_matches('\n') == format!("### Match '{}'", graph.environment)
        && graph
            .commits
            .get(parent)
            .is_some_and(|parent| parent.tree == commit.tree)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_orders_first_explicit_merges_and_records_history_markers_and_attribution(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let mut graph = base_graph();
        add_commit(&mut graph, "graduated", "tg", &["base"], "graduated");
        add_commit(
            &mut graph,
            "graduated-merge",
            "tgm",
            &["base", "graduated"],
            "old graduated merge",
        );
        add_commit(&mut graph, "zeta-1", "tz1", &["base"], "zeta one");
        add_commit(
            &mut graph,
            "zeta-merge-1",
            "tzm1",
            &["graduated-merge", "zeta-1"],
            "first zeta merge",
        );
        add_commit(&mut graph, "zeta-2", "tz2", &["zeta-1"], "zeta two");
        add_commit(
            &mut graph,
            "zeta-merge-2",
            "tzm2",
            &["zeta-merge-1", "zeta-2"],
            "second zeta merge",
        );
        add_commit(
            &mut graph,
            "marker",
            "tzm2",
            &["zeta-merge-2"],
            "### Match 'qa'",
        );
        add_commit(&mut graph, "alpha", "ta", &["base"], "alpha");
        add_commit(
            &mut graph,
            "alpha-merge",
            "tam",
            &["marker", "alpha"],
            "alpha merge",
        );
        graph.environment_tip = "alpha-merge".to_owned();
        graph.environment_ancestors = ids(&[
            "base",
            "graduated",
            "graduated-merge",
            "zeta-1",
            "zeta-merge-1",
            "zeta-2",
            "zeta-merge-2",
            "marker",
            "alpha",
            "alpha-merge",
        ]);
        graph.main_tip = "main-tip".to_owned();
        graph.main_ancestors = ids(&["base", "graduated", "main-tip"]);
        graph.feature_refs = vec![
            feature("feature/alpha", "alpha", &["alpha", "base"]),
            feature("feature/zeta", "zeta-2", &["zeta-2", "zeta-1", "base"]),
            feature("feature/graduated", "graduated", &["graduated", "base"]),
        ];

        let snapshot = build_snapshot(&graph)?;

        assert_eq!(
            snapshot
                .features
                .iter()
                .map(|feature| feature.name.as_str())
                .collect::<Vec<_>>(),
            ["feature/zeta", "feature/alpha"]
        );
        assert_eq!(snapshot.features[0].historical_merges.len(), 2);
        assert_eq!(
            snapshot
                .graduated_features
                .iter()
                .map(|feature| feature.name.as_str())
                .collect::<Vec<_>>(),
            ["feature/graduated"]
        );
        assert_eq!(snapshot.dropped_markers.len(), 1);
        assert_eq!(snapshot.dropped_markers[0].commit, "marker");
        assert_eq!(
            snapshot
                .attributed_commits
                .iter()
                .map(|commit| commit.commit.as_str())
                .collect::<Vec<_>>(),
            ["alpha", "zeta-1", "zeta-2"]
        );
        Ok(())
    }

    #[test]
    fn snapshot_captures_the_current_tip_of_an_advanced_explicit_feature(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let mut graph = graph_with_merge(vec![feature(
            "feature/advanced",
            "advanced",
            &["advanced", "feature", "base"],
        )]);
        add_commit(&mut graph, "advanced", "ta", &["feature"], "advanced");

        let snapshot = build_snapshot(&graph)?;

        assert_eq!(snapshot.features.len(), 1);
        assert_eq!(snapshot.features[0].name, "feature/advanced");
        assert_eq!(snapshot.features[0].tip, "advanced");
        Ok(())
    }

    #[test]
    fn removal_selection_rejects_duplicate_unknown_graduated_and_indirect_names() {
        let snapshot = planning_snapshot();

        assert_eq!(
            select_features(&snapshot, &["feature/a".to_owned(), "feature/a".to_owned()]),
            Err(SelectionError::Duplicate {
                branch: "feature/a".to_owned()
            })
        );
        assert_eq!(
            select_features(&snapshot, &["feature/graduated".to_owned()]),
            Err(SelectionError::Graduated {
                branch: "feature/graduated".to_owned()
            })
        );
        assert_eq!(
            select_features(&snapshot, &["feature/indirect".to_owned()]),
            Err(SelectionError::IndirectOnly {
                branch: "feature/indirect".to_owned()
            })
        );
        assert_eq!(
            select_features(&snapshot, &["feature/missing".to_owned()]),
            Err(SelectionError::Unknown {
                branch: "feature/missing".to_owned()
            })
        );
    }

    #[test]
    fn removal_selection_reports_retained_branches_that_keep_feature_work() {
        let snapshot = planning_snapshot();

        assert_eq!(
            select_features(&snapshot, &["feature/a".to_owned()]),
            Err(SelectionError::RetainedDependency {
                branch: "feature/a".to_owned(),
                dependents: vec!["feature/b".to_owned()]
            })
        );
        let selection =
            select_features(&snapshot, &["feature/b".to_owned(), "feature/a".to_owned()]);
        assert!(
            matches!(selection, Ok(selection) if selection.retained.is_empty()
            && selection.removed.iter().map(|feature| feature.name.as_str()).collect::<Vec<_>>()
                == ["feature/a", "feature/b"])
        );
    }

    #[test]
    fn plan_digest_binds_inputs_identity_selection_and_tree_but_not_preview_commit(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let mut snapshot = planning_snapshot();
        snapshot.attributed_commits.clear();
        let selection = select_features(&snapshot, &["feature/b".to_owned()])?;
        let author = RestackAuthor {
            name: "Test Author".to_owned(),
            email: "test@example.com".to_owned(),
        };
        let endpoints = RemoteEndpointIdentity {
            fetch_sha256: "11".repeat(32),
            push_sha256: "22".repeat(32),
        };
        let outcomes = vec![MergeOutcome {
            branch: "feature/a".to_owned(),
            tip: "a".to_owned(),
            commit: "preview-one".to_owned(),
            tree: "tree-a".to_owned(),
            resolution: MergeResolution::Clean,
        }];

        let first = build_plan(
            snapshot.clone(),
            endpoints.clone(),
            author.clone(),
            selection.clone(),
            Reconstruction {
                merges: outcomes.clone(),
                final_tree: "final-tree".to_owned(),
                preview_commit: "preview-one".to_owned(),
            },
            Vec::new(),
        )?;
        let mut regenerated = outcomes;
        regenerated[0].commit = "preview-two".to_owned();
        let second = build_plan(
            snapshot.clone(),
            endpoints.clone(),
            author.clone(),
            selection.clone(),
            Reconstruction {
                merges: regenerated.clone(),
                final_tree: "final-tree".to_owned(),
                preview_commit: "preview-two".to_owned(),
            },
            Vec::new(),
        )?;
        let changed_author = build_plan(
            snapshot.clone(),
            endpoints.clone(),
            RestackAuthor {
                name: "Other Author".to_owned(),
                email: author.email.clone(),
            },
            selection.clone(),
            Reconstruction {
                merges: regenerated,
                final_tree: "final-tree".to_owned(),
                preview_commit: "preview-two".to_owned(),
            },
            Vec::new(),
        )?;
        let changed_tree = build_plan(
            snapshot.clone(),
            endpoints.clone(),
            author.clone(),
            selection.clone(),
            Reconstruction {
                merges: vec![MergeOutcome {
                    branch: "feature/a".to_owned(),
                    tip: "a".to_owned(),
                    commit: "preview-three".to_owned(),
                    tree: "tree-a".to_owned(),
                    resolution: MergeResolution::Clean,
                }],
                final_tree: "other-tree".to_owned(),
                preview_commit: "preview-three".to_owned(),
            },
            Vec::new(),
        )?;
        let changed_endpoint = build_plan(
            snapshot,
            RemoteEndpointIdentity {
                fetch_sha256: endpoints.fetch_sha256,
                push_sha256: "33".repeat(32),
            },
            author,
            selection,
            Reconstruction {
                merges: vec![MergeOutcome {
                    branch: "feature/a".to_owned(),
                    tip: "a".to_owned(),
                    commit: "preview-four".to_owned(),
                    tree: "tree-a".to_owned(),
                    resolution: MergeResolution::Clean,
                }],
                final_tree: "final-tree".to_owned(),
                preview_commit: "preview-four".to_owned(),
            },
            Vec::new(),
        )?;

        assert_eq!(first.digest, second.digest);
        assert_eq!(
            first.digest,
            "9077fbf6b660331cea97f2be2209570096b124619a0256774ddb6833c3891e4c"
        );
        assert_ne!(first.digest, changed_author.digest);
        assert_ne!(first.digest, changed_tree.digest);
        assert_ne!(first.digest, changed_endpoint.digest);
        assert_eq!(first.digest.len(), 64);
        Ok(())
    }

    #[test]
    fn direct_environment_work_is_rejected_with_its_commit() {
        let mut graph = base_graph();
        add_commit(&mut graph, "direct", "td", &["base"], "direct");
        graph.environment_tip = "direct".to_owned();
        graph.environment_ancestors = ids(&["base", "direct"]);

        assert_eq!(
            build_snapshot(&graph).err(),
            Some(InventoryError::DirectCommit {
                commit: "direct".to_owned()
            })
        );
    }

    #[test]
    fn fast_forward_history_is_distinct_from_direct_work() {
        let mut graph = base_graph();
        add_commit(&mut graph, "feature", "tf", &["base"], "feature");
        graph.environment_tip = "feature".to_owned();
        graph.environment_ancestors = ids(&["base", "feature"]);
        graph.feature_refs = vec![feature(
            "feature/fast-forwarded",
            "feature",
            &["feature", "base"],
        )];

        assert_eq!(
            build_snapshot(&graph).err(),
            Some(InventoryError::FastForwardHistory {
                commit: "feature".to_owned(),
                branches: vec!["feature/fast-forwarded".to_owned()]
            })
        );
    }

    #[test]
    fn deleted_feature_ref_is_rejected_with_merge_parent_evidence() {
        let graph = graph_with_merge(Vec::new());

        assert_eq!(
            build_snapshot(&graph).err(),
            Some(InventoryError::DeletedFeatureRef {
                merge_commit: "merge".to_owned(),
                feature_parent: "feature".to_owned()
            })
        );
    }

    #[test]
    fn octopus_merge_is_rejected_with_parent_count() {
        let mut graph = base_graph();
        add_commit(&mut graph, "one", "t1", &["base"], "one");
        add_commit(&mut graph, "two", "t2", &["base"], "two");
        add_commit(
            &mut graph,
            "octopus",
            "to",
            &["base", "one", "two"],
            "octopus",
        );
        graph.environment_tip = "octopus".to_owned();
        graph.environment_ancestors = ids(&["base", "one", "two", "octopus"]);

        assert_eq!(
            build_snapshot(&graph).err(),
            Some(InventoryError::OctopusMerge {
                merge_commit: "octopus".to_owned(),
                parents: 3
            })
        );
    }

    #[test]
    fn aliased_feature_tips_are_rejected_as_ambiguous() {
        let graph = graph_with_merge(vec![
            feature("feature/one", "feature", &["feature", "base"]),
            feature("feature/two", "feature", &["feature", "base"]),
        ]);

        assert_eq!(
            build_snapshot(&graph).err(),
            Some(InventoryError::AmbiguousFeatureRefs {
                merge_commit: "merge".to_owned(),
                feature_parent: "feature".to_owned(),
                branches: vec!["feature/one".to_owned(), "feature/two".to_owned()]
            })
        );
    }

    #[test]
    fn only_an_exact_empty_phase_marker_is_dropped() {
        let mut exact = base_graph();
        add_commit(&mut exact, "marker", "tb", &["base"], "### Match 'qa'");
        exact.environment_tip = "marker".to_owned();
        exact.environment_ancestors = ids(&["base", "marker"]);
        let snapshot = build_snapshot(&exact);
        assert!(matches!(snapshot, Ok(snapshot) if snapshot.dropped_markers.len() == 1));

        let mut changed_tree = base_graph();
        add_commit(
            &mut changed_tree,
            "marker",
            "different",
            &["base"],
            "### Match 'qa'",
        );
        changed_tree.environment_tip = "marker".to_owned();
        changed_tree.environment_ancestors = ids(&["base", "marker"]);
        assert!(matches!(
            build_snapshot(&changed_tree),
            Err(InventoryError::DirectCommit { commit }) if commit == "marker"
        ));

        let mut other_empty = base_graph();
        add_commit(
            &mut other_empty,
            "empty",
            "tb",
            &["base"],
            "ordinary empty commit",
        );
        other_empty.environment_tip = "empty".to_owned();
        other_empty.environment_ancestors = ids(&["base", "empty"]);
        assert!(matches!(
            build_snapshot(&other_empty),
            Err(InventoryError::DirectCommit { commit }) if commit == "empty"
        ));
    }

    #[test]
    fn interaction_starts_with_every_feature_retained_in_merge_order() {
        let mut interaction = RestackInteraction::new(planning_snapshot());

        assert_eq!(interaction.stage(), RestackInteractionStage::Selection);
        assert!(interaction.is_retained(0));
        assert!(interaction.is_retained(1));
        assert_eq!(
            interaction.update(RestackInteractionAction::Continue),
            RestackInteractionEffect::Preview(RestackSelection {
                retained: vec![
                    BranchIdentity {
                        name: "feature/a".to_owned(),
                        tip: "a".to_owned(),
                    },
                    BranchIdentity {
                        name: "feature/b".to_owned(),
                        tip: "b".to_owned(),
                    },
                ],
                removed: Vec::new(),
            })
        );
    }

    #[test]
    fn interaction_rejects_a_removal_that_a_retained_branch_still_carries() {
        let mut interaction = RestackInteraction::new(planning_snapshot());

        assert_eq!(
            interaction.retained_dependents(0),
            vec!["feature/b".to_owned()]
        );
        let effect = interaction.update(RestackInteractionAction::Toggle);

        assert_eq!(
            effect,
            RestackInteractionEffect::Rejected(SelectionError::RetainedDependency {
                branch: "feature/a".to_owned(),
                dependents: vec!["feature/b".to_owned()],
            })
        );
        assert!(interaction.is_retained(0));
    }

    #[test]
    fn interaction_supports_batch_selection_and_inventory_navigation() {
        let mut interaction = RestackInteraction::new(planning_snapshot());

        let _ = interaction.update(RestackInteractionAction::RemoveAll);
        assert!(!interaction.is_retained(0));
        assert!(!interaction.is_retained(1));

        let _ = interaction.update(RestackInteractionAction::KeepAll);
        assert!(interaction.is_retained(0));
        assert!(interaction.is_retained(1));

        let _ = interaction.update(RestackInteractionAction::MoveLast);
        assert_eq!(interaction.cursor(), 1);
        let _ = interaction.update(RestackInteractionAction::MoveFirst);
        assert_eq!(interaction.cursor(), 0);
        let _ = interaction.update(RestackInteractionAction::MovePageDown);
        assert_eq!(interaction.cursor(), 1);
        let _ = interaction.update(RestackInteractionAction::MovePageUp);
        assert_eq!(interaction.cursor(), 0);
        let _ = interaction.update(RestackInteractionAction::MoveTo(1));
        assert_eq!(interaction.cursor(), 1);
    }

    #[test]
    fn interaction_requires_review_then_explicit_confirmation() {
        let mut interaction = RestackInteraction::new(planning_snapshot());
        let _ = interaction.update(RestackInteractionAction::MoveDown);
        assert_eq!(interaction.cursor(), 1);
        let _ = interaction.update(RestackInteractionAction::MoveUp);
        assert_eq!(interaction.cursor(), 0);
        interaction.review_ready();

        assert!(!interaction.review_details());
        let _ = interaction.update(RestackInteractionAction::ToggleDetails);
        assert!(interaction.review_details());

        let _ = interaction.update(RestackInteractionAction::MoveDown);
        assert_eq!(interaction.review_scroll(), 1);
        let _ = interaction.update(RestackInteractionAction::MoveUp);
        assert_eq!(interaction.review_scroll(), 0);
        let _ = interaction.update(RestackInteractionAction::MoveLast);
        assert_eq!(interaction.review_scroll(), usize::MAX);
        let _ = interaction.update(RestackInteractionAction::MoveFirst);
        assert_eq!(interaction.review_scroll(), 0);

        assert_eq!(
            interaction.update(RestackInteractionAction::Continue),
            RestackInteractionEffect::None
        );
        assert_eq!(interaction.stage(), RestackInteractionStage::Confirmation);
        assert_eq!(
            interaction.update(RestackInteractionAction::Confirm),
            RestackInteractionEffect::Publish
        );
        assert_eq!(
            interaction.update(RestackInteractionAction::Back),
            RestackInteractionEffect::None
        );
        assert_eq!(interaction.stage(), RestackInteractionStage::Review);
        assert_eq!(
            interaction.update(RestackInteractionAction::Back),
            RestackInteractionEffect::Revise
        );
        assert_eq!(interaction.stage(), RestackInteractionStage::Selection);
    }

    fn base_graph() -> RestackGraph {
        let mut commits = BTreeMap::new();
        commits.insert(
            "base".to_owned(),
            GraphCommit {
                id: "base".to_owned(),
                tree: "tb".to_owned(),
                parents: Vec::new(),
                message: "base".to_owned(),
            },
        );
        RestackGraph {
            remote: "origin".to_owned(),
            environment: "qa".to_owned(),
            environment_ref: "refs/remotes/origin/qa".to_owned(),
            environment_tip: "base".to_owned(),
            main: "main".to_owned(),
            main_ref: "refs/remotes/origin/main".to_owned(),
            main_tip: "base".to_owned(),
            environment_ancestors: ids(&["base"]),
            main_ancestors: ids(&["base"]),
            feature_refs: Vec::new(),
            commits,
        }
    }

    fn graph_with_merge(feature_refs: Vec<FeatureRef>) -> RestackGraph {
        let mut graph = base_graph();
        add_commit(&mut graph, "feature", "tf", &["base"], "feature");
        add_commit(&mut graph, "merge", "tm", &["base", "feature"], "merge");
        graph.environment_tip = "merge".to_owned();
        graph.environment_ancestors = ids(&["base", "feature", "merge"]);
        graph.feature_refs = feature_refs;
        graph
    }

    fn planning_snapshot() -> RestackSnapshot {
        RestackSnapshot {
            remote: "origin".to_owned(),
            environment: "qa".to_owned(),
            environment_ref: "refs/remotes/origin/qa".to_owned(),
            environment_tip: "environment".to_owned(),
            main: "main".to_owned(),
            main_ref: "refs/remotes/origin/main".to_owned(),
            main_tip: "main-tip".to_owned(),
            features: vec![
                ExplicitFeature {
                    name: "feature/a".to_owned(),
                    tip: "a".to_owned(),
                    historical_merges: Vec::new(),
                },
                ExplicitFeature {
                    name: "feature/b".to_owned(),
                    tip: "b".to_owned(),
                    historical_merges: Vec::new(),
                },
            ],
            graduated_features: vec![BranchIdentity {
                name: "feature/graduated".to_owned(),
                tip: "graduated".to_owned(),
            }],
            indirect_features: vec![BranchIdentity {
                name: "feature/indirect".to_owned(),
                tip: "indirect".to_owned(),
            }],
            dropped_markers: Vec::new(),
            attributed_commits: vec![AttributedCommit {
                commit: "shared".to_owned(),
                branches: vec!["feature/a".to_owned(), "feature/b".to_owned()],
            }],
            inventory_mode: InventoryMode::History,
            unsupported_history: None,
            carried_features: Vec::new(),
            unattributed_commits: Vec::new(),
        }
    }

    #[test]
    fn history_snapshot_reports_history_mode_without_fallback_evidence(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let snapshot = build_snapshot(&simple_graph())?;
        assert_eq!(snapshot.inventory_mode, InventoryMode::History);
        assert_eq!(snapshot.unsupported_history, None);
        assert!(snapshot.carried_features.is_empty());
        Ok(())
    }

    #[test]
    fn plan_and_snapshot_round_trip_every_inventory_field() -> Result<(), Box<dyn std::error::Error>>
    {
        assert_eq!(RESTACK_SCHEMA_VERSION, 2);
        let mut snapshot = build_snapshot(&simple_graph())?;
        snapshot.inventory_mode = InventoryMode::Reachability;
        snapshot.unsupported_history = Some(UnsupportedHistory {
            kind: "ambiguousFeatureRefs".to_owned(),
            commit: Some("merge".to_owned()),
            feature_parent: Some("feature".to_owned()),
            branches: vec!["feature/a".to_owned(), "feature/b".to_owned()],
            parents: None,
        });
        snapshot.carried_features = vec![CarriedFeature {
            name: "feature/b".to_owned(),
            tip: "feature".to_owned(),
            carriers: vec!["feature/a".to_owned()],
        }];
        snapshot.unattributed_commits = vec!["lost".to_owned()];
        let encoded = serde_json::to_string(&snapshot)?;
        let decoded: RestackSnapshot = serde_json::from_str(&encoded)?;
        assert_eq!(decoded, snapshot);

        let orphan = OrphanedCommit {
            commit: "lost".to_owned(),
            subject: "lost work".to_owned(),
            author: "Pat".to_owned(),
            date: "2026-01-02".to_owned(),
        };
        let encoded = serde_json::to_string(&orphan)?;
        assert_eq!(
            encoded,
            r#"{"commit":"lost","subject":"lost work","author":"Pat","date":"2026-01-02"}"#
        );
        let selection = select_features(&snapshot, &[])?;
        let plan = build_plan(
            snapshot.clone(),
            RemoteEndpointIdentity {
                fetch_sha256: "f".repeat(64),
                push_sha256: "p".repeat(64),
            },
            RestackAuthor {
                name: "Pat".to_owned(),
                email: "pat@example.com".to_owned(),
            },
            selection.clone(),
            Reconstruction {
                merges: vec![MergeOutcome {
                    branch: "feature/a".to_owned(),
                    tip: "feature".to_owned(),
                    commit: "preview".to_owned(),
                    tree: "tree".to_owned(),
                    resolution: MergeResolution::Clean,
                }],
                final_tree: "tree".to_owned(),
                preview_commit: "preview".to_owned(),
            },
            vec![orphan.clone()],
        )?;
        assert_eq!(plan.orphaned_commits, vec![orphan]);
        Ok(())
    }

    /// main: base; environment: base -> merge(feature/a at `feature`).
    fn simple_graph() -> RestackGraph {
        let mut graph = RestackGraph {
            remote: "origin".to_owned(),
            environment: "qa".to_owned(),
            environment_ref: "refs/remotes/origin/qa".to_owned(),
            environment_tip: "merge".to_owned(),
            main: "main".to_owned(),
            main_ref: "refs/remotes/origin/main".to_owned(),
            main_tip: "base".to_owned(),
            environment_ancestors: ids(&["base", "feature", "merge"]),
            main_ancestors: ids(&["base"]),
            feature_refs: vec![feature("feature/a", "feature", &["feature"])],
            commits: BTreeMap::new(),
        };
        add_commit(&mut graph, "base", "tb", &[], "base");
        add_commit(&mut graph, "feature", "tf", &["base"], "feature work");
        add_commit(&mut graph, "merge", "tm", &["base", "feature"], "merge");
        graph
    }

    #[test]
    fn inventory_snapshot_splits_top_level_and_carried_and_attributes_reached_work(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let graph = ambiguous_graph();
        let reason = UnsupportedHistory::from(InventoryError::AmbiguousFeatureRefs {
            merge_commit: "a2".to_owned(),
            feature_parent: "a1b".to_owned(),
            branches: vec!["feature/a".to_owned(), "feature/b".to_owned()],
        });

        let snapshot = build_inventory_snapshot(&graph, reason.clone(), &BTreeMap::new());

        assert_eq!(snapshot.inventory_mode, InventoryMode::Reachability);
        assert_eq!(snapshot.unsupported_history, Some(reason));
        assert_eq!(names(&snapshot.features), ["feature/b"]);
        assert!(snapshot.features[0].historical_merges.is_empty());
        assert_eq!(
            snapshot.carried_features,
            vec![CarriedFeature {
                name: "feature/a".to_owned(),
                tip: "a2".to_owned(),
                carriers: vec!["feature/b".to_owned()],
            }]
        );
        assert_eq!(
            snapshot.indirect_features,
            vec![BranchIdentity {
                name: "feature/a".to_owned(),
                tip: "a2".to_owned(),
            }]
        );
        assert_eq!(
            snapshot.graduated_features,
            vec![BranchIdentity {
                name: "feature/gone".to_owned(),
                tip: "base".to_owned(),
            }]
        );
        assert_eq!(snapshot.dropped_markers.len(), 1);
        assert_eq!(snapshot.dropped_markers[0].commit, "marker");
        let attributed = snapshot
            .attributed_commits
            .iter()
            .map(|commit| commit.commit.as_str())
            .collect::<Vec<_>>();
        assert_eq!(attributed, ["a1", "a1b", "b1"]);
        assert!(snapshot
            .attributed_commits
            .iter()
            .all(|commit| commit.branches == ["feature/b"]));
        Ok(())
    }

    #[test]
    fn inventory_chain_keeps_only_the_outermost_tip_as_the_single_carrier() {
        let mut graph = empty_graph("c");
        for id in ["base", "a", "b", "c"] {
            add_commit(&mut graph, id, id, &[], id);
        }
        graph.environment_ancestors = ids(&["base", "a", "b", "c"]);
        graph.main_ancestors = ids(&["base"]);
        graph.feature_refs = vec![
            feature("feature/a", "a", &["a"]),
            feature("feature/b", "b", &["a", "b"]),
            feature("feature/c", "c", &["a", "b", "c"]),
        ];
        let snapshot = build_inventory_snapshot(&graph, direct_reason(), &BTreeMap::new());
        assert_eq!(names(&snapshot.features), ["feature/c"]);
        assert_eq!(
            snapshot
                .carried_features
                .iter()
                .map(|carried| (carried.name.as_str(), carried.carriers.clone()))
                .collect::<Vec<_>>(),
            [
                ("feature/a", vec!["feature/c".to_owned()]),
                ("feature/b", vec!["feature/c".to_owned()]),
            ]
        );
    }

    #[test]
    fn inventory_diamond_keeps_both_carriers_and_orders_by_tip_age_then_name() {
        let mut graph = empty_graph("env");
        for id in ["base", "x", "left", "right", "same", "undated"] {
            add_commit(&mut graph, id, id, &[], id);
        }
        graph.environment_ancestors = ids(&["base", "x", "left", "right", "same", "undated"]);
        graph.main_ancestors = ids(&["base"]);
        graph.feature_refs = vec![
            feature("feature/right", "right", &["x", "right"]),
            feature("feature/left", "left", &["x", "left"]),
            feature("feature/x", "x", &["x"]),
            feature("feature/zzz-same", "same", &["same"]),
            feature("feature/aaa-same", "same", &["same"]),
            feature("feature/undated", "undated", &["undated"]),
        ];
        let timestamps = BTreeMap::from([
            ("left".to_owned(), 200),
            ("right".to_owned(), 100),
            ("same".to_owned(), 100),
        ]);
        let snapshot = build_inventory_snapshot(&graph, direct_reason(), &timestamps);
        assert_eq!(
            names(&snapshot.features),
            [
                "feature/aaa-same",
                "feature/right",
                "feature/left",
                "feature/undated"
            ]
        );
        assert_eq!(
            snapshot
                .carried_features
                .iter()
                .map(|carried| (carried.name.as_str(), carried.carriers.clone()))
                .collect::<Vec<_>>(),
            [
                (
                    "feature/x",
                    vec!["feature/right".to_owned(), "feature/left".to_owned()]
                ),
                ("feature/zzz-same", vec!["feature/aaa-same".to_owned()]),
            ]
        );
    }

    #[test]
    fn every_inventory_error_becomes_fallback_evidence() {
        let cases = [
            (
                InventoryError::MissingCommit {
                    commit: "m".to_owned(),
                },
                ("missingCommit", Some("m"), None, vec![], None),
            ),
            (
                InventoryError::DirectCommit {
                    commit: "d".to_owned(),
                },
                ("directCommit", Some("d"), None, vec![], None),
            ),
            (
                InventoryError::FastForwardHistory {
                    commit: "f".to_owned(),
                    branches: vec!["a".to_owned()],
                },
                ("fastForwardHistory", Some("f"), None, vec!["a"], None),
            ),
            (
                InventoryError::OctopusMerge {
                    merge_commit: "o".to_owned(),
                    parents: 3,
                },
                ("octopusMerge", Some("o"), None, vec![], Some(3)),
            ),
            (
                InventoryError::DeletedFeatureRef {
                    merge_commit: "x".to_owned(),
                    feature_parent: "p".to_owned(),
                },
                ("deletedFeatureRef", Some("x"), Some("p"), vec![], None),
            ),
            (
                InventoryError::AmbiguousFeatureRefs {
                    merge_commit: "y".to_owned(),
                    feature_parent: "q".to_owned(),
                    branches: vec!["a".to_owned(), "b".to_owned()],
                },
                (
                    "ambiguousFeatureRefs",
                    Some("y"),
                    Some("q"),
                    vec!["a", "b"],
                    None,
                ),
            ),
        ];
        for (error, (kind, commit, feature_parent, branches, parents)) in cases {
            let evidence = UnsupportedHistory::from(error);
            assert_eq!(evidence.kind, kind);
            assert_eq!(evidence.commit.as_deref(), commit);
            assert_eq!(evidence.feature_parent.as_deref(), feature_parent);
            assert_eq!(evidence.branches, branches);
            assert_eq!(evidence.parents, parents);
        }
    }

    #[test]
    fn inventory_snapshot_records_work_no_top_level_feature_reaches() {
        let snapshot =
            build_inventory_snapshot(&ambiguous_graph(), direct_reason(), &BTreeMap::new());
        assert_eq!(snapshot.unattributed_commits, ["stray"]);
    }

    #[test]
    fn orphans_follow_the_retained_set_and_never_include_merges_or_markers(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let snapshot =
            build_inventory_snapshot(&ambiguous_graph(), direct_reason(), &BTreeMap::new());
        let everything = select_features(&snapshot, &[])?;
        assert_eq!(
            orphaned_commit_ids(&snapshot, &everything.retained),
            ["stray"]
        );
        let nothing = select_features(&snapshot, &["feature/b".to_owned()])?;
        assert_eq!(
            orphaned_commit_ids(&snapshot, &nothing.retained),
            ["a1", "a1b", "b1", "stray"]
        );
        assert_eq!(
            select_features(&snapshot, &["feature/a".to_owned()]),
            Err(SelectionError::IndirectOnly {
                branch: "feature/a".to_owned()
            })
        );
        let history = build_snapshot(&simple_graph())?;
        assert!(orphaned_commit_ids(&history, &[]).is_empty());
        Ok(())
    }

    #[test]
    fn plan_requires_the_exact_orphan_rows_and_binds_mode_and_orphans_into_the_digest(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let snapshot =
            build_inventory_snapshot(&ambiguous_graph(), direct_reason(), &BTreeMap::new());
        let selection = select_features(&snapshot, &[])?;
        let reconstruction = || Reconstruction {
            merges: vec![MergeOutcome {
                branch: "feature/b".to_owned(),
                tip: "b1".to_owned(),
                commit: "preview".to_owned(),
                tree: "tree".to_owned(),
                resolution: MergeResolution::Clean,
            }],
            final_tree: "tree".to_owned(),
            preview_commit: "preview".to_owned(),
        };
        let author = RestackAuthor {
            name: "Pat".to_owned(),
            email: "pat@example.com".to_owned(),
        };
        let endpoints = RemoteEndpointIdentity {
            fetch_sha256: "f".repeat(64),
            push_sha256: "p".repeat(64),
        };
        let orphan = |commit: &str| OrphanedCommit {
            commit: commit.to_owned(),
            subject: "s".to_owned(),
            author: "Pat".to_owned(),
            date: "2026-01-01".to_owned(),
        };

        assert_eq!(
            build_plan(
                snapshot.clone(),
                endpoints.clone(),
                author.clone(),
                selection.clone(),
                reconstruction(),
                Vec::new(),
            )
            .map(|plan| plan.digest),
            Err(PlanError::OrphanedCommits {
                expected: 1,
                actual: 0,
                mismatch: "stray".to_owned(),
            })
        );
        let plan = build_plan(
            snapshot.clone(),
            endpoints.clone(),
            author.clone(),
            selection.clone(),
            reconstruction(),
            vec![orphan("stray")],
        )?;

        let mut as_history = snapshot.clone();
        as_history.inventory_mode = InventoryMode::History;
        let history_plan = build_plan(
            as_history,
            endpoints.clone(),
            author.clone(),
            selection.clone(),
            reconstruction(),
            Vec::new(),
        )?;
        assert_ne!(plan.digest, history_plan.digest);

        let mut extra_orphan = snapshot;
        extra_orphan
            .unattributed_commits
            .push("stray-two".to_owned());
        let more_orphans = build_plan(
            extra_orphan,
            endpoints,
            author,
            selection,
            reconstruction(),
            vec![orphan("stray"), orphan("stray-two")],
        )?;
        assert_ne!(plan.digest, more_orphans.digest);
        Ok(())
    }

    /// main: base. feature/a: base -> a1 -> a2(pull-merge of a1 and a1b).
    /// feature/b branched from feature/a: a2 -> b1. Environment spine:
    /// base -> a1 -> a2 -> envm(merge b1) -> stray (direct work) -> marker.
    fn ambiguous_graph() -> RestackGraph {
        let mut graph = empty_graph("marker");
        add_commit(&mut graph, "base", "tb", &[], "base");
        add_commit(&mut graph, "a1", "ta1", &["base"], "a one");
        add_commit(&mut graph, "a1b", "ta1b", &["base"], "a one b");
        add_commit(
            &mut graph,
            "a2",
            "ta2",
            &["a1", "a1b"],
            "Merge branch 'feature/a' into feature/a",
        );
        add_commit(&mut graph, "b1", "tb1", &["a2"], "b one");
        add_commit(&mut graph, "envm", "tenv", &["a2", "b1"], "merge b");
        add_commit(&mut graph, "stray", "tstray", &["envm"], "direct work");
        add_commit(&mut graph, "marker", "tstray", &["stray"], "### Match 'qa'");
        graph.environment_ancestors =
            ids(&["base", "a1", "a1b", "a2", "b1", "envm", "stray", "marker"]);
        graph.main_ancestors = ids(&["base"]);
        graph.feature_refs = vec![
            feature("feature/a", "a2", &["a1", "a1b", "a2"]),
            feature("feature/b", "b1", &["a1", "a1b", "a2", "b1"]),
            feature("feature/gone", "base", &[]),
            feature("feature/unmerged", "elsewhere", &[]),
        ];
        graph
    }

    #[test]
    fn inventory_interaction_reaches_the_checklist_only_by_explicit_acceptance() {
        let snapshot =
            build_inventory_snapshot(&ambiguous_graph(), direct_reason(), &BTreeMap::new());
        let mut interaction = RestackInteraction::from_inventory(snapshot);
        assert_eq!(
            interaction.stage(),
            RestackInteractionStage::UnsupportedHistory
        );
        assert_eq!(interaction.inventory_mode(), InventoryMode::Reachability);
        assert_eq!(
            interaction
                .unsupported_history()
                .map(|reason| reason.kind.as_str()),
            Some("directCommit")
        );
        assert_eq!(interaction.carried_features().len(), 1);
        assert_eq!(interaction.orphaned_commit_count(), 1);

        for action in [
            RestackInteractionAction::Toggle,
            RestackInteractionAction::Continue,
            RestackInteractionAction::Confirm,
            RestackInteractionAction::Back,
            RestackInteractionAction::RemoveAll,
        ] {
            assert_eq!(interaction.update(action), RestackInteractionEffect::None);
            assert_eq!(
                interaction.stage(),
                RestackInteractionStage::UnsupportedHistory
            );
        }
        assert!(interaction.is_retained(0));
        assert_eq!(
            interaction.update(RestackInteractionAction::Cancel),
            RestackInteractionEffect::Cancel
        );

        assert_eq!(
            interaction.update(RestackInteractionAction::AcceptInventoryFallback),
            RestackInteractionEffect::None
        );
        assert_eq!(interaction.stage(), RestackInteractionStage::Selection);
        assert!(interaction.is_retained(0));
        assert_eq!(
            interaction.update(RestackInteractionAction::Toggle),
            RestackInteractionEffect::None
        );
        assert_eq!(interaction.orphaned_commit_count(), 4);
        assert_eq!(
            interaction.update(RestackInteractionAction::AcceptInventoryFallback),
            RestackInteractionEffect::None
        );
        assert_eq!(
            interaction.update(RestackInteractionAction::Back),
            RestackInteractionEffect::None
        );
        assert_eq!(interaction.stage(), RestackInteractionStage::Selection);
    }

    #[test]
    fn history_interaction_never_visits_the_unsupported_stage(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let interaction = RestackInteraction::new(build_snapshot(&simple_graph())?);
        assert_eq!(interaction.stage(), RestackInteractionStage::Selection);
        assert_eq!(interaction.inventory_mode(), InventoryMode::History);
        assert_eq!(interaction.orphaned_commit_count(), 0);
        Ok(())
    }

    fn direct_reason() -> UnsupportedHistory {
        UnsupportedHistory::from(InventoryError::DirectCommit {
            commit: "stray".to_owned(),
        })
    }

    fn names(features: &[ExplicitFeature]) -> Vec<&str> {
        features
            .iter()
            .map(|feature| feature.name.as_str())
            .collect()
    }

    fn empty_graph(environment_tip: &str) -> RestackGraph {
        RestackGraph {
            remote: "origin".to_owned(),
            environment: "qa".to_owned(),
            environment_ref: "refs/remotes/origin/qa".to_owned(),
            environment_tip: environment_tip.to_owned(),
            main: "main".to_owned(),
            main_ref: "refs/remotes/origin/main".to_owned(),
            main_tip: "base".to_owned(),
            environment_ancestors: BTreeSet::new(),
            main_ancestors: BTreeSet::new(),
            feature_refs: Vec::new(),
            commits: BTreeMap::new(),
        }
    }

    fn add_commit(graph: &mut RestackGraph, id: &str, tree: &str, parents: &[&str], message: &str) {
        graph.commits.insert(
            id.to_owned(),
            GraphCommit {
                id: id.to_owned(),
                tree: tree.to_owned(),
                parents: parents.iter().map(ToString::to_string).collect(),
                message: message.to_owned(),
            },
        );
    }

    fn feature(name: &str, tip: &str, ancestors: &[&str]) -> FeatureRef {
        FeatureRef {
            name: name.to_owned(),
            tip: tip.to_owned(),
            ancestors: ids(ancestors),
        }
    }

    fn ids(values: &[&str]) -> BTreeSet<String> {
        values.iter().map(ToString::to_string).collect()
    }
}
