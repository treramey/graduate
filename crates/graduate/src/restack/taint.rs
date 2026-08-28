//! Detection of feature branches that absorbed environment merges.

use std::collections::BTreeSet;

use super::{FeatureRef, RestackGraph, TaintedFeature};

/// Merge commits on the environment's own first-parent line whose second
/// parent is not on main, keyed by id.
///
/// These are the merges that promoted features into the environment. A
/// merge that only pulled main into the environment brings nothing a
/// feature branch could wrongly absorb, so it is skipped. The walk stops at
/// the first commit on main or the first commit the graph does not hold.
#[must_use]
pub(super) fn environment_first_parent_merges(graph: &RestackGraph) -> BTreeSet<String> {
    let mut merges = BTreeSet::new();
    let mut visited = BTreeSet::new();
    let mut current = graph.environment_tip.clone();
    while !graph.main_ancestors.contains(&current) && visited.insert(current.clone()) {
        let Some(commit) = graph.commits.get(&current) else {
            break;
        };
        if let [_, second, ..] = commit.parents.as_slice() {
            if !graph.main_ancestors.contains(second) {
                merges.insert(commit.id.clone());
            }
        }
        let Some(parent) = commit.parents.first() else {
            break;
        };
        current.clone_from(parent);
    }
    merges
}

/// Environment merges the feature reaches without owning them, sorted.
///
/// A merge on the feature's own first-parent line is the feature's own work
/// (the environment may have been fast-forwarded onto the branch), and a
/// merge whose second parent is on that line promoted this feature itself.
/// Every other environment merge the tip reaches was pulled in by merging
/// the environment into the branch. The rule is the same in history and
/// inventory mode.
#[must_use]
pub(super) fn absorbed_merges(
    graph: &RestackGraph,
    feature: &FeatureRef,
    environment_merges: &BTreeSet<String>,
) -> BTreeSet<String> {
    let reached = environment_merges
        .iter()
        .filter(|merge| feature.ancestors.contains(*merge))
        .collect::<Vec<_>>();
    if reached.is_empty() {
        return BTreeSet::new();
    }
    let own_line = first_parent_line(graph, feature);
    reached
        .into_iter()
        .filter(|merge| !own_line.contains(*merge))
        .filter(|merge| {
            let promoted_this_feature = graph
                .commits
                .get(*merge)
                .and_then(|commit| commit.parents.get(1))
                .is_some_and(|second| own_line.contains(second));
            !promoted_this_feature
        })
        .cloned()
        .collect()
}

fn first_parent_line(graph: &RestackGraph, feature: &FeatureRef) -> BTreeSet<String> {
    let mut line = BTreeSet::new();
    let mut current = feature.tip.clone();
    while feature.ancestors.contains(&current) && line.insert(current.clone()) {
        let Some(parent) = graph
            .commits
            .get(&current)
            .and_then(|commit| commit.parents.first())
        else {
            break;
        };
        current.clone_from(parent);
    }
    line
}

/// Environment-only commits the feature reaches without passing through an
/// absorbed environment merge: the work the branch can claim as its own.
///
/// A commit reachable only through an absorbed merge belongs to whichever
/// feature the environment merged, not to the branch that later merged the
/// environment into itself. A feature with no absorbed merges owns every
/// ancestor.
#[must_use]
pub(super) fn own_ancestors(
    graph: &RestackGraph,
    feature: &FeatureRef,
    absorbed: &BTreeSet<String>,
) -> BTreeSet<String> {
    if absorbed.is_empty() {
        return feature.ancestors.clone();
    }
    let mut own = BTreeSet::new();
    let mut pending = vec![feature.tip.clone()];
    while let Some(id) = pending.pop() {
        if !feature.ancestors.contains(&id) || absorbed.contains(&id) || !own.insert(id.clone()) {
            continue;
        }
        if let Some(commit) = graph.commits.get(&id) {
            pending.extend(commit.parents.iter().cloned());
        }
    }
    own
}

/// Build the sorted tainted list from per-feature absorbed merges.
#[must_use]
pub(super) fn tainted_features<'a, I>(features: I) -> Vec<TaintedFeature>
where
    I: IntoIterator<Item = (&'a FeatureRef, BTreeSet<String>)>,
{
    let mut tainted = features
        .into_iter()
        .filter(|(_, absorbed)| !absorbed.is_empty())
        .map(|(feature, absorbed_merges)| TaintedFeature {
            name: feature.name.clone(),
            tip: feature.tip.clone(),
            absorbed_merges: absorbed_merges.into_iter().collect(),
        })
        .collect::<Vec<_>>();
    tainted.sort_by(|left, right| left.name.cmp(&right.name));
    tainted
}
