//! Feature checklist rendering.

use graduate::promotion::jira_key_from_branch;
use graduate::restack::{InventoryMode, RestackInteraction};
use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{List, ListItem, Paragraph, Wrap};
use ratatui::Frame;

use super::keys::{carried_by, filtered_feature_indices};
use super::render::{pad_text, short_oid, truncate_text};
use super::RestackViewState;
use crate::shared::terminal_text::escape;
use crate::shared::theme::Palette;

pub(super) fn render_selection(
    frame: &mut Frame<'_>,
    area: Rect,
    interaction: &RestackInteraction,
    rejection: Option<&str>,
    view: &mut RestackViewState,
) -> bool {
    let snapshot = interaction.snapshot();
    let retained_count = snapshot
        .features
        .iter()
        .enumerate()
        .filter(|(index, _)| interaction.is_retained(*index))
        .count();
    let removed_count = snapshot.features.len().saturating_sub(retained_count);
    let visible = filtered_feature_indices(interaction, &view.filter);
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("SELECT FEATURES", Palette::primary().bold()),
            Span::styled(
                format!(
                    "  {}/{}",
                    escape(&snapshot.remote),
                    escape(&snapshot.environment)
                ),
                Palette::muted(),
            ),
        ])),
        Rect::new(area.x, area.y, area.width, 1),
    );
    let summary_style = if removed_count == 0 {
        Palette::muted()
    } else {
        Palette::warning()
    };
    let filter_summary = if !view.filter.is_empty() && area.width >= 76 {
        format!(" · {}/{} shown", visible.len(), snapshot.features.len())
    } else {
        String::new()
    };
    frame.render_widget(
        Paragraph::new(Line::styled(
            format!("{retained_count} retained · {removed_count} removed{filter_summary}"),
            summary_style,
        ))
        .alignment(Alignment::Right),
        Rect::new(area.x, area.y, area.width, 1),
    );
    let inventory_mode = interaction.inventory_mode() == InventoryMode::Reachability;
    if inventory_mode {
        let dropped = interaction.orphaned_commit_count();
        let dropped_text = match dropped {
            0 => "no commits dropped".to_owned(),
            1 => "1 commit will be dropped".to_owned(),
            count => format!("{count} commits will be dropped"),
        };
        let banner = if area.width >= 100 {
            "Inventory mode: reachability · oldest tip first · no reused resolutions"
        } else {
            "Inventory mode · no rerere"
        };
        let banner_row = Rect::new(area.x, area.y.saturating_add(1), area.width, 1);
        let dropped_width = u16::try_from(dropped_text.chars().count()).unwrap_or(u16::MAX);
        let banner_width = area.width.saturating_sub(dropped_width.saturating_add(2));
        frame.render_widget(
            Paragraph::new(Line::styled(
                truncate_text(banner, usize::from(banner_width)),
                Palette::primary(),
            )),
            Rect::new(banner_row.x, banner_row.y, banner_width, 1),
        );
        frame.render_widget(
            Paragraph::new(Line::styled(
                dropped_text,
                if dropped == 0 {
                    Palette::muted()
                } else {
                    Palette::warning()
                },
            ))
            .alignment(Alignment::Right),
            banner_row,
        );
    }
    let row_width = usize::from(area.width.saturating_sub(4));
    let wide_rows = area.width >= 96;
    let branch_width = if wide_rows {
        row_width.saturating_sub(63).clamp(16, 38)
    } else {
        row_width.saturating_sub(17).clamp(16, 46)
    };
    let mut item_features = Vec::new();
    let mut items = Vec::new();
    for (index, feature) in visible.iter().filter_map(|index| {
        snapshot
            .features
            .get(*index)
            .map(|feature| (*index, feature))
    }) {
        item_features.push(Some(index));
        items.push(feature_item(FeatureRow {
            interaction,
            index,
            feature,
            row_width,
            wide_rows,
            branch_width,
        }));
        if let Some(tainted) = interaction.tainted_feature(index) {
            item_features.push(None);
            items.push(ListItem::new(Line::from(vec![
                Span::styled("      ↳ tainted  ", Palette::warning()),
                Span::styled(
                    absorbed_merges_text(tainted.absorbed_merges.len()),
                    Palette::muted(),
                ),
            ])));
        }
        for carried in carried_by(interaction, &feature.name) {
            let also = carried
                .carriers
                .iter()
                .skip(1)
                .map(|carrier| escape(carrier))
                .collect::<Vec<_>>();
            let suffix = if also.is_empty() {
                String::new()
            } else {
                format!("  (also via {})", also.join(", "))
            };
            item_features.push(None);
            items.push(ListItem::new(Line::from(vec![
                Span::styled("      ↳ carried  ", Palette::muted()),
                Span::styled(
                    truncate_text(&escape(&carried.name), branch_width),
                    Palette::muted(),
                ),
                Span::styled(
                    format!("  {}{suffix}", short_oid(&carried.tip)),
                    Palette::muted(),
                ),
            ])));
        }
    }
    let selected = item_features
        .iter()
        .position(|index| *index == Some(interaction.cursor()));
    view.feature_list.select(selected);
    let selected_context = selection_context(interaction, view.show_shortcuts);
    let context_height = rejection.map_or_else(
        || {
            u16::try_from(
                Line::raw(&selected_context)
                    .width()
                    .div_ceil(usize::from(area.width).max(1)),
            )
            .unwrap_or(2)
            .max(1)
        },
        |message| {
            u16::try_from(
                Line::raw(message)
                    .width()
                    .div_ceil(usize::from(area.width).max(1)),
            )
            .unwrap_or(2)
            .max(1)
        },
    );
    let list_height = area
        .height
        .saturating_sub(2_u16.saturating_add(context_height));
    let list_area = Rect::new(area.x, area.y.saturating_add(2), area.width, list_height);
    if items.is_empty() {
        frame.render_widget(
            Paragraph::new(Line::styled(
                format!("No branches match “{}”.", escape(&view.filter)),
                Palette::muted(),
            ))
            .alignment(Alignment::Center),
            list_area,
        );
    } else {
        frame.render_stateful_widget(
            List::new(items)
                .highlight_style(
                    Style::default()
                        .bg(Color::DarkGray)
                        .add_modifier(Modifier::BOLD),
                )
                .highlight_symbol("› ")
                .scroll_padding(1),
            list_area,
            &mut view.feature_list,
        );
    }
    let context_y = list_area.bottom();
    let (context, style) = rejection.map_or_else(
        || (selected_context, Palette::muted()),
        |message| (message.to_owned(), Palette::error()),
    );
    frame.render_widget(
        Paragraph::new(context)
            .style(style)
            .wrap(Wrap { trim: true }),
        Rect::new(
            area.x,
            context_y,
            area.width,
            context_height.min(area.bottom().saturating_sub(context_y)),
        ),
    );
    let carried_rows = item_features.iter().filter(|index| index.is_none()).count();
    visible
        .len()
        .saturating_mul(if wide_rows { 1 } else { 2 })
        .saturating_add(carried_rows)
        > usize::from(list_height)
}

