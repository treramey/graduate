//! Screen rendering.

use graduate::jira_auth::OnboardingScreen;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Stylize;
use ratatui::text::{Line, Text};
use ratatui::widgets::{Paragraph, Wrap};
use ratatui::Frame;

use super::events::size_is_undersized;
use super::widgets::{
    render_feedback, render_field, render_footer, FieldPresentation, SetupButton,
};
use super::{
    ConnectionStatus, OnboardingModel, MIN_TERMINAL_HEIGHT, MIN_TERMINAL_WIDTH, REVIEW_LABEL_WIDTH,
};
use crate::shared::error::CliError;
use crate::shared::terminal::StderrTerminal;
use crate::shared::terminal_text;
use crate::shared::theme::{
    constrain_content_width, render_brand_header, Palette, GRADUATE_ART_HEIGHT,
};

pub(super) fn draw(
    terminal: &mut StderrTerminal,
    model: &mut OnboardingModel,
) -> Result<(), CliError> {
    terminal.terminal_mut().draw(|frame| {
        render(frame, model);
    })?;
    Ok(())
}

pub(super) fn render(frame: &mut Frame<'_>, model: &OnboardingModel) {
    if size_is_undersized(frame.area().width, frame.area().height) {
        render_resize_message(frame, frame.area());
        return;
    }
    let [_top_padding, header, _gap, body, footer] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(GRADUATE_ART_HEIGHT),
        Constraint::Length(2),
        Constraint::Fill(1),
        Constraint::Length(1),
    ])
    .areas(frame.area());
    let header = constrain_content_width(header);
    let body = constrain_content_width(body);
    let footer = constrain_content_width(footer);

    render_brand_header(frame, header);
    match model.stage {
        OnboardingScreen::JiraDetails => render_jira_details(frame, body, model),
        OnboardingScreen::JiraToken => render_jira_token(frame, body, model),
        OnboardingScreen::Save => render_save(frame, body, model),
    }
    render_footer(frame, footer, model);
}

fn render_resize_message(frame: &mut Frame<'_>, area: Rect) {
    let message = Text::from(vec![
        Line::from("Terminal too small").bold(),
        Line::default(),
        Line::from(format!(
            "Current size: {} columns by {} rows.",
            area.width, area.height
        )),
        Line::from(format!(
            "Resize to at least {MIN_TERMINAL_WIDTH} columns by {MIN_TERMINAL_HEIGHT} rows to continue."
        )),
        Line::from("Your input is preserved.").dim(),
        Line::from("Ctrl-C cancels without saving.").dim(),
    ]);
    frame.render_widget(
        Paragraph::new(message).centered().wrap(Wrap { trim: true }),
        area,
    );
}

fn render_jira_details(frame: &mut Frame<'_>, area: Rect, model: &OnboardingModel) {
    let [intro, _, hostname, _, email, _, action, _, feedback] = Layout::vertical([
        Constraint::Length(2),
        Constraint::Length(2),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(2),
        Constraint::Length(3),
        Constraint::Length(1),
        Constraint::Fill(1),
    ])
    .areas(area);
    frame.render_widget(
        Paragraph::new(Text::from(vec![
            Line::from("Connect your Jira account").bold(),
            Line::from("Enter the Atlassian account Graduate should use.").dim(),
        ])),
        intro,
    );
    render_field(
        frame,
        hostname,
        "Jira site",
        &terminal_text::escape(model.hostname.value()),
        "company.atlassian.net",
        FieldPresentation {
            cursor: model.hostname.cursor(),
            focused: model.focus == 0,
            cursor_visible: model.cursor_visible(),
            invalid: model
                .error
                .as_deref()
                .is_some_and(|error| error.contains("Jira site")),
            ..FieldPresentation::default()
        },
    );
    render_field(
        frame,
        email,
        "Atlassian email",
        &terminal_text::escape(model.email.value()),
        "you@example.com",
        FieldPresentation {
            cursor: model.email.cursor(),
            focused: model.focus == 1,
            cursor_visible: model.cursor_visible(),
            invalid: model
                .error
                .as_deref()
                .is_some_and(|error| error.contains("Atlassian email")),
            ..FieldPresentation::default()
        },
    );
    frame.render_widget(
        SetupButton::new(
            "Continue to API token",
            model.focus == 2,
            ConnectionStatus::NotConnected,
            model.pending_symbol(),
        ),
        action,
    );
    render_feedback(frame, feedback, model);
}

