//! Plan and inventory errors with unsupported-history evidence.

use thiserror::Error;

use super::UnsupportedHistory;

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