struct FeatureRow<'a> {
    interaction: &'a RestackInteraction,
    index: usize,
    feature: &'a graduate::restack::ExplicitFeature,
    row_width: usize,
    wide_rows: bool,
    branch_width: usize,
}

fn feature_item(row: FeatureRow<'_>) -> ListItem<'static> {
    let FeatureRow {
        interaction,
        index,
        feature,
        row_width,
        wide_rows,
        branch_width,
    } = row;

    let retained = interaction.is_retained(index);
    let locked = !interaction.retained_dependents(index).is_empty();
    let keep = if retained { "✓" } else { "–" };
    let lock = if locked { "◆" } else { " " };
    let keep_style = if retained {
        Palette::success()
    } else {
        Palette::warning()
    };
    let jira = jira_key_from_branch(&feature.name).unwrap_or_else(|| "—".to_owned());
    let history = if feature.historical_merges.is_empty() {
        "—"
    } else {
        "available"
    };
    let history_style = if feature.historical_merges.is_empty() {
        Palette::muted()
    } else {
        Palette::success()
    };
    let branch = pad_text(
        &truncate_text(&escape(&feature.name), branch_width),
        branch_width,
    );
    let branch_style = if retained {
        Palette::text().bold()
    } else {
        Palette::muted()
    };
    let left_meta = format!("    #{} · {}", index + 1, short_oid(&feature.tip));
    let required = if locked { "  ◆ required" } else { "" };
    let right_width =
        "history: ".chars().count() + history.chars().count() + required.chars().count();
    let gap = row_width
        .saturating_sub(left_meta.chars().count() + right_width)
        .max(1);
    let first_line = Line::from(vec![
        Span::styled(keep, keep_style.bold()),
        Span::styled(lock, Palette::warning()),
        Span::raw("  "),
        Span::styled(branch.clone(), branch_style),
        Span::styled(format!(" {jira:>12}"), Palette::muted()),
    ]);
    if wide_rows {
        ListItem::new(Line::from(vec![
            Span::styled(keep, keep_style.bold()),
            Span::styled(lock, Palette::warning()),
            Span::raw(format!("  #{}  ", index + 1)),
            Span::styled(branch, branch_style),
            Span::styled(format!("  {}", short_oid(&feature.tip)), Palette::muted()),
            Span::styled(format!("  {jira:>12}"), Palette::muted()),
            Span::styled("  history: ", Palette::muted()),
            Span::styled(history, history_style),
            Span::styled(required, Palette::warning()),
        ]))
    } else {
        ListItem::new(Text::from(vec![
            first_line,
            Line::from(vec![
                Span::styled(left_meta, Palette::muted()),
                Span::raw(" ".repeat(gap)),
                Span::styled("history: ", Palette::muted()),
                Span::styled(history, history_style),
                Span::styled(required, Palette::warning()),
            ]),
        ]))
    }
}

pub(super) fn absorbed_merges_text(count: usize) -> String {
    if count == 1 {
        "1 environment merge absorbed".to_owned()
    } else {
        format!("{count} environment merges absorbed")
    }
}

fn selection_context(interaction: &RestackInteraction, show_shortcuts: bool) -> String {
    if let Some(tainted) = interaction.tainted_feature(interaction.cursor()) {
        return format!(
            "↳ Tainted: {} merged {} into itself. Recreate it from {} and cherry-pick your commits.",
            escape(&tainted.name),
            escape(&interaction.snapshot().environment),
            escape(&interaction.snapshot().main)
        );
    }
    let dependents = interaction.retained_dependents(interaction.cursor());
    if !dependents.is_empty() {
        return format!(
            "◆ Required by {}. Remove those retained features first.",
            dependents
                .iter()
                .map(|dependent| escape(dependent))
                .collect::<Vec<_>>()
                .join(", ")
        );
    }
    if show_shortcuts {
        return "Shortcuts: a keep all · x remove all · Home/End first/last · PgUp/PgDn page · q/Ctrl-C cancel"
            .to_owned();
    }
    if interaction.inventory_mode() == InventoryMode::Reachability {
        return "✓ retained · – removed · ◆ dependency · ↳ carried · ↳ tainted".to_owned();
    }
    "✓ retained · – removed · ◆ retained dependency · ↳ tainted · history = reusable resolution"
        .to_owned()
}
