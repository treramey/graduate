//! Commit-age report modal rendering.

use ratatui::layout::{Constraint, HorizontalAlignment, Layout};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Padding, Paragraph, Row, Table};
use ratatui::Frame;

use super::render::modal_area;
use super::{DiffModel, SPACE_1X};
use crate::diff::{age_bucket_label, age_bucket_reading, share_percent};
use crate::shared::terminal_text;
use crate::shared::theme::Palette;

pub(super) fn render_age_report(frame: &mut Frame<'_>, model: &mut DiffModel) {
    let Some(age) = model.age_report.as_ref() else {
        return;
    };
    let outer = frame.area();
    frame
        .buffer_mut()
        .set_style(outer, Style::new().add_modifier(Modifier::DIM));
    let height = outer.height.saturating_sub(4).clamp(1, 34);
    let area = modal_area(outer, height);
    let card = Block::default()
        .borders(Borders::ALL)
        .border_style(Palette::muted())
        .style(Palette::overlay())
        .padding(Padding::new(SPACE_1X, SPACE_1X, 0, 0));
    let content = card.inner(area);
    let [title, context, ages, oldest_title, oldest, footer] = Layout::vertical([
        Constraint::Length(2),
        Constraint::Length(2),
        Constraint::Length(12),
        Constraint::Length(2),
        Constraint::Fill(1),
        Constraint::Length(2),
    ])
    .areas(content);
    let mut age_rows = age
        .buckets
        .iter()
        .map(|bucket| {
            Row::new([
                age_bucket_label(bucket.year),
                bucket.commits.to_string(),
                format!("{:.1}%", share_percent(bucket.commits, age.total_commits)),
                age_bucket_reading(age, bucket),
            ])
        })
        .collect::<Vec<_>>();
    age_rows.push(
        Row::new([
            "Written in last 90 days".to_owned(),
            age.last_90_days.commits.to_string(),
            format!(
                "{:.1}%",
                share_percent(age.last_90_days.commits, age.total_commits)
            ),
            "Genuinely in flight".to_owned(),
        ])
        .style(Palette::text().bold()),
    );
    age_rows.push(
        Row::new([
            "Older than one year".to_owned(),
            age.older_than_one_year.commits.to_string(),
            format!(
                "{:.1}%",
                share_percent(age.older_than_one_year.commits, age.total_commits)
            ),
            "Will not ship without a decision".to_owned(),
        ])
        .style(Palette::text().bold()),
    );
    let age_table = Table::new(
        age_rows,
        [
            Constraint::Length(24),
            Constraint::Length(10),
            Constraint::Length(8),
            Constraint::Min(24),
        ],
    )
    .header(Row::new(["WRITTEN IN", "COMMITS", "SHARE", "READING"]).style(Palette::muted().bold()))
    .column_spacing(1)
    .row_highlight_style(Palette::action_focus())
    .highlight_symbol("› ");
    if content.height < 22 {
        let [compact_title, compact_ages] =
            Layout::vertical([Constraint::Length(1), Constraint::Fill(1)]).areas(content);
        frame.render_widget(Clear, area);
        frame.render_widget(card, area);
        frame.render_widget(
            Paragraph::new(Line::styled(
                "The age of unshipped work — the decisive measure",
                Palette::primary().bold(),
            ))
            .alignment(HorizontalAlignment::Center),
            compact_title,
        );
        frame.render_stateful_widget(age_table, compact_ages, &mut model.age_list_state);
        return;
    }
    let oldest_rows = age.oldest_branches.iter().take(6).map(|branch| {
        Row::new([
            terminal_text::escape(&branch.branch),
            branch.commits.to_string(),
            branch.oldest.to_string(),
            branch.newest.to_string(),
        ])
    });
    let oldest_table = Table::new(
        oldest_rows,
        [
            Constraint::Min(28),
            Constraint::Length(8),
            Constraint::Length(10),
            Constraint::Length(10),
        ],
    )
    .header(Row::new(["BRANCH", "COMMITS", "OLDEST", "NEWEST"]).style(Palette::muted().bold()))
    .column_spacing(1);
    let oldest_year = age
        .oldest_year()
        .map_or_else(String::new, |year| year.to_string());

    frame.render_widget(Clear, area);
    frame.render_widget(card, area);
    frame.render_widget(
        Paragraph::new(Line::styled(
            "The age of unshipped work — the decisive measure",
            Palette::primary().bold(),
        ))
        .alignment(HorizontalAlignment::Center),
        title,
    );
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::raw("All "),
            Span::styled(age.total_commits.to_string(), Palette::text().bold()),
            Span::raw(" unique authored commits in "),
            Span::styled(
                terminal_text::escape(&model.environment),
                Palette::text().bold(),
            ),
            Span::raw(" but not "),
            Span::styled(terminal_text::escape(&model.main), Palette::text().bold()),
            Span::styled(format!("  ·  as of {}", age.as_of), Palette::muted()),
        ]))
        .alignment(HorizontalAlignment::Center),
        context,
    );
    frame.render_stateful_widget(age_table, ages, &mut model.age_list_state);
    let oldest_heading = if age.oldest_branches.len() > 6 {
        format!(
            "Top 6 of {} branches carrying {oldest_year} commits",
            age.oldest_branches.len()
        )
    } else {
        format!("Branches carrying {oldest_year} commits")
    };
    frame.render_widget(
        Paragraph::new(Line::styled(oldest_heading, Palette::primary().bold())),
        oldest_title,
    );
    frame.render_widget(oldest_table, oldest);
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("↑/↓", Palette::primary()),
            Span::raw(" scroll   "),
            Span::styled("Esc/a", Palette::primary()),
            Span::raw(" close age report"),
        ]))
        .alignment(HorizontalAlignment::Center),
        footer,
    );
}
