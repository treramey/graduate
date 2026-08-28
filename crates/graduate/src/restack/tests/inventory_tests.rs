use super::*;

#[test]
fn history_snapshot_reports_history_mode_without_fallback_evidence(
) -> Result<(), Box<dyn std::error::Error>> {
    let snapshot = build_snapshot(&simple_graph())?;
    assert_eq!(snapshot.inventory_mode, InventoryMode::History);
    assert_eq!(snapshot.unsupported_history, None);
    assert!(snapshot.carried_features.is_empty());
    Ok(())
}

#[test]
fn plan_and_snapshot_round_trip_every_inventory_field() -> Result<(), Box<dyn std::error::Error>> {
    assert_eq!(RESTACK_SCHEMA_VERSION, 3);
    let mut snapshot = build_snapshot(&simple_graph())?;
    snapshot.inventory_mode = InventoryMode::Reachability;
    snapshot.unsupported_history = Some(UnsupportedHistory {
        kind: "ambiguousFeatureRefs".to_owned(),
        commit: Some("merge".to_owned()),
        feature_parent: Some("feature".to_owned()),
        branches: vec!["feature/a".to_owned(), "feature/b".to_owned()],
        parents: None,
    });
    snapshot.carried_features = vec![CarriedFeature {
        name: "feature/b".to_owned(),
        tip: "feature".to_owned(),
        carriers: vec!["feature/a".to_owned()],
    }];
    snapshot.unattributed_commits = vec!["lost".to_owned()];
    let encoded = serde_json::to_string(&snapshot)?;
    let decoded: RestackSnapshot = serde_json::from_str(&encoded)?;
    assert_eq!(decoded, snapshot);

    let orphan = OrphanedCommit {
        commit: "lost".to_owned(),
        subject: "lost work".to_owned(),
        author: "Pat".to_owned(),
        date: "2026-01-02".to_owned(),
    };
    let encoded = serde_json::to_string(&orphan)?;
    assert_eq!(
        encoded,
        r#"{"commit":"lost","subject":"lost work","author":"Pat","date":"2026-01-02"}"#
    );
    let selection = select_features(&snapshot, &[])?;
    let plan = build_plan(
        snapshot.clone(),
        RemoteEndpointIdentity {
            fetch_sha256: "f".repeat(64),
            push_sha256: "p".repeat(64),
        },
        RestackAuthor {
            name: "Pat".to_owned(),
            email: "pat@example.com".to_owned(),
        },
        selection.clone(),
        Reconstruction {
            merges: vec![MergeOutcome {
                branch: "feature/a".to_owned(),
                tip: "feature".to_owned(),
                commit: "preview".to_owned(),
                tree: "tree".to_owned(),
                resolution: MergeResolution::Clean,
            }],
            final_tree: "tree".to_owned(),
            preview_commit: "preview".to_owned(),
        },
        vec![orphan.clone()],
    )?;
    assert_eq!(plan.orphaned_commits, vec![orphan]);
    Ok(())
}

#[test]
fn inventory_snapshot_splits_top_level_and_carried_and_attributes_reached_work(
) -> Result<(), Box<dyn std::error::Error>> {
    let graph = ambiguous_graph();
    let reason = UnsupportedHistory::from(InventoryError::AmbiguousFeatureRefs {
        merge_commit: "a2".to_owned(),
        feature_parent: "a1b".to_owned(),
        branches: vec!["feature/a".to_owned(), "feature/b".to_owned()],
    });

    let snapshot = build_inventory_snapshot(&graph, reason.clone(), &BTreeMap::new());

    assert_eq!(snapshot.inventory_mode, InventoryMode::Reachability);
    assert_eq!(snapshot.unsupported_history, Some(reason));
    assert_eq!(names(&snapshot.features), ["feature/b"]);
    assert!(snapshot.features[0].historical_merges.is_empty());
    assert_eq!(
        snapshot.carried_features,
        vec![CarriedFeature {
            name: "feature/a".to_owned(),
            tip: "a2".to_owned(),
            carriers: vec!["feature/b".to_owned()],
        }]
    );
    assert_eq!(
        snapshot.indirect_features,
        vec![BranchIdentity {
            name: "feature/a".to_owned(),
            tip: "a2".to_owned(),
        }]
    );
    assert_eq!(
        snapshot.graduated_features,
        vec![BranchIdentity {
            name: "feature/gone".to_owned(),
            tip: "base".to_owned(),
        }]
    );
    assert_eq!(snapshot.dropped_markers.len(), 1);
    assert_eq!(snapshot.dropped_markers[0].commit, "marker");
    let attributed = snapshot
        .attributed_commits
        .iter()
        .map(|commit| commit.commit.as_str())
        .collect::<Vec<_>>();
    assert_eq!(attributed, ["a1", "a1b", "b1"]);
    assert!(snapshot
        .attributed_commits
        .iter()
        .all(|commit| commit.branches == ["feature/b"]));
    Ok(())
}

#[test]
fn inventory_chain_keeps_only_the_outermost_tip_as_the_single_carrier() {
    let mut graph = empty_graph("c");
    for id in ["base", "a", "b", "c"] {
        add_commit(&mut graph, id, id, &[], id);
    }
    graph.environment_ancestors = ids(&["base", "a", "b", "c"]);
    graph.main_ancestors = ids(&["base"]);
    graph.feature_refs = vec![
        feature("feature/a", "a", &["a"]),
        feature("feature/b", "b", &["a", "b"]),
        feature("feature/c", "c", &["a", "b", "c"]),
    ];
    let snapshot = build_inventory_snapshot(&graph, direct_reason(), &BTreeMap::new());
    assert_eq!(names(&snapshot.features), ["feature/c"]);
    assert_eq!(
        snapshot
            .carried_features
            .iter()
            .map(|carried| (carried.name.as_str(), carried.carriers.clone()))
            .collect::<Vec<_>>(),
        [
            ("feature/a", vec!["feature/c".to_owned()]),
            ("feature/b", vec!["feature/c".to_owned()]),
        ]
    );
}

#[test]
fn inventory_diamond_keeps_both_carriers_and_orders_by_tip_age_then_name() {
    let mut graph = empty_graph("env");
    for id in ["base", "x", "left", "right", "same", "undated"] {
        add_commit(&mut graph, id, id, &[], id);
    }
    graph.environment_ancestors = ids(&["base", "x", "left", "right", "same", "undated"]);
    graph.main_ancestors = ids(&["base"]);
    graph.feature_refs = vec![
        feature("feature/right", "right", &["x", "right"]),
        feature("feature/left", "left", &["x", "left"]),
        feature("feature/x", "x", &["x"]),
        feature("feature/zzz-same", "same", &["same"]),
        feature("feature/aaa-same", "same", &["same"]),
        feature("feature/undated", "undated", &["undated"]),
    ];
    let timestamps = BTreeMap::from([
        ("left".to_owned(), 200),
        ("right".to_owned(), 100),
        ("same".to_owned(), 100),
    ]);
    let snapshot = build_inventory_snapshot(&graph, direct_reason(), &timestamps);
    assert_eq!(
        names(&snapshot.features),
        [
            "feature/aaa-same",
            "feature/right",
            "feature/left",
            "feature/undated"
        ]
    );
    assert_eq!(
        snapshot
            .carried_features
            .iter()
            .map(|carried| (carried.name.as_str(), carried.carriers.clone()))
            .collect::<Vec<_>>(),
        [
            (
                "feature/x",
                vec!["feature/right".to_owned(), "feature/left".to_owned()]
            ),
            ("feature/zzz-same", vec!["feature/aaa-same".to_owned()]),
        ]
    );
}
