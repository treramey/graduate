//! Selected-branch inspector and detail rendering.

use graduate::promotion::{JiraIssueState, PromotionBranch};
use ratatui::layout::{Constraint, HorizontalAlignment, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Padding, Paragraph, Wrap};
use ratatui::Frame;

use super::render::report_metadata;
use super::{DiffModel, SPACE_1X, SPACE_2X};
use crate::shared::terminal_text;
use crate::shared::theme::Palette;

pub(super) fn render_inspector(frame: &mut Frame<'_>, area: Rect, model: &DiffModel) {
    let selected_row = model.rows.get(model.selected);
    let selected = selected_row.and_then(|row| row.report.as_ref());
    let branch = selected_row.map_or("No branch selected", |row| row.branch.as_str());
    let position = if model.rows.is_empty() {
        "0 of 0".to_owned()
    } else {
        format!("{} of {}", model.selected + 1, model.rows.len())
    };
    let compact = area.height < 15;
    let padding = if compact {
        Padding::new(SPACE_1X, SPACE_1X, 0, 0)
    } else {
        Padding::uniform(SPACE_1X)
    };
    let card = Block::default()
        .borders(Borders::ALL)
        .border_style(Palette::muted())
        .title(Line::styled(
            format!(" {} ", terminal_text::escape(branch)),
            Palette::primary().bold(),
        ))
        .title(
            Line::styled(format!(" {position} "), Palette::muted())
                .alignment(HorizontalAlignment::Right),
        )
        .padding(padding);
    let content = card.inner(area);
    let mut lines = Vec::new();
    lines.extend(inspector_status(selected));
    if let Some(report) = selected {
        if !compact {
            lines.push(Line::default());
            lines.push(Line::styled("────────────────", Palette::muted()));
            lines.push(Line::default());
        }
        lines.extend(report_metadata(report));
    }
    frame.render_widget(card, area);
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: true }), content);
}

fn inspector_status(selected: Option<&PromotionBranch>) -> Vec<Line<'static>> {
    let Some(report) = selected else {
        return vec![detail_line("Status", "Measuring branch history…")];
    };
    match &report.jira {
        JiraIssueState::Loaded(issue) => vec![
            detail_line("Jira", &issue.key),
            detail_line("Summary", &issue.summary),
            detail_line("Status", &issue.status),
            detail_line(
                "Assignee",
                issue.assignee.as_deref().unwrap_or("Unassigned"),
            ),
            detail_line(
                "Versions",
                &if issue.fix_versions.is_empty() {
                    "None".to_owned()
                } else {
                    issue.fix_versions.join(", ")
                },
            ),
        ],
        JiraIssueState::Failed { message, .. } => vec![
            Line::from(vec![
                detail_label("Jira"),
                Span::styled("Error", Palette::error()),
            ]),
            detail_line("Reason", message),
        ],
        JiraIssueState::NotConfigured { key } => vec![
            detail_line("Jira", key),
            Line::from(vec![
                detail_label("State"),
                Span::styled("Not configured", Palette::warning()),
            ]),
            detail_line("Next", "gd auth setup jira"),
        ],
        JiraIssueState::Loading { key } => vec![detail_line("Jira", &format!("{key} · Loading…"))],
        JiraIssueState::NotFound { key } => vec![
            detail_line("Jira", key),
            Line::from(vec![
                detail_label("State"),
                Span::styled("Not found", Palette::muted()),
            ]),
            detail_line("Status", "This Jira ticket was not found"),
        ],
        JiraIssueState::NoTicket => vec![detail_line("Jira", "No ticket key in branch name")],
    }
}

pub(super) fn render_details(frame: &mut Frame<'_>, area: Rect, model: &DiffModel) {
    let selected_row = model.rows.get(model.selected);
    let selected = selected_row.and_then(|row| row.report.as_ref());
    let branch = selected_row.map_or("No branch selected", |row| row.branch.as_str());
    let position = if model.rows.is_empty() {
        "0 of 0".to_owned()
    } else {
        format!("{} of {}", model.selected + 1, model.rows.len())
    };
    let (status_lines, metadata_lines) = match selected {
        Some(report) => match &report.jira {
            JiraIssueState::Loaded(issue) => (
                vec![
                    detail_line("Jira", &issue.key),
                    detail_line("Summary", &issue.summary),
                    detail_line("Status", &issue.status),
                    detail_line(
                        "Versions",
                        &if issue.fix_versions.is_empty() {
                            "None".to_owned()
                        } else {
                            issue.fix_versions.join(", ")
                        },
                    ),
                ],
                vec![
                    detail_line(
                        "Assignee",
                        issue.assignee.as_deref().unwrap_or("Unassigned"),
                    ),
                    detail_line("Author", &report.last_author),
                    detail_line("Commits", &report.ahead.to_string()),
                    detail_line("Updated", &report.last),
                ],
            ),
            JiraIssueState::Failed { message, .. } => (
                vec![
                    Line::from(vec![
                        detail_label("Jira"),
                        Span::styled("Error", Palette::error()),
                    ]),
                    detail_line("Reason", message),
                ],
                report_metadata(report),
            ),
            JiraIssueState::NotConfigured { key } => (
                vec![
                    Line::from(vec![
                        detail_label("Jira"),
                        Span::styled(
                            format!("{} · Not configured", terminal_text::escape(key)),
                            Palette::warning(),
                        ),
                    ]),
                    detail_line("Next", "gd auth setup jira"),
                ],
                report_metadata(report),
            ),
            JiraIssueState::Loading { key } => (
                vec![detail_line("Jira", &format!("{key} · Loading…"))],
                report_metadata(report),
            ),
            JiraIssueState::NotFound { key } => (
                vec![
                    Line::from(vec![
                        detail_label("Jira"),
                        Span::styled(
                            format!("{} · Not found", terminal_text::escape(key)),
                            Palette::muted(),
                        ),
                    ]),
                    detail_line("Status", "This Jira ticket was not found"),
                ],
                report_metadata(report),
            ),
            JiraIssueState::NoTicket => (
                vec![detail_line("Jira", "No ticket key in branch name")],
                report_metadata(report),
            ),
        },
        None => (
            vec![detail_line("Status", "Measuring branch history…")],
            Vec::new(),
        ),
    };
    let branch_title =
        Line::from(format!(" {} ", terminal_text::escape(branch))).style(Palette::primary().bold());
    let position_title = Line::from(format!(" {position} ")).style(Palette::muted());
    let card = Block::default()
        .borders(Borders::ALL)
        .border_style(Palette::muted())
        .title(branch_title)
        .title(position_title.alignment(HorizontalAlignment::Right))
        .padding(Padding::uniform(SPACE_1X));
    let content = card.inner(area);
    let [status, metadata] =
        Layout::horizontal([Constraint::Percentage(55), Constraint::Fill(1)]).areas(content);
    frame.render_widget(card, area);
    frame.render_widget(
        Paragraph::new(status_lines).wrap(Wrap { trim: true }),
        status,
    );
    frame.render_widget(
        Paragraph::new(metadata_lines)
            .block(
                Block::default()
                    .borders(Borders::LEFT)
                    .border_style(Palette::muted())
                    .padding(Padding::new(SPACE_2X, 0, 0, 0)),
            )
            .wrap(Wrap { trim: true }),
        metadata,
    );
}

fn detail_label(label: &str) -> Span<'static> {
    Span::styled(format!("{label}  "), Palette::muted())
}

pub(super) fn detail_line(label: &str, value: &str) -> Line<'static> {
    Line::from(vec![
        detail_label(label),
        Span::raw(terminal_text::escape(value)),
    ])
}
