//! Review technical detail and summary lines.

use graduate::restack::RestackPlan;
use ratatui::text::{Line, Span};

use super::render::short_oid;
use crate::shared::terminal_text::escape;
use crate::shared::theme::Palette;

pub(super) fn technical_detail_lines(plan: &RestackPlan) -> Vec<Line<'static>> {
    vec![
        Line::from(vec![
            Span::styled("Base            ", Palette::muted()),
            Span::raw(format!(
                "{} @ {}",
                escape(&plan.snapshot.main_ref),
                escape(&plan.snapshot.main_tip)
            )),
        ]),
        Line::from(vec![
            Span::styled("Environment     ", Palette::muted()),
            Span::raw(format!(
                "{} @ {}",
                escape(&plan.snapshot.environment_ref),
                escape(&plan.snapshot.environment_tip)
            )),
        ]),
        Line::from(vec![
            Span::styled("Preview commit  ", Palette::muted()),
            Span::raw(escape(&plan.preview_commit)),
        ]),
        Line::from(vec![
            Span::styled("Final tree      ", Palette::muted()),
            Span::raw(escape(&plan.final_tree)),
        ]),
        Line::from(vec![
            Span::styled("Author          ", Palette::muted()),
            Span::raw(format!(
                "{} <{}>",
                escape(&plan.author.name),
                escape(&plan.author.email)
            )),
        ]),
        Line::from(vec![
            Span::styled("Fetch endpoint  ", Palette::muted()),
            Span::raw(format!("sha256:{}", plan.remote_endpoints.fetch_sha256)),
        ]),
        Line::from(vec![
            Span::styled("Push endpoint   ", Palette::muted()),
            Span::raw(format!("sha256:{}", plan.remote_endpoints.push_sha256)),
        ]),
        Line::from(vec![
            Span::styled("Publish guard   ", Palette::muted()),
            Span::raw(format!(
                "exact lease; {}/{} must still be at {}",
                escape(&plan.snapshot.remote),
                escape(&plan.snapshot.environment),
                escape(&plan.snapshot.environment_tip)
            )),
        ]),
        Line::from(vec![
            Span::styled("Signing         ", Palette::muted()),
            Span::raw("unsigned canonical merge commits"),
        ]),
        Line::from(vec![
            Span::styled("Dropped markers ", Palette::muted()),
            Span::raw(format!(
                "{} exact phase marker(s)",
                plan.snapshot.dropped_markers.len()
            )),
        ]),
        Line::from(vec![
            Span::styled("Inventory       ", Palette::muted()),
            Span::raw(match &plan.snapshot.unsupported_history {
                None => "history proof; every commit attributed".to_owned(),
                Some(reason) => format!(
                    "reachability; history proof failed with {} at {}",
                    escape(&reason.kind),
                    reason
                        .commit
                        .as_deref()
                        .map_or_else(|| "?".to_owned(), short_oid)
                ),
            }),
        ]),
        Line::from(vec![
            Span::styled("Carried         ", Palette::muted()),
            Span::raw(format!(
                "{} branch(es) reached by a retained tip",
                plan.snapshot.carried_features.len()
            )),
        ]),
    ]
}

pub(super) fn dropped_summary(count: usize) -> String {
    match count {
        0 => "no commits dropped".to_owned(),
        1 => "1 commit dropped".to_owned(),
        count => format!("{count} commits dropped"),
    }
}

pub(super) fn resolution_summary(plan: &RestackPlan) -> String {
    let clean = plan
        .merges
        .iter()
        .filter(|merge| merge.resolution == graduate::restack::MergeResolution::Clean)
        .count();
    let reused = plan
        .merges
        .iter()
        .filter(|merge| merge.resolution == graduate::restack::MergeResolution::Reused)
        .count();
    let manual = plan
        .merges
        .iter()
        .filter(|merge| merge.resolution == graduate::restack::MergeResolution::Manual)
        .count();
    let mut parts = Vec::new();
    if clean > 0 {
        parts.push(format!("{clean} clean"));
    }
    if reused > 0 {
        parts.push(format!("{reused} history reused"));
    }
    if manual > 0 {
        parts.push(format!("{manual} manual"));
    }
    let total = clean.saturating_add(reused).saturating_add(manual);
    if parts.is_empty() {
        "0 merges".to_owned()
    } else if parts.len() == 1 {
        format!(
            "{} {}",
            parts.join(""),
            if total == 1 { "merge" } else { "merges" }
        )
    } else {
        format!("{total} merges: {}", parts.join(" · "))
    }
}
