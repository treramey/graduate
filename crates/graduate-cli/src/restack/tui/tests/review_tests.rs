use ratatui::text::Text;

use super::super::render::{action_allowed_when_undersized, confirmation_minimum_height, render};
use super::super::review::wrapped_text_height;
use super::*;

#[test]
fn undersized_terminal_replaces_the_workflow_with_resize_guidance(
) -> Result<(), Box<dyn std::error::Error>> {
    let interaction = RestackInteraction::new(snapshot());
    let rendered = rendered_at(&interaction, None, None, 55, 11)?;

    assert!(rendered.contains("Terminal too small for a safe restack review"));
    assert!(rendered.contains("56 columns × 18 rows"));
    assert!(!rendered.contains("SELECT FEATURES"));
    Ok(())
}

#[test]
fn undersized_review_allows_escape_but_blocks_progression_and_publication(
) -> Result<(), Box<dyn std::error::Error>> {
    let plan = plan()?;
    let mut interaction = RestackInteraction::new(snapshot());
    interaction.review_ready();
    let mut view = RestackViewState::default();
    let mut terminal = Terminal::new(TestBackend::new(55, 11))?;

    terminal.draw(|frame| render(frame, &interaction, Some(&plan), None, &mut view))?;

    assert!(view.undersized);
    assert!(!action_allowed_when_undersized(
        RestackInteractionAction::Continue
    ));
    assert!(!action_allowed_when_undersized(
        RestackInteractionAction::Confirm
    ));
    assert!(action_allowed_when_undersized(
        RestackInteractionAction::Back
    ));
    assert!(action_allowed_when_undersized(
        RestackInteractionAction::Cancel
    ));
    Ok(())
}

#[test]
fn wrapped_height_uses_paragraph_word_boundaries() {
    let text = Text::from("aaaa aaaa aaaa");

    assert_eq!(wrapped_text_height(&text, 8), 3);
}

