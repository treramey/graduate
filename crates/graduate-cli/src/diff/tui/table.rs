//! Branch table rendering.

use graduate::promotion::{jira_issue_is_closed, JiraIssueState, JiraIssueSummary};
use ratatui::layout::{Constraint, HorizontalAlignment, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::Line;
use ratatui::widgets::{Cell, Row, Table};
use ratatui::Frame;

use super::{DiffModel, SortKey};
use crate::shared::terminal_text;
use crate::shared::theme::Palette;

pub(super) fn render_table(
    frame: &mut Frame<'_>,
    area: Rect,
    model: &mut DiffModel,
    show_jira: bool,
) {
    let sort_label = |label: &str, key: SortKey| {
        if model.sort == key {
            format!("{label}{}", key.indicator())
        } else {
            label.to_owned()
        }
    };
    let mut labels = vec![
        (
            sort_label("BRANCH", SortKey::Branch),
            HorizontalAlignment::Left,
        ),
        (
            sort_label("STARTED", SortKey::Started),
            HorizontalAlignment::Left,
        ),
        (sort_label("LAST", SortKey::Last), HorizontalAlignment::Left),
        (
            sort_label("AHEAD", SortKey::Ahead),
            HorizontalAlignment::Right,
        ),
        ("UNMERGED".to_owned(), HorizontalAlignment::Right),
    ];
    if show_jira {
        labels.push(("JIRA".to_owned(), HorizontalAlignment::Left));
        labels.push(("STATUS".to_owned(), HorizontalAlignment::Left));
    }
    let header = Row::new(
        labels
            .into_iter()
            .map(|(label, alignment)| Cell::from(Line::from(label).alignment(alignment))),
    )
    .style(Palette::muted().add_modifier(Modifier::BOLD))
    .bottom_margin(1);
    let rows = model.rows.iter().map(|row| {
        let flagged = row
            .report
            .as_ref()
            .is_some_and(|report| !report.merged_environments.is_empty());
        let branch = terminal_text::escape(&row.branch);
        let branch = if flagged {
            format!("{branch} ⚠")
        } else {
            branch
        };
        let mut cells = match &row.report {
            Some(report) => vec![
                Cell::from(branch),
                Cell::from(report.started.clone()),
                Cell::from(report.last.clone()),
                Cell::from(
                    Line::from(report.ahead.to_string()).alignment(HorizontalAlignment::Right),
                ),
                Cell::from(
                    Line::from(unmerged_label(report.unmerged_ahead))
                        .alignment(HorizontalAlignment::Right),
                ),
            ],
            None => vec![
                Cell::from(branch),
                placeholder_cell(),
                placeholder_cell(),
                Cell::from(
                    Line::styled("…", Palette::muted()).alignment(HorizontalAlignment::Right),
                ),
                Cell::from(
                    Line::styled("…", Palette::muted()).alignment(HorizontalAlignment::Right),
                ),
            ],
        };
        if show_jira {
            match &row.report {
                Some(report) => {
                    let (key, status) = jira_cells(&report.jira, flagged);
                    cells.push(key);
                    cells.push(status);
                }
                None => {
                    cells.push(placeholder_cell());
                    cells.push(Cell::from(Line::styled("measuring", Palette::muted())));
                }
            }
        }
        let cells = Row::new(cells);
        if flagged {
            cells.style(Palette::error())
        } else {
            cells
        }
    });
    let longest_branch = model
        .rows
        .iter()
        .map(|row| terminal_text::escape(&row.branch).chars().count())
        .max()
        .unwrap_or(0)
        .saturating_add(2);
    let branch_width = u16::try_from(longest_branch).unwrap_or(u16::MAX);
    let widths = if show_jira {
        vec![
            Constraint::Length(branch_width.max(24)),
            Constraint::Length(10),
            Constraint::Length(10),
            Constraint::Length(7),
            Constraint::Length(8),
            Constraint::Length(12),
            Constraint::Length(16),
        ]
    } else {
        vec![
            Constraint::Length(branch_width.max(20)),
            Constraint::Length(10),
            Constraint::Length(10),
            Constraint::Length(7),
            Constraint::Length(8),
        ]
    };
    let table = Table::new(rows, widths)
        .column_spacing(2)
        .header(header)
        .row_highlight_style(Palette::action_focus())
        .highlight_symbol("› ");
    frame.render_stateful_widget(table, area, &mut model.table_state);
}

/// Hide a zero so fully merged branches stay visually quiet.
fn unmerged_label(unmerged_ahead: usize) -> String {
    if unmerged_ahead == 0 {
        String::new()
    } else {
        unmerged_ahead.to_string()
    }
}

fn placeholder_cell() -> Cell<'static> {
    Cell::from(Line::styled("…", Palette::muted()))
}

fn jira_cells(state: &JiraIssueState, flagged: bool) -> (Cell<'static>, Cell<'static>) {
    // Only a Jira-validated ticket key may appear in the ticket column.
    let key = match state {
        JiraIssueState::Loaded(issue) => terminal_text::escape(&issue.key),
        _ => String::new(),
    };
    let (status, style) = match state {
        JiraIssueState::NoTicket | JiraIssueState::NotFound { .. } => {
            ("not found".to_owned(), Palette::muted())
        }
        JiraIssueState::NotConfigured { .. } => ("not configured".to_owned(), Palette::warning()),
        JiraIssueState::Loading { .. } => ("loading…".to_owned(), Palette::muted()),
        JiraIssueState::Loaded(issue) => {
            let status = terminal_text::escape(&issue.status);
            let style = jira_status_style(issue);
            (status, style)
        }
        JiraIssueState::Failed { .. } => ("Jira error".to_owned(), Palette::error()),
    };
    if flagged {
        // The row's warning color must own every cell of a flagged branch.
        (Cell::from(key), Cell::from(status))
    } else {
        (
            Cell::from(Line::styled(key, Palette::muted())),
            Cell::from(Line::styled(status, style)),
        )
    }
}

fn jira_status_style(issue: &JiraIssueSummary) -> Style {
    if issue.status.to_ascii_lowercase().contains("cancel") {
        Palette::muted()
    } else if jira_issue_is_closed(&issue.status, issue.status_category.as_deref()) {
        Palette::success()
    } else {
        Palette::text()
    }
}
