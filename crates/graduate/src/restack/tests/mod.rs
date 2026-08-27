//! Shared fixtures and helpers.

use super::errors::*;
use super::interaction::*;
use super::inventory::*;
use super::plan::*;
use super::snapshot::*;
use super::*;

mod interaction_tests;
mod inventory_orphans_tests;
mod inventory_tests;
mod snapshot_tests;
mod unsupported_history_tests;

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
