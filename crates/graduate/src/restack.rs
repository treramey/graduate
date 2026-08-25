//! Deterministic restack inventory contracts and history classification.

use std::collections::{BTreeMap, BTreeSet};

use thiserror::Error;

/// One commit needed to classify an environment history.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GraphCommit {
    pub id: String,
    pub tree: String,
    pub parents: Vec<String>,
    pub message: String,
}

/// One remote feature ref and the commits reachable from its captured tip.
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
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HistoricalMerge {
    pub commit: String,
    pub first_parent: String,
    pub feature_parent: String,
    pub tree: String,
}

/// A surviving explicit feature, ordered by its first merge into the environment.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExplicitFeature {
    pub name: String,
    pub tip: String,
    pub historical_merges: Vec<HistoricalMerge>,
}

/// An exact obsolete phase marker that a v1 restack deliberately drops.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DroppedMarker {
    pub commit: String,
    pub parent: String,
    pub tree: String,
}

/// One environment-unique non-merge commit and its explicit feature owners.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AttributedCommit {
    pub commit: String,
    pub branches: Vec<String>,
}

/// A complete, ordered proof that an environment can be reconstructed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RestackSnapshot {
    pub remote: String,
    pub environment: String,
    pub environment_ref: String,
    pub environment_tip: String,
    pub main: String,
    pub main_ref: String,
    pub main_tip: String,
    pub features: Vec<ExplicitFeature>,
    pub dropped_markers: Vec<DroppedMarker>,
    pub attributed_commits: Vec<AttributedCommit>,
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
        .filter(|feature| {
            graph.environment_ancestors.contains(&feature.tip)
                && !graph.main_ancestors.contains(&feature.tip)
        })
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

    Ok(RestackSnapshot {
        remote: graph.remote.clone(),
        environment: graph.environment.clone(),
        environment_ref: graph.environment_ref.clone(),
        environment_tip: graph.environment_tip.clone(),
        main: graph.main.clone(),
        main_ref: graph.main_ref.clone(),
        main_tip: graph.main_tip.clone(),
        features,
        dropped_markers,
        attributed_commits,
    })
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
