//! Reachability inventory fallback snapshot.

use std::collections::{BTreeMap, BTreeSet};

use super::snapshot::{branch_identity, is_dropped_marker};
use super::taint::{
    absorbed_merges, environment_first_parent_merges, own_ancestors, tainted_features,
};
use super::{
    AttributedCommit, BranchIdentity, CarriedFeature, DroppedMarker, ExplicitFeature, FeatureRef,
    InventoryMode, RestackGraph, RestackSnapshot, UnsupportedHistory,
};

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
    // A branch that merged the environment reaches every promoted feature,
    // but it neither carries nor owns that work.
    let environment_merges = environment_first_parent_merges(graph);
    let absorbed: BTreeMap<&str, BTreeSet<String>> = candidates
        .iter()
        .map(|feature| {
            (
                feature.name.as_str(),
                absorbed_merges(graph, feature, &environment_merges),
            )
        })
        .collect();
    let owned: BTreeMap<&str, BTreeSet<String>> = candidates
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
    let owns = |feature: &FeatureRef, commit: &str| {
        owned
            .get(feature.name.as_str())
            .is_some_and(|own| own.contains(commit))
    };
    let carriers_of = |feature: &FeatureRef| -> Vec<String> {
        candidates
            .iter()
            .filter(|other| other.name != feature.name)
            .filter(|other| owns(other, &feature.tip))
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
            .filter(|feature| owns(feature, id))
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
        tainted_features: tainted_features(top_level.iter().map(|feature| {
            (
                *feature,
                absorbed
                    .get(feature.name.as_str())
                    .cloned()
                    .unwrap_or_default(),
            )
        })),
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
