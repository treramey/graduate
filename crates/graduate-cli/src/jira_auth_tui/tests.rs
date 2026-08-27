//! Tests.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use graduate::jira::JiraValidationError;
use ratatui::backend::TestBackend;
use ratatui::style::Modifier;
use ratatui::Terminal;

use super::events::{reduced_motion_value, Action};
use super::render::render;
use super::*;
use crate::theme::{GRADUATE_ART, MUTED_COLOR, PRIMARY_COLOR};

const TEST_WIDTH: u16 = 100;

const TEST_HEIGHT: u16 = 50;

fn model(stage: OnboardingScreen) -> OnboardingModel {
    OnboardingModel {
        pending_animation_elapsed: Duration::ZERO,
        cursor_blink_elapsed: Duration::ZERO,
        reduced_motion: true,
        stage,
        focus: 0,
        hostname: "company.atlassian.net".into(),
        email: "person@example.com".into(),
        display_name: "Example Person".to_owned(),
        jira_token: Input::default(),
        can_retain_jira_token: false,
        jira_instruction: "Create or manage your Atlassian API token:".to_owned(),
        jira_url: "https://id.atlassian.com/manage-profile/security/api-tokens".to_owned(),
        jira_page_can_open: false,
        jira_page_loaded: true,
        jira_status: ConnectionStatus::NotConnected,
        error: None,
        warning: None,
    }
}

fn render_text(
    width: u16,
    height: u16,
    model: &OnboardingModel,
) -> Result<String, Box<dyn std::error::Error>> {
    let mut terminal = Terminal::new(TestBackend::new(width, height))?;
    terminal.draw(|frame| {
        render(frame, model);
    })?;
    let buffer = terminal.backend().buffer();
    let mut rendered = String::new();
    for row in 0..buffer.area.height {
        for column in 0..buffer.area.width {
            rendered.push_str(buffer[(column, row)].symbol());
        }
        rendered.push('\n');
    }
    Ok(rendered)
}

#[test]
fn jira_details_use_compact_inline_prompts() -> Result<(), Box<dyn std::error::Error>> {
    let rendered = render_text(
        TEST_WIDTH,
        TEST_HEIGHT,
        &model(OnboardingScreen::JiraDetails),
    )?;

    assert!(rendered.contains(GRADUATE_ART[0]));
    assert!(rendered.contains("Jira site> company.atlassian.net"));
    assert!(rendered.contains("Atlassian email> person@example.com"));
    assert!(rendered.contains("Continue to API token"));
    assert!(!rendered.contains("Your Atlassian workspace address"));
    assert!(rendered.contains("┌"));
    assert!(rendered.contains("┘"));
    assert!(!rendered.contains("Review & save"));
    let heading = rendered
        .lines()
        .find(|line| line.contains("Connect your Jira account"))
        .ok_or("Jira heading was not rendered")?;
    assert!(heading.starts_with("Connect your Jira account"));
    assert!(rendered.contains("Enter the Atlassian account Graduate should use"));
    Ok(())
}

#[test]
fn enter_moves_through_both_fields_to_the_continue_button() {
    let mut model = model(OnboardingScreen::JiraDetails);

    assert!(matches!(
        model.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
        Action::None
    ));
    assert_eq!(model.focus, 1);
    assert!(matches!(
        model.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
        Action::None
    ));
    assert_eq!(model.focus, 2);
    assert!(matches!(
        model.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
        Action::Continue
    ));
}

#[test]
fn arrow_keys_move_focus_in_both_directions() {
    let mut model = model(OnboardingScreen::JiraDetails);

    let _ = model.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
    assert_eq!(model.focus, 1);

    let _ = model.handle_key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
    assert_eq!(model.focus, 0);

    let _ = model.handle_key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
    assert_eq!(model.focus, 2);
}

#[test]
fn text_input_edits_at_the_unicode_aware_cursor() {
    let mut model = model(OnboardingScreen::JiraDetails);
    model.hostname = "café".into();

    let _ = model.handle_key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE));
    let _ = model.handle_key(KeyEvent::new(KeyCode::Char('!'), KeyModifiers::NONE));

    assert_eq!(model.hostname.value(), "caf!é");
}

#[test]
fn token_screen_masks_input_and_renders_a_focusable_connect_button(
) -> Result<(), Box<dyn std::error::Error>> {
    let mut model = model(OnboardingScreen::JiraToken);
    model.jira_token = "never-render-this-secret".into();
    model.focus = 1;

    let rendered = render_text(TEST_WIDTH, TEST_HEIGHT, &model)?;

    assert!(rendered.contains("Atlassian API token"));
    assert!(rendered.contains("••••"));
    assert!(!rendered.contains("never-render-this-secret"));
    assert!(rendered.contains("Connect Jira"));
    assert!(rendered.contains("┌"));
    Ok(())
}

#[test]
fn retained_token_is_labeled_without_loading_the_secret() -> Result<(), Box<dyn std::error::Error>>
{
    let mut model = model(OnboardingScreen::JiraToken);
    model.can_retain_jira_token = true;

    let rendered = render_text(TEST_WIDTH, TEST_HEIGHT, &model)?;

    assert!(rendered.contains("Atlassian API token (stored)"));
    assert!(rendered.contains("••••••••••••"));
    Ok(())
}

