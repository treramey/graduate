//! Explicit-merge snapshot construction.

use std::collections::{BTreeMap, BTreeSet};

use super::errors::InventoryError;
use super::taint::{
    absorbed_merges, environment_first_parent_merges, own_ancestors, tainted_features,
};
use super::{
    AttributedCommit, BranchIdentity, DroppedMarker, ExplicitFeature, FeatureRef, GraphCommit,
    HistoricalMerge, InventoryMode, RestackGraph, RestackSnapshot,
};

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

    let environment_merges = environment_first_parent_merges(graph);
    let absorbed: BTreeMap<&str, BTreeSet<String>> = surviving
        .iter()
        .map(|feature| {
            (
                feature.name.as_str(),
                absorbed_merges(graph, feature, &environment_merges),
            )
        })
        .collect();
    let owned: BTreeMap<&str, BTreeSet<String>> = surviving
        .iter()
        .map(|feature| {
            let absorbed = absorbed.get(feature.name.as_str());
            (
                feature.name.as_str(),
                absorbed.map_or_else(
                    || feature.ancestors.clone(),
                    |absorbed| own_ancestors(graph, feature, absorbed),
                ),
            )
        })
        .collect();
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
                    owned
                        .get(feature.name.as_str())
                        .is_some_and(|own| own.contains(id))
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
    let tainted_features = tainted_features(
        surviving
            .iter()
            .copied()
            .filter(|feature| explicit_names.contains(feature.name.as_str()))
            .map(|feature| {
                (
                    feature,
                    absorbed
                        .get(feature.name.as_str())
                        .cloned()
                        .unwrap_or_default(),
                )
            }),
    );

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
        tainted_features,
    })
}

pub(super) fn branch_identity_of(feature: &ExplicitFeature) -> BranchIdentity {
    BranchIdentity {
        name: feature.name.clone(),
        tip: feature.tip.clone(),
    }
}

pub(super) fn branch_identity(feature: &FeatureRef) -> BranchIdentity {
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

pub(super) fn is_dropped_marker(graph: &RestackGraph, commit: &GraphCommit, parent: &str) -> bool {
    commit.message.trim_end_matches('\n') == format!("### Match '{}'", graph.environment)
        && graph
            .commits
            .get(parent)
            .is_some_and(|parent| parent.tree == commit.tree)
}
