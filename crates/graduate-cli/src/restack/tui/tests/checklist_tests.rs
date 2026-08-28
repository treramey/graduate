use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::super::keys::selection_action_for_key;
use super::super::render::{render, selection_error_message};
use super::*;

#[test]
fn checklist_renders_order_identity_jira_key_and_rerere_availability(
) -> Result<(), Box<dyn std::error::Error>> {
    let interaction = RestackInteraction::new(snapshot());
    let rendered = rendered(&interaction, None, None)?;

    assert!(rendered.contains("SELECT FEATURES"));
    assert!(rendered.contains("● 1 Select › ○ 2 Review › ○ 3 Publish"));
    assert!(!rendered.contains("Filter:"));
    assert!(rendered.contains("feature/PROJ-12-one"));
    assert!(rendered.contains("aaaaaaa"));
    assert!(rendered.contains("PROJ-12"));
    assert!(rendered.contains("available"));
    assert!(rendered.contains("feature/two"));
    assert!(rendered.contains("2 retained · 0 removed"));
    assert!(rendered.contains("◆ Required by feature/two"));
    let footer_row = rendered
        .lines()
        .position(|line| line.contains("Enter Review"))
        .ok_or("selection footer was not rendered")?;
    assert!(footer_row >= 36);
    Ok(())
}

#[test]
fn dependency_rejection_names_the_retained_dependent() -> Result<(), Box<dyn std::error::Error>> {
    let mut interaction = RestackInteraction::new(snapshot());
    let rejection = match interaction.update(RestackInteractionAction::Toggle) {
        RestackInteractionEffect::Rejected(error) => selection_error_message(&error, "main"),
        _ => String::new(),
    };
    let rendered = rendered(&interaction, None, Some(&rejection))?;

    assert!(rendered.contains("Cannot remove feature/PROJ-12-one"));
    assert!(rendered.contains("feature/two"));
    assert!(interaction.is_retained(0));
    let compact = rendered_at(&interaction, None, Some(&rejection), 56, 24)?;
    assert!(compact.contains("feature/two"));
    Ok(())
}

#[test]
fn checklist_updates_the_impact_summary_after_a_toggle() -> Result<(), Box<dyn std::error::Error>> {
    let mut snapshot = snapshot();
    snapshot.attributed_commits.clear();
    let mut interaction = RestackInteraction::new(snapshot);
    let _ = interaction.update(RestackInteractionAction::Toggle);

    let rendered = rendered(&interaction, None, None)?;

    assert!(rendered.contains("1 retained · 1 removed"));
    assert!(rendered.contains("Space Toggle"));
    Ok(())
}

#[test]
fn checklist_preserves_list_viewport_while_the_cursor_moves(
) -> Result<(), Box<dyn std::error::Error>> {
    let mut snapshot = snapshot();
    snapshot.features = (1..=10)
        .map(|index| ExplicitFeature {
            name: format!("feature/{index}"),
            tip: format!("{index:040}"),
            historical_merges: Vec::new(),
        })
        .collect();
    snapshot.attributed_commits.clear();
    let mut interaction = RestackInteraction::new(snapshot);
    for _ in 0..9 {
        let _ = interaction.update(RestackInteractionAction::MoveDown);
    }
    let mut view = RestackViewState::default();
    let mut terminal = Terminal::new(TestBackend::new(80, 18))?;

    terminal.draw(|frame| render(frame, &interaction, None, None, &mut view))?;
    let scrolled_offset = view.feature_list.offset();
    assert!(scrolled_offset > 0);
    let _ = interaction.update(RestackInteractionAction::MoveUp);
    terminal.draw(|frame| render(frame, &interaction, None, None, &mut view))?;

    assert_eq!(view.feature_list.selected(), Some(interaction.cursor()));
    assert_eq!(view.feature_list.offset(), scrolled_offset);
    Ok(())
}

