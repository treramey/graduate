//! Frame layout, title, footer, and shared report text.

use graduate::promotion::{systemic_not_found, JiraIssueState, PromotionBranch};
use ratatui::layout::{Constraint, HorizontalAlignment, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use super::age_report::render_age_report;
use super::history::render_history;
use super::inspector::{detail_line, render_details, render_inspector};
use super::table::render_table;
use super::{
    DiffModel, MASTER_DETAIL_MAX_HEIGHT, MASTER_DETAIL_MIN_WIDTH, SPACE_1X, SPACE_2X, SPINNER,
};
use crate::shared::error::CliError;
use crate::shared::terminal::StderrTerminal;
use crate::shared::terminal_text;
use crate::shared::theme::{self, Palette};

pub(super) fn draw(terminal: &mut StderrTerminal, model: &mut DiffModel) -> Result<(), CliError> {
    terminal.terminal_mut().draw(|frame| render(frame, model))?;
    Ok(())
}

pub(super) fn render(frame: &mut Frame<'_>, model: &mut DiffModel) {
    let area = theme::constrain_content_width(frame.area());
    if frame.area().height <= MASTER_DETAIL_MAX_HEIGHT {
        let [_top_padding, title, _title_margin, main, _footer_margin, footer] =
            Layout::vertical([
                Constraint::Length(SPACE_1X),
                Constraint::Length(1),
                Constraint::Length(SPACE_1X),
                Constraint::Fill(1),
                Constraint::Length(SPACE_1X),
                Constraint::Length(1),
            ])
            .areas(area);
        render_title(frame, title, model, true);
        render_report(frame, main, model);
        render_footer(frame, footer, model);
        if model.age_report.is_some() {
            render_age_report(frame, model);
        } else if model.history_open {
            render_history(frame, model);
        }
        return;
    }
    let [_top_padding, header, _header_padding, title, main, _footer_margin, footer] =
        Layout::vertical([
            Constraint::Length(SPACE_2X),
            Constraint::Length(theme::GRADUATE_ART_HEIGHT),
            Constraint::Length(SPACE_1X),
            Constraint::Length(3),
            Constraint::Fill(1),
            Constraint::Length(SPACE_1X),
            Constraint::Length(3),
        ])
        .areas(area);
    theme::render_brand_header(frame, header);
    render_title(frame, title, model, false);
    render_report(frame, main, model);
    render_footer(frame, footer, model);
    if model.age_report.is_some() {
        render_age_report(frame, model);
    } else if model.history_open {
        render_history(frame, model);
    }
}

fn render_report(frame: &mut Frame<'_>, area: Rect, model: &mut DiffModel) {
    if area.width >= MASTER_DETAIL_MIN_WIDTH && frame.area().height <= MASTER_DETAIL_MAX_HEIGHT {
        let [table, _gutter, inspector] = Layout::horizontal([
            Constraint::Percentage(62),
            Constraint::Length(SPACE_2X),
            Constraint::Fill(1),
        ])
        .areas(area);
        render_table(frame, table, model, false);
        render_inspector(frame, inspector, model);
    } else {
        let [details, _gutter, table] = Layout::vertical([
            Constraint::Length(8),
            Constraint::Length(SPACE_1X),
            Constraint::Fill(1),
        ])
        .areas(area);
        render_details(frame, details, model);
        render_table(frame, table, model, true);
    }
}

pub(super) fn modal_area(outer: Rect, height: u16) -> Rect {
    let viewport = theme::constrain_content_width(outer);
    Rect::new(
        viewport.x,
        outer.y + outer.height.saturating_sub(height) / 2,
        viewport.width.max(1),
        height,
    )
}

fn render_title(frame: &mut Frame<'_>, area: Rect, model: &DiffModel, center_summary: bool) {
    let measured = model.rows.iter().filter(|row| row.report.is_some()).count();
    let pending_jira = model
        .rows
        .iter()
        .filter(|row| {
            row.report
                .as_ref()
                .is_some_and(|report| matches!(report.jira, JiraIssueState::Loading { .. }))
        })
        .count();
    let status = if model.finished {
        let synchronization = if model.inventory.behind_main.is_empty() {
            "in sync with main".to_owned()
        } else {
            format!("{} behind main", model.inventory.behind_main.len())
        };
        format!(
            "{} branches  ·  {synchronization}  ·  complete",
            model.rows.len()
        )
    } else if measured == model.rows.len() && pending_jira > 0 {
        format!(
            "{} loading Jira for {pending_jira} tickets",
            SPINNER[model.frame % SPINNER.len()]
        )
    } else {
        format!(
            "{} measuring {measured}/{}",
            SPINNER[model.frame % SPINNER.len()],
            model.rows.len()
        )
    };
    let summary = Line::from(vec![
        Span::raw("In "),
        Span::styled(
            terminal_text::escape(&model.environment),
            Palette::text().bold(),
        ),
        Span::raw(" but not "),
        Span::styled(terminal_text::escape(&model.main), Palette::text().bold()),
        Span::styled(format!("  ·  {status}"), Palette::muted()),
    ]);
    let summary = if center_summary {
        summary.alignment(HorizontalAlignment::Right)
    } else {
        summary
    };
    if center_summary {
        frame.render_widget(
            Paragraph::new(Line::styled("GRADUATE", Palette::primary().bold())),
            area,
        );
        frame.render_widget(Paragraph::new(summary), area);
    } else {
        frame.render_widget(
            Paragraph::new(vec![
                Line::styled("Promotion report", Palette::primary().bold()),
                summary,
            ]),
            area,
        );
    }
}

pub(super) fn report_metadata(report: &PromotionBranch) -> Vec<Line<'static>> {
    vec![
        detail_line("Author", &report.last_author),
        detail_line("Commits", &report.ahead.to_string()),
        detail_line("Updated", &report.last),
    ]
}

