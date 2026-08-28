//! Plan review rendering.

use graduate::restack::{InventoryMode, RestackPlan};
use ratatui::layout::Rect;
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Paragraph, Wrap};
use ratatui::Frame;

use super::render::{pad_text, short_oid, truncate_text};
use super::review_details::{dropped_summary, resolution_summary, technical_detail_lines};
use super::selection::absorbed_merges_text;
use crate::shared::terminal_text::escape;
use crate::shared::theme::Palette;

pub(super) fn render_review(
    frame: &mut Frame<'_>,
    area: Rect,
    plan: Option<&RestackPlan>,
    scroll: usize,
    show_details: bool,
) -> bool {
    let text = plan.map_or_else(
        || Text::from("The reviewed plan is unavailable."),
        |plan| review_text(plan, show_details),
    );
    let line_count = wrapped_text_height(&text, area.width);
    let max_scroll = line_count.saturating_sub(usize::from(area.height));
    let scroll = if scroll > max_scroll {
        let distance_from_end = usize::MAX.saturating_sub(scroll);
        max_scroll.saturating_sub(distance_from_end)
    } else {
        scroll
    };
    let scroll = scroll.min(usize::from(u16::MAX)) as u16;
    frame.render_widget(
        Paragraph::new(text)
            .scroll((scroll, 0))
            .wrap(Wrap { trim: false }),
        area,
    );
    max_scroll > 0
}

pub(super) fn wrapped_text_height(text: &Text<'_>, width: u16) -> usize {
    Paragraph::new(text.clone())
        .wrap(Wrap { trim: false })
        .line_count(width.max(1))
}

const TAINTED_EVIDENCE_LIMIT: usize = 3;

