//! Unsupported-history explanation screen.

use graduate::restack::RestackInteraction;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Paragraph, Wrap};
use ratatui::Frame;

use super::render::short_oid;
use super::review::wrapped_text_height;
use crate::terminal_text::escape;
use crate::theme::Palette;

pub(super) fn render_unsupported_history(
    frame: &mut Frame<'_>,
    area: Rect,
    interaction: &RestackInteraction,
) -> bool {
    let text = unsupported_history_text(interaction);
    let height = wrapped_text_height(&text, area.width);
    frame.render_widget(Paragraph::new(text).wrap(Wrap { trim: false }), area);
    height > usize::from(area.height)
}

const EVIDENCE_LIMIT: usize = 3;

fn unsupported_history_text(interaction: &RestackInteraction) -> Text<'static> {
    let snapshot = interaction.snapshot();
    let environment = escape(&snapshot.environment);
    let main = escape(&snapshot.main);
    let mut lines = vec![
        Line::from(vec![
            Span::styled("HISTORY CANNOT BE READ", Palette::warning().bold()),
            Span::styled(
                format!("  ·  {}/{environment}", escape(&snapshot.remote)),
                Palette::muted(),
            ),
        ]),
        Line::from(""),
    ];
    let Some(reason) = interaction.unsupported_history() else {
        lines.push(Line::raw(
            "The environment history could not be classified.",
        ));
        return Text::from(lines);
    };
    let commit = reason
        .commit
        .as_deref()
        .map_or_else(|| "?".to_owned(), short_oid);
    let feature_parent = reason
        .feature_parent
        .as_deref()
        .map_or_else(|| "?".to_owned(), short_oid);
    let (explanation, evidence_title): (Vec<String>, Option<&str>) = match reason.kind.as_str() {
        "ambiguousFeatureRefs" => (
            vec![
                format!(
                    "Merge {commit} on {environment}'s history brings in {feature_parent}, which {} branches contain.",
                    reason.branches.len()
                ),
                "Restack cannot tell which one it meant.".to_owned(),
            ],
            Some("Branches containing that commit"),
        ),
        "deletedFeatureRef" => (
            vec![
                format!(
                    "Merge {commit} on {environment}'s history brings in {feature_parent},"
                ),
                "but no remote branch contains it any more.".to_owned(),
            ],
            None,
        ),
        "directCommit" => (
            vec![
                format!("Commit {commit} was made directly on {environment}"),
                "instead of being merged from a feature branch.".to_owned(),
            ],
            None,
        ),
        "fastForwardHistory" => (
            vec![
                format!("{environment} was fast-forwarded through {commit};"),
                "there is no merge commit to attribute that work to.".to_owned(),
            ],
            Some("Branches containing that commit"),
        ),
        "octopusMerge" => (
            vec![
                format!(
                    "Merge {commit} has {} parents;",
                    reason.parents.unwrap_or_default()
                ),
                "restack only understands two-parent merges.".to_owned(),
            ],
            None,
        ),
        "missingCommit" => (
            vec![format!(
                "Commit {commit} is missing from the fetched history."
            )],
            None,
        ),
        other => (
            vec![format!("The history proof failed with {}.", escape(other))],
            None,
        ),
    };
    lines.extend(
        explanation
            .into_iter()
            .map(|sentence| Line::styled(sentence, Palette::text())),
    );
    if let Some(title) = evidence_title {
        if !reason.branches.is_empty() {
            lines.push(Line::from(""));
            lines.push(Line::styled(title, Palette::muted().bold()));
            lines.extend(
                reason
                    .branches
                    .iter()
                    .take(EVIDENCE_LIMIT)
                    .map(|branch| Line::raw(format!("  •  {}", escape(branch)))),
            );
            if reason.branches.len() > EVIDENCE_LIMIT {
                lines.push(Line::styled(
                    format!(
                        "  … and {} more",
                        reason.branches.len().saturating_sub(EVIDENCE_LIMIT)
                    ),
                    Palette::muted(),
                ));
            }
        }
    }
    let top_level = snapshot.features.len();
    let carried = interaction.carried_features().len();
    let dropped = interaction.orphaned_commit_count();
    lines.extend([
        Line::from(""),
        Line::styled(
            "Rebuilding from inventory instead",
            Palette::primary().bold(),
        ),
        Line::raw(format!(
            "  •  Membership: remote tips in {environment}, not in {main}. You pick."
        )),
        Line::raw("  •  Order: oldest branch tip first. No reused resolutions."),
        Line::raw("  •  Commits on no kept branch are dropped; listed first."),
        Line::from(""),
        Line::from(vec![
            Span::raw(format!(
                "{top_level} top-level {} · {carried} carried · ",
                if top_level == 1 { "branch" } else { "branches" }
            )),
            Span::styled(
                format!(
                    "{dropped} {} dropped",
                    if dropped == 1 { "commit" } else { "commits" }
                ),
                if dropped == 0 {
                    Palette::muted()
                } else {
                    Palette::warning()
                },
            ),
        ]),
    ]);
    Text::from(lines)
}
