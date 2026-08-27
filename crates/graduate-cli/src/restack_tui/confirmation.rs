//! Explicit rewrite confirmation rendering.

use graduate::restack::RestackPlan;
use ratatui::layout::{Alignment, Rect};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};
use ratatui::Frame;

use super::render::short_oid;
use super::review::wrapped_text_height;
use super::review_details::resolution_summary;
use crate::terminal_text::escape;
use crate::theme::Palette;

pub(super) fn render_confirmation(frame: &mut Frame<'_>, area: Rect, plan: Option<&RestackPlan>) {
    let text = confirmation_text(plan);
    let text_height = wrapped_text_height(&text, area.width).min(usize::from(u16::MAX)) as u16;
    let panel_height = area.height.min(text_height.saturating_add(1));
    frame.render_widget(
        Paragraph::new(text)
            .block(
                Block::default()
                    .borders(Borders::TOP)
                    .border_style(Palette::warning())
                    .title(" PUBLISH REMOTE REWRITE "),
            )
            .wrap(Wrap { trim: false })
            .alignment(Alignment::Left),
        Rect::new(area.x, area.y, area.width, panel_height),
    );
}

pub(super) fn confirmation_text(plan: Option<&RestackPlan>) -> Text<'static> {
    plan.map_or_else(
        || Text::from("The reviewed plan is unavailable."),
        |plan| {
            const OMITTED_BRANCH_LIMIT: usize = 3;
            let retained = plan.selection.retained.len();
            let removed = plan.selection.removed.len();
            let retained_label = if retained == 1 { "feature" } else { "features" };
            let target = format!(
                "{}/{}",
                escape(&plan.snapshot.remote),
                escape(&plan.snapshot.environment)
            );
            let mut lines = vec![
                Line::from(vec![
                    Span::styled("Current tip     ", Palette::muted()),
                    Span::raw(format!("{target} @ ")),
                    Span::styled(
                        short_oid(&plan.snapshot.environment_tip),
                        Palette::warning(),
                    ),
                ]),
                Line::from(vec![
                    Span::styled("Reviewed tip    ", Palette::muted()),
                    Span::raw(format!("{target} @ ")),
                    Span::styled(short_oid(&plan.preview_commit), Palette::primary().bold()),
                ]),
                Line::from(vec![
                    Span::styled("Rewrite scope   ", Palette::muted()),
                    Span::raw(format!(
                        "rebuild {target} from {retained} retained {retained_label} · {removed} omitted · {}",
                        resolution_summary(plan),
                    )),
                ]),
                Line::from(""),
                Line::from(vec![
                    Span::styled("Remote guard    ", Palette::muted()),
                    Span::raw(format!(
                        "publish stops if {target} changed since review (exact lease)"
                    )),
                ]),
            ];
            let dropped = plan.orphaned_commits.len();
            if dropped > 0 {
                lines.push(Line::from(""));
                lines.push(Line::styled(
                    format!(
                        "Drops {dropped} {} that no retained branch contains.",
                        if dropped == 1 { "commit" } else { "commits" }
                    ),
                    Palette::warning().bold(),
                ));
            }
            if removed > 0 {
                lines.push(Line::from(""));
                lines.push(Line::styled(
                    "Omitted from the reviewed result",
                    Palette::warning().bold(),
                ));
                lines.extend(
                    plan.selection
                        .removed
                        .iter()
                        .take(OMITTED_BRANCH_LIMIT)
                        .map(|branch| {
                            Line::styled(
                                format!(
                                    "  –  {} @ {}",
                                    escape(&branch.name),
                                    short_oid(&branch.tip)
                                ),
                                Palette::warning(),
                            )
                        }),
                );
                if removed > OMITTED_BRANCH_LIMIT {
                    lines.push(Line::styled(
                        format!(
                            "  … and {} more; press Esc to review every omission",
                            removed.saturating_sub(OMITTED_BRANCH_LIMIT)
                        ),
                        Palette::muted(),
                    ));
                }
            }
            lines.extend([
                Line::from(""),
                Line::styled(
                    format!(
                        "This rewrites {target} history; collaborators tracking it must resync after publish."
                    ),
                    Palette::warning(),
                ),
                Line::styled(
                    "Feature branches and local work remain unchanged.",
                    Palette::muted(),
                ),
                Line::from(""),
                Line::styled(
                    format!("Press Ctrl+Y to replace {target} with this reviewed result."),
                    Palette::warning().bold(),
                ),
                Line::styled(
                    "Esc returns to Review; q abandons this plan without changing refs.",
                    Palette::muted(),
                ),
            ]);
            Text::from(lines)
        },
    )
}
