use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use graduate::restack::RestackInteractionStage;

use super::super::handoff::{conflict_text, preserved_text, success_text};
use super::super::keys::action_for_key;
use super::super::render::{pad_text, render, truncate_text};
use super::*;

#[test]
fn confirmation_bounds_large_omission_lists_and_points_back_to_review(
) -> Result<(), Box<dyn std::error::Error>> {
    let mut plan = plan()?;
    plan.selection.removed = (1..=5)
        .map(|index| BranchIdentity {
            name: format!("feature/removed-{index}"),
            tip: format!("{index:040}"),
        })
        .collect();
    let mut interaction = RestackInteraction::new(snapshot());
    interaction.review_ready();
    let _ = interaction.update(RestackInteractionAction::Continue);

    let confirmation = rendered_at(&interaction, Some(&plan), None, 115, 38)?;

    assert!(confirmation.contains("5 omitted"));
    assert!(confirmation.contains("feature/removed-1"));
    assert!(confirmation.contains("feature/removed-3"));
    assert!(!confirmation.contains("feature/removed-4"));
    assert!(confirmation.contains("and 2 more; press Esc to review every omission"));
    Ok(())
}

#[test]
fn checklist_truncates_wide_unicode_by_terminal_columns() {
    let value = "feature/界界界界界界界界界界";
    let truncated = truncate_text(value, 16);
    let padded = pad_text(&truncated, 16);

    assert!(Line::raw(&truncated).width() <= 16);
    assert_eq!(Line::raw(&padded).width(), 16);
    assert!(truncated.ends_with('…'));
}

#[test]
fn compact_review_keeps_every_action_visible() -> Result<(), Box<dyn std::error::Error>> {
    let plan = plan()?;
    let mut interaction = RestackInteraction::new(snapshot());
    interaction.review_ready();
    let _ = interaction.update(RestackInteractionAction::ToggleDetails);
    let mut terminal = Terminal::new(TestBackend::new(60, 24))?;
    let mut view = RestackViewState::default();

    terminal.draw(|frame| render(frame, &interaction, Some(&plan), None, &mut view))?;
    let rendered = terminal.backend().to_string();

    assert!(view.scrollable);
    assert!(rendered.contains("Omitted from origin/qa"));
    assert!(rendered.contains("feature/two"));
    assert!(rendered.contains("to revise."));
    assert!(rendered.contains("PgUp/Dn Home/End Scroll"));
    assert!(rendered.contains("Enter Confirm publish"));
    assert!(rendered.contains("Esc Revise"));
    assert!(rendered.contains("d Details"));
    assert!(rendered.contains("q Cancel"));
    Ok(())
}

#[test]
fn review_navigates_hundreds_of_retained_features_and_keeps_details_near_the_top(
) -> Result<(), Box<dyn std::error::Error>> {
    let mut plan = plan()?;
    plan.selection.removed.clear();
    plan.selection.retained = (1..=250)
        .map(|index| BranchIdentity {
            name: format!("feature/{index:03}"),
            tip: format!("{index:040}"),
        })
        .collect();
    plan.merges = plan
        .selection
        .retained
        .iter()
        .map(|branch| MergeOutcome {
            branch: branch.name.clone(),
            tip: branch.tip.clone(),
            commit: format!("merge-{}", branch.name),
            tree: format!("tree-{}", branch.name),
            resolution: MergeResolution::Clean,
        })
        .collect();
    let mut interaction = RestackInteraction::new(snapshot());
    interaction.review_ready();
    let mut view = RestackViewState::default();
    let mut terminal = Terminal::new(TestBackend::new(80, 24))?;

    terminal.draw(|frame| render(frame, &interaction, Some(&plan), None, &mut view))?;
    let first_page = terminal.backend().to_string();
    assert!(view.scrollable);
    assert!(first_page.contains("250 retained"));
    assert!(first_page.contains("Plan details"));
    assert!(first_page.contains("Home/End Scroll"));

    let _ = interaction.update(RestackInteractionAction::MoveLast);
    terminal.draw(|frame| render(frame, &interaction, Some(&plan), None, &mut view))?;
    let last_page = terminal.backend().to_string();
    assert!(last_page.contains("feature/250"));

    let _ = interaction.update(RestackInteractionAction::MoveUp);
    terminal.draw(|frame| render(frame, &interaction, Some(&plan), None, &mut view))?;
    let above_last_page = terminal.backend().to_string();
    assert!(!above_last_page.contains("feature/250"));

    let _ = interaction.update(RestackInteractionAction::MoveFirst);
    terminal.draw(|frame| render(frame, &interaction, Some(&plan), None, &mut view))?;
    let returned = terminal.backend().to_string();
    assert!(returned.contains("RESTACK REVIEW"));
    assert!(returned.contains("Plan details"));
    Ok(())
}