#[test]
fn review_uses_a_compact_field_table_and_confirmation() -> Result<(), Box<dyn std::error::Error>> {
    let mut model = model(OnboardingScreen::Save);
    model.jira_status = ConnectionStatus::Connected;

    let rendered = render_text(TEST_WIDTH, TEST_HEIGHT, &model)?;

    assert!(rendered.contains("Review Jira setup"));
    assert!(rendered.contains("Field     Value"));
    assert!(rendered.contains("Site      company.atlassian.net"));
    assert!(rendered.contains("Does this look right?"));
    assert!(rendered.contains("Save configuration"));
    assert!(rendered.contains("┌"));
    assert!(rendered.contains("J Change Jira account"));
    assert!(!rendered.contains("Tempo"));
    Ok(())
}

#[test]
fn action_uses_the_main_tui_focus_style() -> Result<(), Box<dyn std::error::Error>> {
    for focus in [0, 2] {
        let mut model = model(OnboardingScreen::JiraDetails);
        model.focus = focus;
        let mut terminal = Terminal::new(TestBackend::new(TEST_WIDTH, TEST_HEIGHT))?;
        terminal.draw(|frame| render(frame, &model))?;

        let corners = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .filter(|cell| matches!(cell.symbol(), "┌" | "┐" | "└" | "┘"))
            .collect::<Vec<_>>();
        assert_eq!(corners.len(), 4);
        if focus == 2 {
            assert!(corners.iter().all(|cell| cell.fg == PRIMARY_COLOR));
            assert!(corners.iter().all(|cell| cell.bg != MUTED_COLOR));
        } else {
            assert!(corners.iter().all(|cell| cell.fg == MUTED_COLOR));
            assert!(corners.iter().all(|cell| cell.bg != MUTED_COLOR));
        }
        let focused_cells = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .filter(|cell| cell.modifier.contains(Modifier::REVERSED))
            .count();
        if focus == 2 {
            assert!(focused_cells >= "Continue to API token".len());
        } else {
            assert_eq!(focused_cells, 1);
        }

        let left_edge = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .position(|cell| cell.symbol() == "┌")
            .ok_or("button left edge was not rendered")?;
        assert_eq!(left_edge % usize::from(TEST_WIDTH), 0);
    }
    Ok(())
}

#[test]
fn resize_message_replaces_the_form_but_preserves_cancel_help(
) -> Result<(), Box<dyn std::error::Error>> {
    let rendered = render_text(
        MIN_TERMINAL_WIDTH - 1,
        MIN_TERMINAL_HEIGHT - 1,
        &model(OnboardingScreen::JiraDetails),
    )?;

    assert!(rendered.contains("Terminal too small"));
    assert!(rendered.contains("Ctrl-C cancels without saving"));
    assert!(!rendered.contains("Atlassian email"));
    Ok(())
}

#[test]
fn every_screen_fits_a_60_by_24_split_pane() -> Result<(), Box<dyn std::error::Error>> {
    for (stage, heading, action) in [
        (
            OnboardingScreen::JiraDetails,
            "Connect your Jira account",
            "Continue to API token",
        ),
        (OnboardingScreen::JiraToken, "Connect Jira", "Connect Jira"),
        (
            OnboardingScreen::Save,
            "Review Jira setup",
            "Save configuration",
        ),
    ] {
        let rendered = render_text(60, 24, &model(stage))?;

        assert!(!rendered.contains("Terminal too small"));
        assert!(rendered.contains(GRADUATE_ART[0]));
        assert!(rendered.contains(heading));
        assert!(rendered.contains(action));
        assert!(rendered.contains("┌"));
        assert!(rendered.contains("┘"));
    }
    Ok(())
}

#[test]
fn untrusted_review_values_are_rendered_visibly() -> Result<(), Box<dyn std::error::Error>> {
    let mut model = model(OnboardingScreen::Save);
    model.display_name = "Person\nInjected\u{202e}".to_owned();

    let rendered = render_text(TEST_WIDTH, TEST_HEIGHT, &model)?;

    assert!(rendered.contains("Person\\nInjected\\u{202e}"));
    assert!(!rendered.contains("Person\nInjected"));
    Ok(())
}

#[test]
fn reduced_motion_environment_accepts_only_explicit_truthy_values() {
    for value in [Some("1"), Some("true"), Some("YES"), Some("on")] {
        assert!(reduced_motion_value(value));
    }
    for value in [None, Some(""), Some("0"), Some("false"), Some("no")] {
        assert!(!reduced_motion_value(value));
    }
}

#[test]
fn core_validation_errors_select_the_matching_field() {
    let mut model = model(OnboardingScreen::JiraDetails);
    let error = OnboardingError::JiraValidation(JiraValidationError::AtlassianEmailRequired);

    model.show_validation_error(&error);

    assert_eq!(model.focus, 1);
    assert_eq!(model.error.as_deref(), Some("Atlassian email is required"));
}
