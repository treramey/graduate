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
    assert_eq!(
        snapshot
            .graduated_features
            .iter()
            .map(|feature| feature.name.as_str())
            .collect::<Vec<_>>(),
        ["feature/graduated"]
    );
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
fn snapshot_captures_the_current_tip_of_an_advanced_explicit_feature(
) -> Result<(), Box<dyn std::error::Error>> {
    let mut graph = graph_with_merge(vec![feature(
        "feature/advanced",
        "advanced",
        &["advanced", "feature", "base"],
    )]);
    add_commit(&mut graph, "advanced", "ta", &["feature"], "advanced");

    let snapshot = build_snapshot(&graph)?;

    assert_eq!(snapshot.features.len(), 1);
    assert_eq!(snapshot.features[0].name, "feature/advanced");
    assert_eq!(snapshot.features[0].tip, "advanced");
    Ok(())
}

#[test]
fn removal_selection_rejects_duplicate_unknown_graduated_and_indirect_names() {
    let snapshot = planning_snapshot();

    assert_eq!(
        select_features(&snapshot, &["feature/a".to_owned(), "feature/a".to_owned()]),
        Err(SelectionError::Duplicate {
            branch: "feature/a".to_owned()
        })
    );
    assert_eq!(
        select_features(&snapshot, &["feature/graduated".to_owned()]),
        Err(SelectionError::Graduated {
            branch: "feature/graduated".to_owned()
        })
    );
    assert_eq!(
        select_features(&snapshot, &["feature/indirect".to_owned()]),
        Err(SelectionError::IndirectOnly {
            branch: "feature/indirect".to_owned()
        })
    );
    assert_eq!(
        select_features(&snapshot, &["feature/missing".to_owned()]),
        Err(SelectionError::Unknown {
            branch: "feature/missing".to_owned()
        })
    );
}

#[test]
fn removal_selection_reports_retained_branches_that_keep_feature_work() {
    let snapshot = planning_snapshot();

    assert_eq!(
        select_features(&snapshot, &["feature/a".to_owned()]),
        Err(SelectionError::RetainedDependency {
            branch: "feature/a".to_owned(),
            dependents: vec!["feature/b".to_owned()]
        })
    );
    let selection = select_features(&snapshot, &["feature/b".to_owned(), "feature/a".to_owned()]);
    assert!(
        matches!(selection, Ok(selection) if selection.retained.is_empty()
        && selection.removed.iter().map(|feature| feature.name.as_str()).collect::<Vec<_>>()
            == ["feature/a", "feature/b"])
    );
}

#[test]
fn plan_digest_binds_inputs_identity_selection_and_tree_but_not_preview_commit(
) -> Result<(), Box<dyn std::error::Error>> {
    let mut snapshot = planning_snapshot();
    snapshot.attributed_commits.clear();
    let selection = select_features(&snapshot, &["feature/b".to_owned()])?;
    let author = RestackAuthor {
        name: "Test Author".to_owned(),
        email: "test@example.com".to_owned(),
    };
    let endpoints = RemoteEndpointIdentity {
        fetch_sha256: "11".repeat(32),
        push_sha256: "22".repeat(32),
    };
    let outcomes = vec![MergeOutcome {
        branch: "feature/a".to_owned(),
        tip: "a".to_owned(),
        commit: "preview-one".to_owned(),
        tree: "tree-a".to_owned(),
        resolution: MergeResolution::Clean,
    }];

    let first = build_plan(
        snapshot.clone(),
        endpoints.clone(),
        author.clone(),
        selection.clone(),
        Reconstruction {
            merges: outcomes.clone(),
            final_tree: "final-tree".to_owned(),
            preview_commit: "preview-one".to_owned(),
        },
        Vec::new(),
    )?;
    let mut regenerated = outcomes;
    regenerated[0].commit = "preview-two".to_owned();
    let second = build_plan(
        snapshot.clone(),
        endpoints.clone(),
        author.clone(),
        selection.clone(),
        Reconstruction {
            merges: regenerated.clone(),
            final_tree: "final-tree".to_owned(),
            preview_commit: "preview-two".to_owned(),
        },
        Vec::new(),
    )?;
    let changed_author = build_plan(
        snapshot.clone(),
        endpoints.clone(),
        RestackAuthor {
            name: "Other Author".to_owned(),
            email: author.email.clone(),
        },
        selection.clone(),
        Reconstruction {
            merges: regenerated,
            final_tree: "final-tree".to_owned(),
            preview_commit: "preview-two".to_owned(),
        },
        Vec::new(),
    )?;
    let changed_tree = build_plan(
        snapshot.clone(),
        endpoints.clone(),
        author.clone(),
        selection.clone(),
        Reconstruction {
            merges: vec![MergeOutcome {
                branch: "feature/a".to_owned(),
                tip: "a".to_owned(),
                commit: "preview-three".to_owned(),
                tree: "tree-a".to_owned(),
                resolution: MergeResolution::Clean,
            }],
            final_tree: "other-tree".to_owned(),
            preview_commit: "preview-three".to_owned(),
        },
        Vec::new(),
    )?;
    let changed_endpoint = build_plan(
        snapshot,
        RemoteEndpointIdentity {
            fetch_sha256: endpoints.fetch_sha256,
            push_sha256: "33".repeat(32),
        },
        author,
        selection,
        Reconstruction {
            merges: vec![MergeOutcome {
                branch: "feature/a".to_owned(),
                tip: "a".to_owned(),
                commit: "preview-four".to_owned(),
                tree: "tree-a".to_owned(),
                resolution: MergeResolution::Clean,
            }],
            final_tree: "final-tree".to_owned(),
            preview_commit: "preview-four".to_owned(),
        },
        Vec::new(),
    )?;

    assert_eq!(first.digest, second.digest);
    assert_eq!(
        first.digest,
        "9077fbf6b660331cea97f2be2209570096b124619a0256774ddb6833c3891e4c"
    );
    assert_ne!(first.digest, changed_author.digest);
    assert_ne!(first.digest, changed_tree.digest);
    assert_ne!(first.digest, changed_endpoint.digest);
    assert_eq!(first.digest.len(), 64);
    Ok(())
}