#[test]
fn compact_checklist_reflows_issue_and_history_instead_of_hiding_them(
) -> Result<(), Box<dyn std::error::Error>> {
    let interaction = RestackInteraction::new(snapshot());
    let rendered = rendered_at(&interaction, None, None, 80, 24)?;

    assert!(!rendered.contains("Filter:"));
    assert!(rendered.contains("PROJ-12"));
    assert!(rendered.contains("available"));
    assert!(rendered.contains("/ Filter"));
    assert!(rendered.contains("? Shortcuts"));
    assert!(rendered.contains("◆ Required by feature/two"));
    assert!(!rendered.contains("a keep all"));
    Ok(())
}

#[test]
fn checklist_reveals_secondary_shortcuts_on_request() -> Result<(), Box<dyn std::error::Error>> {
    let mut snapshot = snapshot();
    snapshot.attributed_commits.clear();
    let mut interaction = RestackInteraction::new(snapshot);
    let _ = interaction.update(RestackInteractionAction::MoveDown);
    let mut view = RestackViewState::default();

    assert_eq!(
        selection_action_for_key(
            &interaction,
            &mut view,
            KeyEvent::new(KeyCode::Char('?'), KeyModifiers::NONE),
        ),
        None
    );
    let mut terminal = Terminal::new(TestBackend::new(80, 24))?;
    terminal.draw(|frame| render(frame, &interaction, None, None, &mut view))?;
    let rendered = terminal.backend().to_string();

    assert!(rendered.contains("a keep all · x remove all"));
    assert!(rendered.contains("Home/End first/last"));
    assert!(rendered.contains("? Hide shortcuts"));
    Ok(())
}

#[test]
fn wide_checklist_collapses_each_feature_to_one_evidence_row(
) -> Result<(), Box<dyn std::error::Error>> {
    let interaction = RestackInteraction::new(snapshot());
    let rendered = rendered_at(&interaction, None, None, 100, 24)?;
    let feature_line = rendered
        .lines()
        .find(|line| line.contains("feature/PROJ-12-one"))
        .ok_or("wide feature row was not rendered")?;

    assert!(feature_line.contains("aaaaaaa"));
    assert!(feature_line.contains("PROJ-12"));
    assert!(feature_line.contains("history: available"));
    Ok(())
}

#[test]
fn checklist_filter_narrows_rows_and_keeps_selection_on_a_visible_branch(
) -> Result<(), Box<dyn std::error::Error>> {
    let mut snapshot = snapshot();
    snapshot.attributed_commits.clear();
    let mut interaction = RestackInteraction::new(snapshot);
    let mut view = RestackViewState::default();
    assert_eq!(
        selection_action_for_key(
            &interaction,
            &mut view,
            KeyEvent::new(KeyCode::Char('/'), KeyModifiers::NONE),
        ),
        None
    );
    for character in ['t', 'w', 'o'] {
        if let Some(action) = selection_action_for_key(
            &interaction,
            &mut view,
            KeyEvent::new(KeyCode::Char(character), KeyModifiers::NONE),
        ) {
            let _ = interaction.update(action);
        }
    }
    let mut terminal = Terminal::new(TestBackend::new(80, 24))?;
    terminal.draw(|frame| render(frame, &interaction, None, None, &mut view))?;
    let rendered = terminal.backend().to_string();

    assert_eq!(view.filter, "two");
    assert_eq!(interaction.cursor(), 1);
    assert!(rendered.contains("feature/two"));
    assert!(!rendered.contains("feature/PROJ-12-one"));
    assert!(rendered.contains("1/2"));
    assert!(rendered.contains("Filter: two▏"));
    assert!(!rendered.contains("Enter Review"));
    Ok(())
}

#[test]
fn checklist_explains_an_empty_filter_result() -> Result<(), Box<dyn std::error::Error>> {
    let interaction = RestackInteraction::new(snapshot());
    let mut view = RestackViewState {
        filter: "missing".to_owned(),
        ..RestackViewState::default()
    };
    let mut terminal = Terminal::new(TestBackend::new(80, 24))?;

    terminal.draw(|frame| render(frame, &interaction, None, None, &mut view))?;
    let rendered = terminal.backend().to_string();

    assert!(rendered.contains("No branches match “missing”"));
    assert!(rendered.contains("0/2"));
    Ok(())
}