fn environment_merge_warning(model: &DiffModel) -> Option<String> {
    let report = model.rows.get(model.selected)?.report.as_ref()?;
    if report.merged_environments.is_empty() {
        return None;
    }
    let environments = report.merged_environments.join(", ");
    let verb = if report.merged_environments.len() == 1 {
        "has"
    } else {
        "have"
    };
    Some(format!(
        "⚠ {environments} {verb} been merged into this branch; its ahead count and dates include environment commits"
    ))
}

fn systemic_not_found_hint(model: &DiffModel) -> Option<String> {
    if !model.finished {
        return None;
    }
    let summary = systemic_not_found(
        model
            .rows
            .iter()
            .filter_map(|row| row.report.as_ref().map(|report| &report.jira)),
    )?;
    Some(format!(
        "⚠ {} of {} ticket lookups returned not found; check your Jira site and project access",
        summary.not_found, summary.resolved
    ))
}

fn render_footer(frame: &mut Frame<'_>, area: Rect, model: &DiffModel) {
    let help = if let Some(warning) = &model.warning {
        Line::styled(terminal_text::escape(warning), Palette::warning())
    } else if let Some(warning) = environment_merge_warning(model) {
        Line::styled(terminal_text::escape(&warning), Palette::error())
    } else if let Some(hint) = systemic_not_found_hint(model) {
        Line::styled(terminal_text::escape(&hint), Palette::warning())
    } else {
        Line::from(vec![
            Span::styled("↑/↓", Palette::primary()),
            Span::raw(" move   "),
            Span::styled("o", Palette::primary()),
            Span::raw(" open Jira   "),
            Span::styled("h", Palette::primary()),
            Span::raw(" git history   "),
            Span::styled("a", Palette::primary()),
            Span::raw(" age report   "),
            Span::styled("s", Palette::primary()),
            Span::raw(" sort   "),
            Span::styled("q", Palette::primary()),
            Span::raw(" close"),
        ])
    };
    frame.render_widget(Paragraph::new(help), area);
}
