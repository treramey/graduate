//! Post-restoration human handoff output.

use std::io::{self, Write};

use graduate::restack::{InventoryMode, RestackPlan};

use super::render::short_oid;
use super::review_details::dropped_summary;
use super::ConflictHandoff;
use crate::error::CliError;
use crate::terminal_text::escape;

pub(crate) fn write_cancelled(environment: &str) -> Result<(), CliError> {
    write_human(cancelled_text(environment))
}

pub(crate) fn write_success(plan: &RestackPlan) -> Result<(), CliError> {
    write_human(success_text(plan))
}

pub(crate) fn write_conflict(handoff: &ConflictHandoff<'_>) -> Result<(), CliError> {
    write_human(conflict_text(handoff))
}

fn write_human(text: String) -> Result<(), CliError> {
    writeln!(io::stderr().lock(), "{text}").map_err(CliError::Io)
}

fn cancelled_text(environment: &str) -> String {
    format!(
        "Restack of {} cancelled; no remote refs changed.",
        escape(environment)
    )
}

pub(super) fn success_text(plan: &RestackPlan) -> String {
    let inventory = match plan.snapshot.inventory_mode {
        InventoryMode::History => String::new(),
        InventoryMode::Reachability => format!(
            " Rebuilt from inventory; {}.",
            dropped_summary(plan.orphaned_commits.len())
        ),
    };
    format!(
        "Restacked {}/{}: {} -> {} (tree {}); {} retained, {} omitted from the environment.{inventory}",
        escape(&plan.snapshot.remote),
        escape(&plan.snapshot.environment),
        short_oid(&plan.snapshot.environment_tip),
        short_oid(&plan.preview_commit),
        short_oid(&plan.final_tree),
        plan.selection.retained.len(),
        plan.selection.removed.len(),
    )
}

pub(super) fn conflict_text(handoff: &ConflictHandoff<'_>) -> String {
    let paths = handoff
        .unresolved_paths
        .iter()
        .map(|path| format!("  - {}", escape(path)))
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        "Restack of {} paused on {}.\nUnresolved paths:\n{}\nWork area: {}\n\nResolve this preserved session:\n  1. Edit the unresolved files in the work area.\n  2. Stage every resolution there; leave no unstaged or untracked files.\n  3. Resume with: gd restack {} --resume {}\n\nDo not commit; Graduate creates the canonical merge commit.\nThis resumable session expires after 24 hours of inactivity.",
        escape(handoff.environment),
        escape(handoff.branch),
        paths,
        escape(handoff.work_area),
        escape(handoff.environment),
        handoff.resume_token,
    )
}
