use super::*;

#[test]
fn interaction_starts_with_every_feature_retained_in_merge_order() {
    let mut interaction = RestackInteraction::new(planning_snapshot());

    assert_eq!(interaction.stage(), RestackInteractionStage::Selection);
    assert!(interaction.is_retained(0));
    assert!(interaction.is_retained(1));
    assert_eq!(
        interaction.update(RestackInteractionAction::Continue),
        RestackInteractionEffect::Preview(RestackSelection {
            retained: vec![
                BranchIdentity {
                    name: "feature/a".to_owned(),
                    tip: "a".to_owned(),
                },
                BranchIdentity {
                    name: "feature/b".to_owned(),
                    tip: "b".to_owned(),
                },
            ],
            removed: Vec::new(),
        })
    );
}

#[test]
fn interaction_rejects_a_removal_that_a_retained_branch_still_carries() {
    let mut interaction = RestackInteraction::new(planning_snapshot());

    assert_eq!(
        interaction.retained_dependents(0),
        vec!["feature/b".to_owned()]
    );
    let effect = interaction.update(RestackInteractionAction::Toggle);

    assert_eq!(
        effect,
        RestackInteractionEffect::Rejected(SelectionError::RetainedDependency {
            branch: "feature/a".to_owned(),
            dependents: vec!["feature/b".to_owned()],
        })
    );
    assert!(interaction.is_retained(0));
}

#[test]
fn interaction_supports_batch_selection_and_inventory_navigation() {
    let mut interaction = RestackInteraction::new(planning_snapshot());

    let _ = interaction.update(RestackInteractionAction::RemoveAll);
    assert!(!interaction.is_retained(0));
    assert!(!interaction.is_retained(1));

    let _ = interaction.update(RestackInteractionAction::KeepAll);
    assert!(interaction.is_retained(0));
    assert!(interaction.is_retained(1));

    let _ = interaction.update(RestackInteractionAction::MoveLast);
    assert_eq!(interaction.cursor(), 1);
    let _ = interaction.update(RestackInteractionAction::MoveFirst);
    assert_eq!(interaction.cursor(), 0);
    let _ = interaction.update(RestackInteractionAction::MovePageDown);
    assert_eq!(interaction.cursor(), 1);
    let _ = interaction.update(RestackInteractionAction::MovePageUp);
    assert_eq!(interaction.cursor(), 0);
    let _ = interaction.update(RestackInteractionAction::MoveTo(1));
    assert_eq!(interaction.cursor(), 1);
}

#[test]
fn interaction_requires_review_then_explicit_confirmation() {
    let mut interaction = RestackInteraction::new(planning_snapshot());
    let _ = interaction.update(RestackInteractionAction::MoveDown);
    assert_eq!(interaction.cursor(), 1);
    let _ = interaction.update(RestackInteractionAction::MoveUp);
    assert_eq!(interaction.cursor(), 0);
    interaction.review_ready();

    assert!(!interaction.review_details());
    let _ = interaction.update(RestackInteractionAction::ToggleDetails);
    assert!(interaction.review_details());

    let _ = interaction.update(RestackInteractionAction::MoveDown);
    assert_eq!(interaction.review_scroll(), 1);
    let _ = interaction.update(RestackInteractionAction::MoveUp);
    assert_eq!(interaction.review_scroll(), 0);
    let _ = interaction.update(RestackInteractionAction::MoveLast);
    assert_eq!(interaction.review_scroll(), usize::MAX);
    let _ = interaction.update(RestackInteractionAction::MoveFirst);
    assert_eq!(interaction.review_scroll(), 0);

    assert_eq!(
        interaction.update(RestackInteractionAction::Continue),
        RestackInteractionEffect::None
    );
    assert_eq!(interaction.stage(), RestackInteractionStage::Confirmation);
    assert_eq!(
        interaction.update(RestackInteractionAction::Confirm),
        RestackInteractionEffect::Publish
    );
    assert_eq!(
        interaction.update(RestackInteractionAction::Back),
        RestackInteractionEffect::None
    );
    assert_eq!(interaction.stage(), RestackInteractionStage::Review);
    assert_eq!(
        interaction.update(RestackInteractionAction::Back),
        RestackInteractionEffect::Revise
    );
    assert_eq!(interaction.stage(), RestackInteractionStage::Selection);
}

#[test]
fn inventory_interaction_reaches_the_checklist_only_by_explicit_acceptance() {
    let snapshot = build_inventory_snapshot(&ambiguous_graph(), direct_reason(), &BTreeMap::new());
    let mut interaction = RestackInteraction::from_inventory(snapshot);
    assert_eq!(
        interaction.stage(),
        RestackInteractionStage::UnsupportedHistory
    );
    assert_eq!(interaction.inventory_mode(), InventoryMode::Reachability);
    assert_eq!(
        interaction
            .unsupported_history()
            .map(|reason| reason.kind.as_str()),
        Some("directCommit")
    );
    assert_eq!(interaction.carried_features().len(), 1);
    assert_eq!(interaction.orphaned_commit_count(), 1);

    for action in [
        RestackInteractionAction::Toggle,
        RestackInteractionAction::Continue,
        RestackInteractionAction::Confirm,
        RestackInteractionAction::Back,
        RestackInteractionAction::RemoveAll,
    ] {
        assert_eq!(interaction.update(action), RestackInteractionEffect::None);
        assert_eq!(
            interaction.stage(),
            RestackInteractionStage::UnsupportedHistory
        );
    }
    assert!(interaction.is_retained(0));
    assert_eq!(
        interaction.update(RestackInteractionAction::Cancel),
        RestackInteractionEffect::Cancel
    );

    assert_eq!(
        interaction.update(RestackInteractionAction::AcceptInventoryFallback),
        RestackInteractionEffect::None
    );
    assert_eq!(interaction.stage(), RestackInteractionStage::Selection);
    assert!(interaction.is_retained(0));
    assert_eq!(
        interaction.update(RestackInteractionAction::Toggle),
        RestackInteractionEffect::None
    );
    assert_eq!(interaction.orphaned_commit_count(), 4);
    assert_eq!(
        interaction.update(RestackInteractionAction::AcceptInventoryFallback),
        RestackInteractionEffect::None
    );
    assert_eq!(
        interaction.update(RestackInteractionAction::Back),
        RestackInteractionEffect::None
    );
    assert_eq!(interaction.stage(), RestackInteractionStage::Selection);
}

#[test]
fn history_interaction_never_visits_the_unsupported_stage() -> Result<(), Box<dyn std::error::Error>>
{
    let interaction = RestackInteraction::new(build_snapshot(&simple_graph())?);
    assert_eq!(interaction.stage(), RestackInteractionStage::Selection);
    assert_eq!(interaction.inventory_mode(), InventoryMode::History);
    assert_eq!(interaction.orphaned_commit_count(), 0);
    Ok(())
}