#[test]
fn explicit_confirmation_and_cancel_keys_have_distinct_actions() {
    assert_eq!(
        action_for_key(
            RestackInteractionStage::Confirmation,
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)
        ),
        None
    );
    assert_eq!(
        action_for_key(
            RestackInteractionStage::Confirmation,
            KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE)
        ),
        None
    );
    assert_eq!(
        action_for_key(
            RestackInteractionStage::Confirmation,
            KeyEvent::new(KeyCode::Char('y'), KeyModifiers::CONTROL)
        ),
        Some(RestackInteractionAction::Confirm)
    );
    assert_eq!(
        action_for_key(
            RestackInteractionStage::Confirmation,
            KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE)
        ),
        Some(RestackInteractionAction::Cancel)
    );
    assert_eq!(
        action_for_key(
            RestackInteractionStage::Confirmation,
            KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)
        ),
        Some(RestackInteractionAction::Back)
    );
    assert_eq!(
        action_for_key(
            RestackInteractionStage::Confirmation,
            KeyEvent::new(KeyCode::Char('n'), KeyModifiers::NONE)
        ),
        None
    );
    assert_eq!(
        action_for_key(
            RestackInteractionStage::Review,
            KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE)
        ),
        Some(RestackInteractionAction::ToggleDetails)
    );
    assert_eq!(
        action_for_key(
            RestackInteractionStage::Review,
            KeyEvent::new(KeyCode::Home, KeyModifiers::NONE)
        ),
        Some(RestackInteractionAction::MoveFirst)
    );
    assert_eq!(
        action_for_key(
            RestackInteractionStage::Review,
            KeyEvent::new(KeyCode::End, KeyModifiers::NONE)
        ),
        Some(RestackInteractionAction::MoveLast)
    );
    assert_eq!(
        action_for_key(
            RestackInteractionStage::Selection,
            KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE)
        ),
        Some(RestackInteractionAction::MovePageDown)
    );
    assert_eq!(
        action_for_key(
            RestackInteractionStage::Selection,
            KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE)
        ),
        Some(RestackInteractionAction::KeepAll)
    );
}

#[test]
fn ordinary_completion_and_conflict_handoff_are_redacted_and_actionable(
) -> Result<(), Box<dyn std::error::Error>> {
    let plan = plan()?;
    let success = success_text(&plan);
    assert!(success.contains("Restacked origin/qa"));
    assert!(success.contains("1 retained, 1 omitted from the environment"));
    assert!(!success.contains("1 removed"));

    let paths = vec!["src/file\nname.rs".to_owned()];
    let handoff = conflict_text(&ConflictHandoff {
        environment: "qa",
        branch: "feature/PROJ-12-one",
        unresolved_paths: &paths,
        resume_token: "v1.safe.token",
        work_area: "/tmp/work\narea",
    });
    assert!(handoff.contains("src/file\\nname.rs"));
    assert!(handoff.contains("Work area: /tmp/work\\narea"));
    assert!(handoff.contains("1. Edit the unresolved files in the work area"));
    assert!(handoff.contains("2. Stage every resolution there"));
    assert!(handoff.contains("3. Resume with: gd restack qa --resume v1.safe.token"));
    assert!(handoff.contains("Do not commit; Graduate creates the canonical merge commit"));
    assert!(handoff.contains("expires after 24 hours of inactivity"));
    Ok(())
}

#[test]
fn preserved_text_names_every_way_to_finish_the_sealed_session() {
    let text = preserved_text("qa", "v1.token");
    assert!(text.starts_with("Restack of qa left unpublished; no remote refs changed."));
    assert!(text.contains("gd restack qa --resume v1.token\n"));
    assert!(text.contains("gd restack qa --resume v1.token --apply"));
    assert!(text.contains("gd restack qa --resume v1.token --abort"));
}
