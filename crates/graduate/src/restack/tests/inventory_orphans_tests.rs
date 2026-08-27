use super::*;

#[test]
fn every_inventory_error_becomes_fallback_evidence() {
    let cases = [
        (
            InventoryError::MissingCommit {
                commit: "m".to_owned(),
            },
            ("missingCommit", Some("m"), None, vec![], None),
        ),
        (
            InventoryError::DirectCommit {
                commit: "d".to_owned(),
            },
            ("directCommit", Some("d"), None, vec![], None),
        ),
        (
            InventoryError::FastForwardHistory {
                commit: "f".to_owned(),
                branches: vec!["a".to_owned()],
            },
            ("fastForwardHistory", Some("f"), None, vec!["a"], None),
        ),
        (
            InventoryError::OctopusMerge {
                merge_commit: "o".to_owned(),
                parents: 3,
            },
            ("octopusMerge", Some("o"), None, vec![], Some(3)),
        ),
        (
            InventoryError::DeletedFeatureRef {
                merge_commit: "x".to_owned(),
                feature_parent: "p".to_owned(),
            },
            ("deletedFeatureRef", Some("x"), Some("p"), vec![], None),
        ),
        (
            InventoryError::AmbiguousFeatureRefs {
                merge_commit: "y".to_owned(),
                feature_parent: "q".to_owned(),
                branches: vec!["a".to_owned(), "b".to_owned()],
            },
            (
                "ambiguousFeatureRefs",
                Some("y"),
                Some("q"),
                vec!["a", "b"],
                None,
            ),
        ),
    ];
    for (error, (kind, commit, feature_parent, branches, parents)) in cases {
        let evidence = UnsupportedHistory::from(error);
        assert_eq!(evidence.kind, kind);
        assert_eq!(evidence.commit.as_deref(), commit);
        assert_eq!(evidence.feature_parent.as_deref(), feature_parent);
        assert_eq!(evidence.branches, branches);
        assert_eq!(evidence.parents, parents);
    }
}

#[test]
fn inventory_snapshot_records_work_no_top_level_feature_reaches() {
    let snapshot = build_inventory_snapshot(&ambiguous_graph(), direct_reason(), &BTreeMap::new());
    assert_eq!(snapshot.unattributed_commits, ["stray"]);
}

#[test]
fn orphans_follow_the_retained_set_and_never_include_merges_or_markers(
) -> Result<(), Box<dyn std::error::Error>> {
    let snapshot = build_inventory_snapshot(&ambiguous_graph(), direct_reason(), &BTreeMap::new());
    let everything = select_features(&snapshot, &[])?;
    assert_eq!(
        orphaned_commit_ids(&snapshot, &everything.retained),
        ["stray"]
    );
    let nothing = select_features(&snapshot, &["feature/b".to_owned()])?;
    assert_eq!(
        orphaned_commit_ids(&snapshot, &nothing.retained),
        ["a1", "a1b", "b1", "stray"]
    );
    assert_eq!(
        select_features(&snapshot, &["feature/a".to_owned()]),
        Err(SelectionError::IndirectOnly {
            branch: "feature/a".to_owned()
        })
    );
    let history = build_snapshot(&simple_graph())?;
    assert!(orphaned_commit_ids(&history, &[]).is_empty());
    Ok(())
}

#[test]
fn plan_requires_the_exact_orphan_rows_and_binds_mode_and_orphans_into_the_digest(
) -> Result<(), Box<dyn std::error::Error>> {
    let snapshot = build_inventory_snapshot(&ambiguous_graph(), direct_reason(), &BTreeMap::new());
    let selection = select_features(&snapshot, &[])?;
    let reconstruction = || Reconstruction {
        merges: vec![MergeOutcome {
            branch: "feature/b".to_owned(),
            tip: "b1".to_owned(),
            commit: "preview".to_owned(),
            tree: "tree".to_owned(),
            resolution: MergeResolution::Clean,
        }],
        final_tree: "tree".to_owned(),
        preview_commit: "preview".to_owned(),
    };
    let author = RestackAuthor {
        name: "Pat".to_owned(),
        email: "pat@example.com".to_owned(),
    };
    let endpoints = RemoteEndpointIdentity {
        fetch_sha256: "f".repeat(64),
        push_sha256: "p".repeat(64),
    };
    let orphan = |commit: &str| OrphanedCommit {
        commit: commit.to_owned(),
        subject: "s".to_owned(),
        author: "Pat".to_owned(),
        date: "2026-01-01".to_owned(),
    };

    assert_eq!(
        build_plan(
            snapshot.clone(),
            endpoints.clone(),
            author.clone(),
            selection.clone(),
            reconstruction(),
            Vec::new(),
        )
        .map(|plan| plan.digest),
        Err(PlanError::OrphanedCommits {
            expected: 1,
            actual: 0,
            mismatch: "stray".to_owned(),
        })
    );
    let plan = build_plan(
        snapshot.clone(),
        endpoints.clone(),
        author.clone(),
        selection.clone(),
        reconstruction(),
        vec![orphan("stray")],
    )?;

    let mut as_history = snapshot.clone();
    as_history.inventory_mode = InventoryMode::History;
    let history_plan = build_plan(
        as_history,
        endpoints.clone(),
        author.clone(),
        selection.clone(),
        reconstruction(),
        Vec::new(),
    )?;
    assert_ne!(plan.digest, history_plan.digest);

    let mut extra_orphan = snapshot;
    extra_orphan
        .unattributed_commits
        .push("stray-two".to_owned());
    let more_orphans = build_plan(
        extra_orphan,
        endpoints,
        author,
        selection,
        reconstruction(),
        vec![orphan("stray"), orphan("stray-two")],
    )?;
    assert_ne!(plan.digest, more_orphans.digest);
    Ok(())
}
