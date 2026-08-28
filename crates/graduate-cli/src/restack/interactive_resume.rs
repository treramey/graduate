//! Terminal continuation of a preserved restack session.
//!
//! A human who resolved a conflict in the work area returns to the same
//! review screen the initial run would have shown, instead of a machine plan.
//! Leaving the review keeps the sealed session; only `--abort` discards it.

use std::path::Path;

use graduate::restack::RestackInteraction;
use serde_json::json;

use super::errors::session_error;
use super::interactive::{
    finish_interactive, write_interactive_outcome, InteractiveConflict, InteractiveOutcome,
    InteractivePreserved,
};
use super::interactive_steps::work_area_text;
use super::machine_output::{machine_failure, machine_usage};
use super::resume::{continue_session, ResumedPreview};
use super::sealed::publish_sealed;
use super::validate_inputs;
use crate::cli::RestackArgs;
use crate::restack::session::SessionStore;
use crate::restack::tui::{draw_loading, review_plan, ReviewDecision};
use crate::shared::error::CliError;
use crate::shared::terminal::StderrTerminal;

pub(super) fn run_interactive_resume(args: RestackArgs) -> Result<(), CliError> {
    let Some(token) = args.resume.clone() else {
        return Err(machine_usage(
            "invalid_usage",
            "interactive resume requires --resume",
            json!({}),
        ));
    };
    validate_inputs(&args)?;
    let sessions = SessionStore::open().map_err(session_error)?;
    sessions.prepare_resume(&token).map_err(session_error)?;
    let source = std::env::current_dir().map_err(|_| {
        machine_failure(
            "repository_unavailable",
            "could not access the source repository",
            json!({"stage": "currentDirectory"}),
        )
    })?;
    let mut terminal = StderrTerminal::new()?;
    let outcome = resumed_workflow(&args, &token, &source, &sessions, &mut terminal);
    finish_interactive(outcome, || terminal.restore(), write_interactive_outcome)
}

fn resumed_workflow(
    args: &RestackArgs,
    token: &str,
    source: &Path,
    sessions: &SessionStore,
    terminal: &mut StderrTerminal,
) -> Result<InteractiveOutcome, CliError> {
    draw_loading(terminal, "Validating the staged resolution…")?;
    let (session, plan) = match continue_session(args, token, source, sessions)? {
        ResumedPreview::Conflict {
            session,
            branch,
            unresolved_paths,
        } => {
            return Ok(InteractiveOutcome::Conflict(InteractiveConflict {
                environment: args.environment.clone(),
                branch,
                unresolved_paths,
                resume_token: token.to_owned(),
                work_area: work_area_text(&session.repository())?,
            }));
        }
        ResumedPreview::Sealed { session, plan } => (session, plan),
    };
    let mut interaction = RestackInteraction::for_review(plan.snapshot.clone());
    match review_plan(terminal, &mut interaction, &plan)? {
        ReviewDecision::Publish => {
            draw_loading(terminal, "Revalidating and publishing under lease…")?;
            let plan = publish_sealed(source, *session)?;
            Ok(InteractiveOutcome::Published(Box::new(plan)))
        }
        ReviewDecision::Cancel | ReviewDecision::Revise => {
            drop(session);
            Ok(InteractiveOutcome::Preserved(InteractivePreserved {
                environment: args.environment.clone(),
                resume_token: token.to_owned(),
            }))
        }
    }
}
