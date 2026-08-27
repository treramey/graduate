use crossterm::event::{KeyCode, KeyEvent};
use graduate::restack::RestackInteractionStage;

use super::super::keys::{action_for_key, selection_action_for_key};
use super::*;

#[test]
fn unsupported_history_screen_explains_every_reason_and_fits_the_minimum_size(
) -> Result<(), Box<dyn std::error::Error>> {
    let reasons = [
        (
            InventoryError::AmbiguousFeatureRefs {
                merge_commit: "886faef4b24230540b9a5d8ae057a233a7dd0126".to_owned(),
                feature_parent: "0bbff862c139d704a3c6431d9fa7f16c55c1aa5a".to_owned(),
                branches: (1..=17).map(|n| format!("HPM-{n}")).collect(),
            },
            vec![
                "brings in 0bbff86, which 17",
                "cannot tell which one it meant",
                "HPM-3",
                "and 14 more",
            ],
        ),
        (
            InventoryError::DeletedFeatureRef {
                merge_commit: "a".repeat(40),
                feature_parent: "b".repeat(40),
            },
            vec!["no remote branch contains it any more"],
        ),
        (
            InventoryError::DirectCommit {
                commit: "c".repeat(40),
            },
            vec!["was made directly on qa"],
        ),
        (
            InventoryError::FastForwardHistory {
                commit: "d".repeat(40),
                branches: vec!["feature/ff".to_owned()],
            },
            vec!["fast-forwarded through ddddddd", "feature/ff"],
        ),
        (
            InventoryError::OctopusMerge {
                merge_commit: "e".repeat(40),
                parents: 3,
            },
            vec!["has 3 parents"],
        ),
        (
            InventoryError::MissingCommit {
                commit: "f".repeat(40),
            },
            vec!["is missing from the fetched history"],
        ),
    ];
    for (error, expectations) in reasons {
        let interaction = RestackInteraction::from_inventory(inventory_snapshot(error.into()));
        for (width, height) in [(60, 24), (100, 30)] {
            let rendered = rendered_at(&interaction, None, None, width, height)?;
            assert!(
                rendered.contains("HISTORY CANNOT BE READ"),
                "{width}x{height}: {rendered}"
            );
            for expectation in &expectations {
                assert!(
                    rendered.contains(expectation),
                    "{width}x{height} missing {expectation:?}:\n{rendered}"
                );
            }
            assert!(rendered.contains("Rebuilding from inventory instead"));
            assert!(rendered.contains("Membership: remote tips in qa, not in main. You pick."));
            assert!(rendered.contains("No reused resolutions"));
            assert!(rendered.contains("dropped; listed first"));
            assert!(rendered.contains("1 top-level branch · 1 carried · 1 commit dropped"));
            assert!(rendered.contains("Rebuild from inventory"));
            assert!(rendered.contains("Cancel"));
            assert!(!rendered.contains("SELECT FEATURES"));
        }
    }
    Ok(())
}

#[test]
fn unsupported_history_screen_routes_live_keys_by_stage() {
    let interaction = RestackInteraction::from_inventory(inventory_snapshot(
        UnsupportedHistory::from(InventoryError::DirectCommit {
            commit: "stray".to_owned(),
        }),
    ));
    let mut view = RestackViewState::default();
    assert_eq!(
        selection_action_for_key(&interaction, &mut view, KeyEvent::from(KeyCode::Char('r'))),
        Some(RestackInteractionAction::AcceptInventoryFallback)
    );
    assert_eq!(
        selection_action_for_key(&interaction, &mut view, KeyEvent::from(KeyCode::Char('/'))),
        None
    );
    assert!(!view.filtering);
    assert_eq!(
        selection_action_for_key(&interaction, &mut view, KeyEvent::from(KeyCode::Char(' '))),
        None
    );
    assert_eq!(
        selection_action_for_key(&interaction, &mut view, KeyEvent::from(KeyCode::Esc)),
        Some(RestackInteractionAction::Cancel)
    );
}

#[test]
fn unsupported_history_keys_accept_the_fallback_or_cancel() {
    let stage = RestackInteractionStage::UnsupportedHistory;
    assert_eq!(
        action_for_key(stage, KeyEvent::from(KeyCode::Char('r'))),
        Some(RestackInteractionAction::AcceptInventoryFallback)
    );
    assert_eq!(
        action_for_key(stage, KeyEvent::from(KeyCode::Esc)),
        Some(RestackInteractionAction::Cancel)
    );
    assert_eq!(
        action_for_key(stage, KeyEvent::from(KeyCode::Char('q'))),
        Some(RestackInteractionAction::Cancel)
    );
    assert_eq!(action_for_key(stage, KeyEvent::from(KeyCode::Enter)), None);
    assert_eq!(
        action_for_key(stage, KeyEvent::from(KeyCode::Char(' '))),
        None
    );
    assert_eq!(
        action_for_key(
            RestackInteractionStage::Selection,
            KeyEvent::from(KeyCode::Char('r'))
        ),
        None
    );
}
