use super::super::handoff::success_text;
use super::super::keys::filtered_feature_indices;
use super::*;

#[test]
fn inventory_checklist_shows_the_banner_carried_rows_and_drop_count(
) -> Result<(), Box<dyn std::error::Error>> {
    let mut interaction = RestackInteraction::from_inventory(inventory_snapshot(
        UnsupportedHistory::from(InventoryError::DirectCommit {
            commit: "stray".to_owned(),
        }),
    ));
    let _ = interaction.update(RestackInteractionAction::AcceptInventoryFallback);

    let narrow = rendered_at(&interaction, None, None, 60, 24)?;
    assert!(narrow.contains("Inventory mode · no rerere"));
    assert!(narrow.contains("1 commit will be dropped"));
    assert!(narrow.contains("feature/two"));
    assert!(narrow.contains("↳ carried  feature/PROJ-12-one"));
    assert!(narrow.contains("◆ dependency · ↳ carried"), "{narrow}");

    let wide = rendered_at(&interaction, None, None, 120, 30)?;
    assert!(
        wide.contains("Inventory mode: reachability · oldest tip first · no reused resolutions")
    );

    let _ = interaction.update(RestackInteractionAction::RemoveAll);
    let removed = rendered_at(&interaction, None, None, 60, 24)?;
    assert!(removed.contains("3 commits will be dropped"));
    assert!(removed.contains("0 retained · 1 removed"));

    let history = RestackInteraction::new(snapshot());
    let plain = rendered_at(&history, None, None, 60, 24)?;
    assert!(!plain.contains("Inventory mode"));
    assert!(!plain.contains("dropped"));
    assert!(!plain.contains("carried"));
    Ok(())
}

#[test]
fn inventory_checklist_lists_every_carried_branch_under_its_top_level_merge(
) -> Result<(), Box<dyn std::error::Error>> {
    let ids = |values: &[&str]| -> std::collections::BTreeSet<String> {
        values.iter().map(ToString::to_string).collect()
    };
    let mut commits = std::collections::BTreeMap::new();
    for (id, parents) in [
        ("base", vec![]),
        ("a", vec!["base"]),
        ("b", vec!["a"]),
        ("c", vec!["b"]),
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
        environment_tip: "c".to_owned(),
        main: "main".to_owned(),
        main_ref: "refs/remotes/origin/main".to_owned(),
        main_tip: "base".to_owned(),
        environment_ancestors: ids(&["base", "a", "b", "c"]),
        main_ancestors: ids(&["base"]),
        feature_refs: vec![
            FeatureRef {
                name: "feature/inner".to_owned(),
                tip: "a".to_owned(),
                ancestors: ids(&["a"]),
            },
            FeatureRef {
                name: "feature/middle".to_owned(),
                tip: "b".to_owned(),
                ancestors: ids(&["a", "b"]),
            },
            FeatureRef {
                name: "feature/outer".to_owned(),
                tip: "c".to_owned(),
                ancestors: ids(&["a", "b", "c"]),
            },
        ],
        commits,
    };
    let snapshot = build_inventory_snapshot(
        &graph,
        UnsupportedHistory::from(InventoryError::DirectCommit {
            commit: "c".to_owned(),
        }),
        &std::collections::BTreeMap::new(),
    );
    let mut interaction = RestackInteraction::from_inventory(snapshot);
    let _ = interaction.update(RestackInteractionAction::AcceptInventoryFallback);

    let rendered = rendered_at(&interaction, None, None, 100, 30)?;
    assert!(rendered.contains("feature/outer"), "{rendered}");
    assert!(rendered.contains("↳ carried  feature/inner"), "{rendered}");
    assert!(rendered.contains("↳ carried  feature/middle"), "{rendered}");
    assert!(!rendered.contains("also via"), "{rendered}");
    assert_eq!(filtered_feature_indices(&interaction, "inner"), vec![0]);
    Ok(())
}

#[test]
fn inventory_filter_matches_carried_branches_through_their_carrier() {
    let mut interaction = RestackInteraction::from_inventory(inventory_snapshot(
        UnsupportedHistory::from(InventoryError::DirectCommit {
            commit: "stray".to_owned(),
        }),
    ));
    let _ = interaction.update(RestackInteractionAction::AcceptInventoryFallback);
    assert_eq!(filtered_feature_indices(&interaction, "proj-12"), vec![0]);
    assert_eq!(
        filtered_feature_indices(&interaction, "nothing"),
        Vec::<usize>::new()
    );
}

#[test]
fn inventory_review_lists_dropped_commits_and_confirmation_states_the_loss(
) -> Result<(), Box<dyn std::error::Error>> {
    for orphan_count in [0_usize, 1, 200] {
        let (interaction, plan) = inventory_plan(orphan_count)?;
        let mut review = interaction.clone();
        review.review_ready();
        for (width, height) in [(60, 24), (120, 40)] {
            let rendered = rendered_at(&review, Some(&plan), None, width, height)?;
            assert!(rendered.contains("RESTACK REVIEW"), "{rendered}");
            assert!(
                rendered.contains("oldest tip first"),
                "{width}x{height}: {rendered}"
            );
            if orphan_count == 0 {
                assert!(rendered.contains("no commits dropped"), "{rendered}");
                assert!(!rendered.contains("Dropped commits ("));
            } else {
                assert!(
                    rendered.contains(&format!("{orphan_count} commit")),
                    "{rendered}"
                );
            }
        }
        if orphan_count > 0 {
            let tall = rendered_at(&review, Some(&plan), None, 120, 60)?;
            if orphan_count == 1 {
                assert!(tall.contains("Dropped commits (1)"), "{tall}");
                assert!(tall.contains("orphan-  2026-01-02  Pat"), "{tall}");
                assert!(tall.contains("lost work 0"), "{tall}");
            } else {
                assert!(tall.contains("Dropped commits (200)"), "{tall}");
            }
        }
        let mut details = review.clone();
        let _ = details.update(RestackInteractionAction::ToggleDetails);
        let detailed = rendered_at(&details, Some(&plan), None, 120, 60)?;
        assert!(
            detailed.contains("history proof failed with directCommit"),
            "{detailed}"
        );
        assert!(detailed.contains("1 branch(es) reached by a retained tip"));

        let mut confirmation = review.clone();
        let _ = confirmation.update(RestackInteractionAction::Continue);
        let rendered = rendered_at(&confirmation, Some(&plan), None, 60, 30)?;
        match orphan_count {
            0 => assert!(!rendered.contains("Drops "), "{rendered}"),
            1 => assert!(
                rendered.contains("Drops 1 commit that no retained branch"),
                "{rendered}"
            ),
            _ => assert!(
                rendered.contains("Drops 200 commits that no retained branch"),
                "{rendered}"
            ),
        }
        assert!(rendered.contains("Ctrl+Y"), "{rendered}");

        let text = success_text(&plan);
        assert!(text.contains("Rebuilt from inventory"), "{text}");
        if orphan_count == 1 {
            assert!(text.ends_with("1 commit dropped."), "{text}");
        }
    }
    let history = plan()?;
    assert!(!success_text(&history).contains("inventory"));
    let mut review = RestackInteraction::new(snapshot());
    review.review_ready();
    let rendered = rendered_at(&review, Some(&history), None, 120, 40)?;
    assert!(!rendered.contains("Dropped commits"));
    assert!(!rendered.contains("oldest tip first"));
    Ok(())
}