fn review_text(plan: &RestackPlan, show_details: bool) -> Text<'static> {
    let retained = plan.selection.retained.len();
    let removed = plan.selection.removed.len();
    let outcomes = resolution_summary(plan);
    let mut lines = vec![
        Line::from(vec![
            Span::styled("RESTACK REVIEW", Palette::primary().bold()),
            Span::styled(
                format!(
                    "  ·  {}/{}",
                    escape(&plan.snapshot.remote),
                    escape(&plan.snapshot.environment)
                ),
                Palette::muted(),
            ),
        ]),
        Line::from(""),
        Line::styled("Remote rewrite", Palette::warning().bold()),
        Line::from(vec![
            Span::styled("Rewrite         ", Palette::muted()),
            Span::styled(
                short_oid(&plan.snapshot.environment_tip),
                Palette::warning(),
            ),
            Span::styled("  →  ", Palette::muted()),
            Span::styled(short_oid(&plan.preview_commit), Palette::primary().bold()),
        ]),
        Line::from(vec![
            Span::styled("Impact          ", Palette::muted()),
            Span::raw(format!(
                "{retained} retained · {removed} omitted from the rebuilt environment · {outcomes}"
            )),
        ]),
    ];
    if plan.snapshot.inventory_mode == InventoryMode::Reachability {
        lines.push(Line::from(vec![
            Span::styled("Inventory       ", Palette::muted()),
            Span::styled(
                format!(
                    "reachability · oldest tip first · resolutions not reused · {}",
                    dropped_summary(plan.orphaned_commits.len())
                ),
                Palette::warning(),
            ),
        ]));
    }
    if !plan.selection.removed.is_empty() {
        lines.push(Line::from(""));
        lines.push(Line::styled(
            format!(
                "Omitted from {}/{}",
                escape(&plan.snapshot.remote),
                escape(&plan.snapshot.environment)
            ),
            Palette::warning().bold(),
        ));
        lines.push(Line::styled(
            "Their remote branches are not changed or deleted; press Esc to revise.",
            Palette::muted(),
        ));
        lines.extend(plan.selection.removed.iter().map(|branch| {
            Line::from(vec![
                Span::styled(
                    format!("  –  {}", escape(&branch.name)),
                    Palette::warning().bold(),
                ),
                Span::styled(
                    format!("  {}  omitted by your selection", short_oid(&branch.tip)),
                    Palette::warning(),
                ),
            ])
        }));
    }
    lines.extend([
        Line::from(""),
        Line::from(vec![
            Span::styled("Remote guard    ", Palette::muted()),
            Span::raw(format!(
                "publish stops if {}/{} changed since this review",
                escape(&plan.snapshot.remote),
                escape(&plan.snapshot.environment)
            )),
        ]),
    ]);
    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::styled("Plan details", Palette::muted().bold()),
        Span::styled(
            if show_details {
                "  ·  d hide refs, identities, endpoints, and signing"
            } else {
                "  ·  d show refs, identities, endpoints, and signing"
            },
            Palette::muted(),
        ),
    ]));
    if show_details {
        lines.extend(technical_detail_lines(plan));
    }
    lines.extend([
        Line::from(""),
        Line::styled("Retained merge order", Palette::primary().bold()),
        Line::styled(
            if plan.snapshot.inventory_mode == InventoryMode::Reachability {
                "Selected feature tips are rebuilt oldest tip first, then by name."
            } else {
                "Selected feature tips are rebuilt in this order."
            },
            Palette::muted(),
        ),
        Line::styled(
            "   #  BRANCH                            COMMIT   OUTCOME",
            Palette::muted(),
        ),
    ]);
    if plan.selection.retained.is_empty() {
        lines.push(Line::raw(
            "  —  none; the environment becomes the captured base",
        ));
    } else {
        lines.extend(
            plan.selection
                .retained
                .iter()
                .enumerate()
                .map(|(index, branch)| {
                    let (outcome, style) = plan.merges.get(index).map_or(
                        ("unavailable", Palette::warning()),
                        |merge| match merge.resolution {
                            graduate::restack::MergeResolution::Clean => {
                                ("✓ clean", Palette::success())
                            }
                            graduate::restack::MergeResolution::Reused => {
                                ("✓ history reused", Palette::success())
                            }
                            graduate::restack::MergeResolution::Manual => {
                                ("◆ manual", Palette::warning())
                            }
                        },
                    );
                    Line::from(vec![
                        Span::raw(format!(
                            "  {:>2}  {}  {:<7}  ",
                            index + 1,
                            pad_text(&truncate_text(&escape(&branch.name), 32), 32),
                            short_oid(&branch.tip)
                        )),
                        Span::styled(outcome, style),
                    ])
                }),
        );
    }
    if !plan.orphaned_commits.is_empty() {
        lines.push(Line::from(""));
        lines.push(Line::styled(
            format!("Dropped commits ({})", plan.orphaned_commits.len()),
            Palette::warning().bold(),
        ));
        lines.push(Line::styled(
            "On no retained branch; they will not be in the rebuilt environment.",
            Palette::muted(),
        ));
        lines.extend(plan.orphaned_commits.iter().map(|commit| {
            Line::from(vec![
                Span::styled(
                    format!(
                        "  {}  {}  ",
                        short_oid(&commit.commit),
                        escape(&commit.date)
                    ),
                    Palette::warning(),
                ),
                Span::styled(
                    format!(
                        "{}  ",
                        pad_text(&truncate_text(&escape(&commit.author), 14), 14)
                    ),
                    Palette::muted(),
                ),
                Span::raw(truncate_text(&escape(&commit.subject), 48)),
            ])
        }));
    }
    if !plan.snapshot.tainted_features.is_empty() {
        lines.push(Line::from(""));
        lines.push(Line::styled(
            format!(
                "Tainted branches ({})",
                plan.snapshot.tainted_features.len()
            ),
            Palette::warning().bold(),
        ));
        lines.push(Line::styled(
            format!(
                "Merged {} into themselves; removed and never retained. Recreate from {} and cherry-pick.",
                escape(&plan.snapshot.environment),
                escape(&plan.snapshot.main)
            ),
            Palette::muted(),
        ));
        lines.extend(
            plan.snapshot
                .tainted_features
                .iter()
                .take(TAINTED_EVIDENCE_LIMIT)
                .map(|tainted| {
                    Line::from(vec![
                        Span::styled(
                            format!(
                                "  {}  {}  ",
                                pad_text(&truncate_text(&escape(&tainted.name), 32), 32),
                                short_oid(&tainted.tip)
                            ),
                            Palette::warning(),
                        ),
                        Span::styled(
                            absorbed_merges_text(tainted.absorbed_merges.len()),
                            Palette::muted(),
                        ),
                    ])
                }),
        );
        let hidden = plan
            .snapshot
            .tainted_features
            .len()
            .saturating_sub(TAINTED_EVIDENCE_LIMIT);
        if hidden > 0 {
            lines.push(Line::styled(
                format!("  … and {hidden} more"),
                Palette::muted(),
            ));
        }
    }
    if show_details {
        lines.push(Line::from(""));
        lines.push(Line::styled(
            "Exact feature identities",
            Palette::muted().bold(),
        ));
        lines.extend(plan.selection.retained.iter().map(|branch| {
            Line::raw(format!(
                "  retained  {} @ {}",
                escape(&branch.name),
                escape(&branch.tip)
            ))
        }));
        lines.extend(plan.selection.removed.iter().map(|branch| {
            Line::raw(format!(
                "  removed   {} @ {}",
                escape(&branch.name),
                escape(&branch.tip)
            ))
        }));
    }
    Text::from(lines)
}