fn render_jira_token(frame: &mut Frame<'_>, area: Rect, model: &OnboardingModel) {
    let fallback_height = if !model.jira_page_can_open || model.warning.is_some() {
        3
    } else {
        0
    };
    let [intro, _, token, _, raw_url, _, status, _, feedback] = Layout::vertical([
        Constraint::Length(2),
        Constraint::Length(2),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(fallback_height),
        Constraint::Length(1),
        Constraint::Length(3),
        Constraint::Length(1),
        Constraint::Fill(1),
    ])
    .areas(area);
    frame.render_widget(
        Paragraph::new(Text::from(vec![
            Line::from("Connect Jira").bold(),
            Line::from("Paste an Atlassian API token.").dim(),
        ])),
        intro,
    );
    render_field(
        frame,
        token,
        "Atlassian API token",
        model.jira_token.value(),
        "paste token",
        FieldPresentation {
            cursor: model.jira_token.cursor(),
            focused: model.focus == 0,
            cursor_visible: model.cursor_visible(),
            masked: true,
            can_retain_secret: model.can_retain_jira_token,
            invalid: model
                .error
                .as_deref()
                .is_some_and(|error| error.contains("Atlassian API token")),
        },
    );
    render_token_url_fallback(
        frame,
        raw_url,
        &model.jira_instruction,
        &model.jira_url,
        model.jira_page_can_open,
        model.warning.is_some(),
    );
    frame.render_widget(
        SetupButton::new(
            "Connect Jira",
            model.focus == 1,
            model.jira_status,
            model.pending_symbol(),
        ),
        status,
    );
    render_feedback(frame, feedback, model);
}

fn render_token_url_fallback(
    frame: &mut Frame<'_>,
    area: Rect,
    instruction: &str,
    url: &str,
    can_open: bool,
    open_failed: bool,
) {
    if !can_open || open_failed {
        frame.render_widget(
            Paragraph::new(Text::from(vec![
                Line::from(terminal_text::escape(instruction)).dim(),
                Line::from(terminal_text::escape(url)).underlined(),
            ]))
            .wrap(Wrap { trim: false }),
            area,
        );
    }
}

fn render_save(frame: &mut Frame<'_>, area: Rect, model: &OnboardingModel) {
    let [intro, _, manifest, _, question, _, action, edit, _, feedback] = Layout::vertical([
        Constraint::Length(2),
        Constraint::Length(1),
        Constraint::Length(5),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(3),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Fill(1),
    ])
    .areas(area);
    frame.render_widget(
        Paragraph::new(Text::from(vec![
            Line::from("Review Jira setup").bold(),
            Line::from("Graduate will save this connection.").dim(),
        ])),
        intro,
    );
    frame.render_widget(
        Paragraph::new(Text::from(vec![
            Line::from(vec![
                ratatui::text::Span::styled(
                    format!("{:<REVIEW_LABEL_WIDTH$}", "Field"),
                    Palette::text().bold(),
                ),
                ratatui::text::Span::styled("Value", Palette::text().bold()),
            ]),
            Line::styled("─".repeat(usize::from(manifest.width)), Palette::muted()),
            detail_line("Site", &terminal_text::escape(model.hostname.value())),
            detail_line("Account", &terminal_text::escape(model.email.value())),
            detail_line("Identity", &terminal_text::escape(&model.display_name)),
        ])),
        manifest,
    );
    frame.render_widget(Paragraph::new("Does this look right?").bold(), question);
    frame.render_widget(
        SetupButton::new(
            "Save configuration",
            true,
            ConnectionStatus::NotConnected,
            model.pending_symbol(),
        ),
        action,
    );
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            ratatui::text::Span::styled("J ", Palette::primary().bold()),
            ratatui::text::Span::styled("Change Jira account", Palette::muted()),
        ])),
        edit,
    );
    render_feedback(frame, feedback, model);
}

fn detail_line<'a>(label: &'static str, value: &'a str) -> Line<'a> {
    Line::from(vec![
        ratatui::text::Span::styled(format!("{label:<REVIEW_LABEL_WIDTH$}"), Palette::muted()),
        ratatui::text::Span::styled(value, Palette::text()),
    ])
}
