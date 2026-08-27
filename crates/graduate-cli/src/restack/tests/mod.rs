//! Shared fixtures and helpers.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::rc::Rc;

use graduate::restack::{build_inventory_snapshot, OrphanedCommit, RestackSnapshot};
use ratatui::backend::TestBackend;
use ratatui::Terminal;

use super::*;

mod interactive_tests;
mod inventory_fallback_tests;
mod plan_json_tests;

/// Reachability snapshot where feature/b carries feature/a and `stray` is dropped.
fn reachability_plan_inputs() -> (RestackSnapshot, Vec<OrphanedCommit>) {
    use graduate::restack::{
        FeatureRef, GraphCommit, InventoryError, RestackGraph, UnsupportedHistory,
    };
    let ids =
        |values: &[&str]| -> BTreeSet<String> { values.iter().map(ToString::to_string).collect() };
    let mut commits = BTreeMap::new();
    for (id, parents) in [
        ("base", vec![]),
        ("a", vec!["base"]),
        ("b", vec!["a"]),
        ("stray", vec!["b"]),
    ] {
        commits.insert(
            id.to_owned(),
            GraphCommit {
                id: id.to_owned(),
                tree: format!("tree-{id}"),
                parents: parents.into_iter().map(str::to_owned).collect(),
                message: id.to_owned(),
            },
        );
    }
    let graph = RestackGraph {
        remote: "origin".to_owned(),
        environment: "qa".to_owned(),
        environment_ref: "refs/remotes/origin/qa".to_owned(),
        environment_tip: "stray".to_owned(),
        main: "main".to_owned(),
        main_ref: "refs/remotes/origin/main".to_owned(),
        main_tip: "base".to_owned(),
        environment_ancestors: ids(&["base", "a", "b", "stray"]),
        main_ancestors: ids(&["base"]),
        feature_refs: vec![
            FeatureRef {
                name: "feature/a".to_owned(),
                tip: "a".to_owned(),
                ancestors: ids(&["a"]),
            },
            FeatureRef {
                name: "feature/b".to_owned(),
                tip: "b".to_owned(),
                ancestors: ids(&["a", "b"]),
            },
        ],
        commits,
    };
    let reason = UnsupportedHistory::from(InventoryError::DirectCommit {
        commit: "stray".to_owned(),
    });
    let snapshot = build_inventory_snapshot(&graph, reason, &BTreeMap::new());
    let orphans = vec![OrphanedCommit {
        commit: "stray".to_owned(),
        subject: "stray".to_owned(),
        author: "Pat".to_owned(),
        date: "2026-01-02".to_owned(),
    }];
    (snapshot, orphans)
}
