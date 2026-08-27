use super::*;

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
