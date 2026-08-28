use super::*;

/// `feature/a` was promoted into `qa`; `feature/b` then merged `qa` into
/// itself before being promoted, so `b` reaches `a`'s promotion merge.
fn tainted_graph() -> RestackGraph {
    let mut graph = empty_graph("envm2");
    add_commit(&mut graph, "base", "tb", &[], "base");
    add_commit(&mut graph, "a1", "ta1", &["base"], "a one");
    add_commit(&mut graph, "envm1", "te1", &["base", "a1"], "promote a");
    add_commit(&mut graph, "b1", "tb1", &["base"], "b one");
    add_commit(&mut graph, "b2", "tb2", &["b1", "envm1"], "sync qa");
    add_commit(&mut graph, "envm2", "te2", &["envm1", "b2"], "promote b");
    graph.environment_ancestors = ids(&["base", "a1", "envm1", "b1", "b2", "envm2"]);
    graph.main_ancestors = ids(&["base"]);
    graph.feature_refs = vec![
        feature("feature/a", "a1", &["a1"]),
        feature("feature/b", "b2", &["a1", "envm1", "b1", "b2"]),
    ];
    graph
}

#[test]
fn history_snapshot_lists_features_that_absorbed_environment_merges(
) -> Result<(), Box<dyn std::error::Error>> {
    let snapshot = build_snapshot(&tainted_graph())?;
    assert_eq!(names(&snapshot.features), ["feature/a", "feature/b"]);
    assert_eq!(
        snapshot.tainted_features,
        vec![TaintedFeature {
            name: "feature/b".to_owned(),
            tip: "b2".to_owned(),
            absorbed_merges: vec!["envm1".to_owned()],
        }]
    );
    // `a one` belongs to `feature/a` alone; `feature/b` only reaches it
    // through the absorbed merge.
    let a_one = snapshot
        .attributed_commits
        .iter()
        .find(|commit| commit.commit == "a1")
        .ok_or("a1 not attributed")?;
    assert_eq!(a_one.branches, ["feature/a"]);
    Ok(())
}

#[test]
fn tainted_features_start_removed_and_cannot_be_retained() -> Result<(), Box<dyn std::error::Error>>
{
    let snapshot = build_snapshot(&tainted_graph())?;
    let interaction = RestackInteraction::new(snapshot.clone());
    assert!(interaction.is_retained(0));
    assert!(!interaction.is_retained(1));
    assert_eq!(interaction.tainted_features().len(), 1);
    assert!(interaction.tainted_feature(1).is_some());
    assert!(interaction.tainted_feature(0).is_none());

    assert_eq!(
        select_features(&snapshot, &[]),
        Err(SelectionError::Tainted {
            branch: "feature/b".to_owned(),
        })
    );
    let selection = select_features(&snapshot, &["feature/b".to_owned()])?;
    assert_eq!(selection.retained[0].name, "feature/a");
    assert_eq!(selection.removed[0].name, "feature/b");
    Ok(())
}

#[test]
fn toggling_a_tainted_feature_back_on_is_rejected() -> Result<(), Box<dyn std::error::Error>> {
    let mut interaction = RestackInteraction::new(build_snapshot(&tainted_graph())?);
    interaction.update(RestackInteractionAction::MoveDown);
    assert_eq!(interaction.cursor(), 1);
    assert_eq!(
        interaction.update(RestackInteractionAction::Toggle),
        RestackInteractionEffect::Rejected(SelectionError::Tainted {
            branch: "feature/b".to_owned(),
        })
    );
    assert!(!interaction.is_retained(1));
    interaction.update(RestackInteractionAction::KeepAll);
    assert!(interaction.is_retained(0));
    assert!(!interaction.is_retained(1));
    Ok(())
}

#[test]
fn inventory_snapshot_taints_without_carrying_through_absorbed_merges() {
    let graph = tainted_graph();
    let snapshot = build_inventory_snapshot(&graph, direct_reason(), &BTreeMap::new());
    assert_eq!(names(&snapshot.features), ["feature/a", "feature/b"]);
    assert!(snapshot.carried_features.is_empty());
    assert_eq!(
        snapshot.tainted_features,
        vec![TaintedFeature {
            name: "feature/b".to_owned(),
            tip: "b2".to_owned(),
            absorbed_merges: vec!["envm1".to_owned()],
        }]
    );
    let a_one = snapshot
        .attributed_commits
        .iter()
        .find(|commit| commit.commit == "a1");
    assert_eq!(
        a_one.map(|commit| commit.branches.clone()),
        Some(vec!["feature/a".to_owned()])
    );
}

#[test]
fn environment_fast_forwarded_onto_a_feature_does_not_taint_it() {
    // `ambiguous_graph` puts feature/a's own merge `a2` on the environment's
    // first-parent line.
    let snapshot = build_inventory_snapshot(&ambiguous_graph(), direct_reason(), &BTreeMap::new());
    assert!(snapshot.tainted_features.is_empty());
}

/// `feature/a` was promoted, then merged `qa` back while `qa` held nothing
/// but `a` itself: the only environment merge it reaches promoted `a`.
fn self_synced_graph() -> RestackGraph {
    let mut graph = empty_graph("envm1");
    add_commit(&mut graph, "base", "tb", &[], "base");
    add_commit(&mut graph, "a1", "ta1", &["base"], "a one");
    add_commit(&mut graph, "envm1", "te1", &["base", "a1"], "promote a");
    add_commit(&mut graph, "a2", "ta2", &["a1", "envm1"], "sync qa");
    graph.environment_ancestors = ids(&["base", "a1", "envm1"]);
    graph.main_ancestors = ids(&["base"]);
    graph.feature_refs = vec![feature("feature/a", "a2", &["a1", "envm1", "a2"])];
    graph
}

#[test]
fn merging_only_your_own_promotion_back_is_not_tainted_in_either_mode(
) -> Result<(), Box<dyn std::error::Error>> {
    let graph = self_synced_graph();
    let history = build_snapshot(&graph);
    // The tip is not in the environment, so history mode reports nothing
    // for it either way; inventory mode must agree once the tip is merged.
    assert!(history.map_or(true, |snapshot| snapshot.tainted_features.is_empty()));

    let mut graph = graph;
    add_commit(
        &mut graph,
        "envm2",
        "te2",
        &["envm1", "a2"],
        "promote a again",
    );
    graph.environment_tip = "envm2".to_owned();
    graph.environment_ancestors = ids(&["base", "a1", "envm1", "a2", "envm2"]);
    let history = build_snapshot(&graph)?;
    assert!(history.tainted_features.is_empty());
    let inventory = build_inventory_snapshot(&graph, direct_reason(), &BTreeMap::new());
    assert!(inventory.tainted_features.is_empty());
    Ok(())
}
