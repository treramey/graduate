//! Commit history sheet rendering.

use ratatui::layout::{Constraint, HorizontalAlignment, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Padding, Paragraph, Row, Table};
use ratatui::Frame;

use super::render::modal_area;
use super::{DiffModel, MIN_HISTORY_HEIGHT, SPACE_1X};
use crate::terminal_text;
use crate::theme::Palette;

pub(super) fn render_history(frame: &mut Frame<'_>, model: &mut DiffModel) {
    let Some(report) = model
        .rows
        .get(model.selected)
        .and_then(|row| row.report.as_ref())
    else {
        return;
    };
    let outer = frame.area();
    frame
        .buffer_mut()
        .set_style(outer, Style::new().add_modifier(Modifier::DIM));
    let desired_height = u16::try_from(report.commits.len())
        .unwrap_or(u16::MAX)
        .saturating_add(13);
    let height = desired_height
        .min(24)
        .min(outer.height.saturating_sub(4))
        .max(MIN_HISTORY_HEIGHT.min(outer.height));
    let area = modal_area(outer, height);
    let rows = report.commits.iter().map(|commit| {
        Row::new([
            terminal_text::escape(&commit.short_id),
            terminal_text::escape(&commit.subject),
            terminal_text::escape(&commit.author),
            commit.date.clone(),
        ])
    });
    let count = report.commits.len();
    let noun = if count == 1 { "commit" } else { "commits" };
    let summary = Line::from(vec![
        Span::styled(
            terminal_text::escape(&report.branch),
            Palette::text().bold(),
        ),
        Span::styled(
            format!("  ·  {count} {noun}  ·  newest first"),
            Palette::muted(),
        ),
    ]);
    let position = if count == 0 {
        "0 of 0".to_owned()
    } else {
        format!("{} of {count}", model.history_selected + 1)
    };
    let card = Block::default()
        .borders(Borders::ALL)
        .border_style(Palette::muted())
        .style(Palette::overlay())
        .padding(Padding::new(SPACE_1X, SPACE_1X, 0, 0));
    let content = card.inner(area);
    let [title, context, headings, top_divider, commits, _position_margin_top, bottom_divider, _footer_margin_top, footer, _footer_margin_bottom] =
        Layout::vertical([
            Constraint::Length(2),
            Constraint::Length(2),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Fill(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .areas(content);
    let widths = [
        Constraint::Length(8),
        Constraint::Min(20),
        Constraint::Length(15),
        Constraint::Length(10),
    ];
    let heading_widths = [
        Constraint::Length(1),
        Constraint::Length(8),
        Constraint::Min(20),
        Constraint::Length(15),
        Constraint::Length(10),
    ];
    let heading = Table::new(std::iter::empty::<Row<'static>>(), heading_widths)
        .header(Row::new(["", "SHA", "SUBJECT", "AUTHOR", "DATE"]).style(Palette::muted().bold()));
    let table = Table::new(rows, widths)
        .row_highlight_style(Palette::action_focus())
        .highlight_symbol("› ");
    frame.render_widget(Clear, area);
    frame.render_widget(card, area);
    frame.render_widget(
        Paragraph::new(Line::styled(
            format!("Commits ahead of {}", terminal_text::escape(&model.main)),
            Palette::primary().bold(),
        ))
        .alignment(HorizontalAlignment::Center),
        title,
    );
    frame.render_widget(
        Paragraph::new(summary).alignment(HorizontalAlignment::Center),
        context,
    );
    frame.render_widget(heading, headings);
    render_history_divider(frame, top_divider);
    frame.render_stateful_widget(table, commits, &mut model.history_list_state);
    render_history_position_divider(frame, bottom_divider, &position);
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("↑/↓", Palette::primary()),
            Span::raw(" move   "),
            Span::styled("Esc/h", Palette::primary()),
            Span::raw(" close history"),
        ]))
        .alignment(HorizontalAlignment::Center),
        footer,
    );
}

fn render_history_divider(frame: &mut Frame<'_>, area: Rect) {
    frame.render_widget(
        Block::default()
            .borders(Borders::TOP)
            .border_style(Palette::muted()),
        area,
    );
}

fn render_history_position_divider(frame: &mut Frame<'_>, area: Rect, position: &str) {
    frame.render_widget(
        Block::default()
            .borders(Borders::TOP)
            .border_style(Palette::muted())
            .title(Line::from(format!(" {position} ")).alignment(HorizontalAlignment::Center)),
        area,
    );
}
