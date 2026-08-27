//! Interactive restack workflow, terminal lifecycle, and outcome output.

use std::collections::BTreeMap;
use std::io::IsTerminal;
use std::path::Path;

use graduate::restack::{
    InventoryMode, OrphanedCommit, RestackAuthor, RestackInteraction, RestackPlan, RestackSnapshot,
};
use serde_json::json;

use super::errors::session_error;
use super::interactive_steps::{discover_interactive, prepare_interactive, publish_interactive};
use super::isolated::IsolatedRepository;
use super::machine_output::{machine_failure, machine_usage};
use super::validate_inputs;
use crate::cli::RestackArgs;
use crate::error::CliError;
use crate::git_process;
use crate::restack_session::{SessionDraft, SessionStore};
use crate::restack_tui::{self, ConflictHandoff, ReviewDecision, SelectionDecision};
use crate::terminal::StderrTerminal;

pub(super) struct InteractiveDiscovery {
    pub(super) remote: git_process::RestackRemote,
    pub(super) repository_id: String,
    pub(super) snapshot: RestackSnapshot,
    /// Rows for every commit a reachability rebuild might drop; empty in history mode.
    pub(super) commit_rows: BTreeMap<String, OrphanedCommit>,
    pub(super) author: RestackAuthor,
    pub(super) source_objects: Vec<u8>,
}

pub(super) struct InteractivePrepared {
    pub(super) isolated: IsolatedRepository,
    pub(super) draft: SessionDraft,
    pub(super) plan: RestackPlan,
}

pub(super) struct InteractiveConflict {
    pub(super) environment: String,
    pub(super) branch: String,
    pub(super) unresolved_paths: Vec<String>,
    pub(super) resume_token: String,
    pub(super) work_area: String,
}

pub(super) enum InteractivePreparation {
    Complete(Box<InteractivePrepared>),
    Conflict(InteractiveConflict),
}

pub(super) enum InteractiveOutcome {
    Cancelled(String),
    Published(Box<RestackPlan>),
    Conflict(InteractiveConflict),
}

pub(super) fn run_interactive(args: RestackArgs) -> Result<(), CliError> {
    if args.apply || args.abort || args.dry_run {
        return Err(machine_usage(
            "invalid_usage",
            "interactive restack uses terminal confirmation instead of --apply or --abort",
            json!({}),
        ));
    }
    if !std::io::stdin().is_terminal() || !std::io::stderr().is_terminal() {
        return Err(machine_usage(
            "params_required",
            "a non-terminal restack requires --params or --resume",
            json!({"expected": {"removeBranches": []}}),
        ));
    }
    validate_inputs(&args)?;
    let sessions = SessionStore::open().map_err(session_error)?;
    sessions.purge_expired().map_err(session_error)?;
    let source = std::env::current_dir().map_err(|_| {
        machine_failure(
            "repository_unavailable",
            "could not access the source repository",
            json!({"stage": "currentDirectory"}),
        )
    })?;
    let mut terminal = StderrTerminal::new()?;
    let outcome = interactive_workflow(&args, &source, &sessions, &mut terminal);
    finish_interactive(outcome, || terminal.restore(), write_interactive_outcome)
}

pub(super) fn finish_interactive(
    outcome: Result<InteractiveOutcome, CliError>,
    restore: impl FnOnce() -> std::io::Result<()>,
    write: impl FnOnce(InteractiveOutcome) -> Result<(), CliError>,
) -> Result<(), CliError> {
    restore()?;
    write(outcome.map_err(interactive_error)?)
}

fn write_interactive_outcome(outcome: InteractiveOutcome) -> Result<(), CliError> {
    match outcome {
        InteractiveOutcome::Cancelled(environment) => restack_tui::write_cancelled(&environment),
        InteractiveOutcome::Published(plan) => restack_tui::write_success(&plan),
        InteractiveOutcome::Conflict(conflict) => restack_tui::write_conflict(&ConflictHandoff {
            environment: &conflict.environment,
            branch: &conflict.branch,
            unresolved_paths: &conflict.unresolved_paths,
            resume_token: &conflict.resume_token,
            work_area: &conflict.work_area,
        }),
    }
}

fn interactive_workflow(
    args: &RestackArgs,
    source: &Path,
    sessions: &SessionStore,
    terminal: &mut StderrTerminal,
) -> Result<InteractiveOutcome, CliError> {
    restack_tui::draw_loading(terminal, "Fetching and inspecting the environment…")?;
    let discovery = discover_interactive(args, source)?;
    let mut interaction = match discovery.snapshot.inventory_mode {
        InventoryMode::History => RestackInteraction::new(discovery.snapshot.clone()),
        InventoryMode::Reachability => {
            RestackInteraction::from_inventory(discovery.snapshot.clone())
        }
    };
    loop {
        let selection = match restack_tui::choose_features(terminal, &mut interaction)? {
            SelectionDecision::Preview(selection) => selection,
            SelectionDecision::Cancel => {
                return Ok(InteractiveOutcome::Cancelled(args.environment.clone()));
            }
        };
        restack_tui::draw_loading(terminal, "Reconstructing the reviewed selection…")?;
        let prepared = match prepare_interactive(&discovery, selection, sessions)? {
            InteractivePreparation::Complete(prepared) => prepared,
            InteractivePreparation::Conflict(conflict) => {
                return Ok(InteractiveOutcome::Conflict(conflict));
            }
        };
        match restack_tui::review_plan(terminal, &mut interaction, &prepared.plan)? {
            ReviewDecision::Revise => {
                prepared.draft.discard().map_err(session_error)?;
            }
            ReviewDecision::Cancel => {
                prepared.draft.discard().map_err(session_error)?;
                return Ok(InteractiveOutcome::Cancelled(args.environment.clone()));
            }
            ReviewDecision::Publish => {
                restack_tui::draw_loading(terminal, "Revalidating and publishing under lease…")?;
                publish_interactive(
                    source,
                    &discovery.remote,
                    &prepared.isolated,
                    &prepared.plan,
                )?;
                prepared.draft.discard().map_err(session_error)?;
                return Ok(InteractiveOutcome::Published(Box::new(prepared.plan)));
            }
        }
    }
}

pub(super) fn interactive_error(error: CliError) -> CliError {
    match error {
        CliError::Machine(error) => CliError::Restack(error.detailed_message()),
        error => error,
    }
}