#[test]
fn review_and_confirmation_show_the_exact_rewrite_and_safety_effects(
) -> Result<(), Box<dyn std::error::Error>> {
    let plan = plan()?;
    let mut interaction = RestackInteraction::new(snapshot());
    interaction.review_ready();
    let review = rendered(&interaction, Some(&plan), None)?;

    assert!(review.contains("RESTACK REVIEW"));
    assert!(review.contains("Remote rewrite"));
    assert!(review.contains("environ"));
    assert!(review.contains("preview"));
    assert!(review.contains("1 retained · 1 omitted from the rebuilt environment · 1 clean merge"));
    assert!(review.contains("publish stops if origin/qa changed since this review"));
    assert!(!review.contains("Unchanged"));
    assert!(!review.contains("Target          origin/qa"));
    assert!(!review.contains("Commit signing"));
    assert!(!review.contains("Starts from"));
    assert!(!review.contains("Builds"));
    assert!(review.contains("Retained merge order"));
    assert!(review.contains("Selected feature tips are rebuilt in this order"));
    assert!(review.contains("✓ clean"));
    let retained_header = review
        .lines()
        .find(|line| line.contains("BRANCH") && line.contains("OUTCOME"))
        .ok_or("retained header was not rendered")?;
    let retained_row = review
        .lines()
        .find(|line| line.contains("feature/PROJ-12-one"))
        .ok_or("retained row was not rendered")?;
    assert_eq!(retained_header.find('#'), retained_row.find('1'));
    assert_eq!(
        retained_header.find("BRANCH"),
        retained_row.find("feature/PROJ-12-one")
    );
    assert!(review.contains("Omitted from origin/qa"));
    assert!(review.contains("remote branches are not changed or deleted; press Esc to revise"));
    assert!(review.contains("omitted by your selection"));
    assert!(review.contains("Plan details  ·  d show refs, identities, endpoints, and signing"));
    assert!(!review.contains("sha256:ffffffff"));
    assert!(review.contains("Enter Confirm publish"));
    assert!(!review.contains("↑/↓ Scroll"));
    let footer_row = review
        .lines()
        .position(|line| line.contains("Enter Confirm publish"))
        .ok_or("review footer was not rendered")?;
    assert!(footer_row >= 36);

    let _ = interaction.update(RestackInteractionAction::ToggleDetails);
    let details = rendered(&interaction, Some(&plan), None)?;
    assert!(details.contains("refs/remotes/origin/main @ main-tip"));
    assert!(details.contains("Pat <pat@example.com>"));
    assert!(details.contains("sha256:ffffffff"));
    assert!(details.contains("unsigned canonical merge commits"));
    assert!(details.contains("0 exact phase marker(s)"));
    assert!(details.contains("history proof; every commit attributed"));
    for _ in 0..22 {
        let _ = interaction.update(RestackInteractionAction::MoveDown);
    }
    let identities = rendered_at(&interaction, Some(&plan), None, 115, 40)?;
    assert!(identities.contains("Exact feature identities"));
    assert!(identities.contains("retained  feature/PROJ-12-one @ aaaaaaaaaa"));
    assert!(identities.contains("removed   feature/two @ bbbbbbbbbb"));

    let _ = interaction.update(RestackInteractionAction::Continue);
    let confirmation = rendered(&interaction, Some(&plan), None)?;
    assert!(confirmation.contains("publish stops if origin/qa changed since review"));
    assert!(confirmation.contains("(exact lease)"));
    assert!(confirmation.contains("Current tip     origin/qa @ environ"));
    assert!(confirmation.contains("Reviewed tip    origin/qa @ preview"));
    assert!(confirmation.contains("rebuild origin/qa from 1 retained feature"));
    assert!(confirmation.contains("1 omitted · 1 clean merge"));
    assert!(confirmation.contains("Omitted from the reviewed result"));
    assert!(confirmation.contains("feature/two @ bbbbbbb"));
    assert!(confirmation.contains("collaborators tracking it must resync after publish"));
    assert!(confirmation.contains("Feature branches and local work remain unchanged"));
    assert!(confirmation.contains("Press Ctrl+Y to replace origin/qa"));
    assert!(confirmation.contains("q abandons this plan without changing refs"));
    assert!(confirmation.contains("Ctrl+Y Publish"));
    assert!(confirmation.contains("Esc Review details"));
    assert!(confirmation.contains("q Abandon plan"));
    assert!(!confirmation.contains("unsigned"));
    let compact_confirmation = rendered_at(&interaction, Some(&plan), None, 80, 24)?;
    assert!(compact_confirmation.contains("rebuild origin/qa from 1 retained feature"));
    assert!(compact_confirmation.contains("publish stops if origin/qa changed since review"));
    assert!(compact_confirmation.contains("collaborators tracking it must resync"));
    assert!(compact_confirmation.contains("Press Ctrl+Y to replace origin/qa"));
    assert!(compact_confirmation.contains("Ctrl+Y Publish"));
    Ok(())
}

#[test]
fn short_confirmation_requires_enough_height_for_the_publish_warning(
) -> Result<(), Box<dyn std::error::Error>> {
    let plan = plan()?;
    let mut interaction = RestackInteraction::new(snapshot());
    interaction.review_ready();
    let _ = interaction.update(RestackInteractionAction::Continue);
    let minimum_height = confirmation_minimum_height(Some(&plan), 56);

    let rendered = rendered_at(
        &interaction,
        Some(&plan),
        None,
        56,
        minimum_height.saturating_sub(1),
    )?;

    assert!(rendered.contains("Terminal too small for a safe restack review"));
    assert!(rendered.contains(&format!("56 columns × {minimum_height} rows")));
    assert!(!rendered.contains("PUBLISH REMOTE REWRITE"));

    let boundary = rendered_at(&interaction, Some(&plan), None, 56, minimum_height)?;
    assert!(boundary.contains("Feature branches and local work"));
    assert!(boundary.contains("Press Ctrl+Y to replace origin/qa"));
    Ok(())
}
