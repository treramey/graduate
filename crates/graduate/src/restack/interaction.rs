//! Terminal-independent review interaction state and queries.

use std::collections::BTreeSet;

use thiserror::Error;

use super::inventory::orphaned_commit_ids;
use super::plan::select_features;
use super::snapshot::branch_identity_of;
use super::{BranchIdentity, CarriedFeature, RestackSelection, RestackSnapshot};

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
    pub(super) snapshot: RestackSnapshot,
    pub(super) retained: Vec<bool>,
    pub(super) cursor: usize,
    pub(super) review_scroll: usize,
    pub(super) review_details: bool,
    pub(super) stage: RestackInteractionStage,
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

    /// Mark the selected reconstruction as ready for review.
    pub fn review_ready(&mut self) {
        self.stage = RestackInteractionStage::Review;
    }

    pub(super) fn toggle_current(&mut self) -> RestackInteractionEffect {
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

    pub(super) fn selection(&self) -> Result<RestackSelection, SelectionError> {
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
