//! Isolated restack preview, interactive review, resume, and apply workflow.

use serde::Deserialize;
use serde_json::json;

use crate::cli::RestackArgs;
use crate::restack::session::SessionStore;
use crate::shared::environment_git::validate_ref_component;
use crate::shared::error::CliError;
use errors::session_error;
use interactive::{interactive_resume_requested, interactive_terminal, run_interactive};
use interactive_resume::run_interactive_resume;
use machine_output::{machine_failure, machine_usage};
use preview::preview;
use resume::{abort_session, resume_apply, resume_preview};

mod errors;
mod interactive;
mod interactive_resume;
mod interactive_steps;
mod isolated;
mod isolated_merge;
mod isolated_validation;
mod machine_output;
mod plan_validation;
mod preview;
mod resume;
mod sealed;
pub(crate) mod session;
mod source;
#[cfg(test)]
mod tests;
pub(crate) mod tui;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct MachineParams {
    remove_branches: Vec<String>,
    #[serde(default)]
    plan_digest: Option<String>,
}

pub(crate) fn run(args: RestackArgs) -> Result<(), CliError> {
    if args.params.is_none() && args.resume.is_none() && !args.dry_run {
        return run_interactive(args);
    }
    if interactive_resume_requested(&args, interactive_terminal()) {
        return run_interactive_resume(args);
    }
    validate_inputs(&args)?;
    let sessions = SessionStore::open().map_err(session_error)?;
    let source = std::env::current_dir().map_err(|_| {
        machine_failure(
            "repository_unavailable",
            "could not access the source repository",
            json!({"stage": "currentDirectory"}),
        )
    })?;
    if let Some(token) = args.resume.as_deref() {
        sessions.prepare_resume(token).map_err(session_error)?;
        if args.abort {
            return abort_session(&args, token, &source, &sessions);
        }
        if args.apply {
            return resume_apply(&args, token, &source, &sessions);
        }
        return resume_preview(&args, token, &source, &sessions);
    }
    sessions.purge_expired().map_err(session_error)?;
    preview(&args, &source, &sessions)
}

/// `git diff --check` in the isolated repository must reject leftover conflict
/// markers but not the whitespace habits of feature content, which a rebuild
/// reproduces faithfully.
const ISOLATED_WHITESPACE_POLICY: &str =
    "-trailing-space,-space-before-tab,-indent-with-non-tab,-tab-in-indent,-blank-at-eof,-blank-at-eol";

/// Object cache shared by the environment, main, and feature history walks.
const INSPECTION_OBJECT_CACHE_BYTES: usize = 64 * 1024 * 1024;

fn validate_inputs(args: &RestackArgs) -> Result<(), CliError> {
    if args.abort && args.resume.is_none() {
        return Err(machine_usage(
            "invalid_usage",
            "--abort requires --resume",
            json!({}),
        ));
    }
    for (label, value) in [
        ("environment", args.environment.as_str()),
        ("remote", args.remote.as_deref().unwrap_or("origin")),
    ] {
        validate_ref_component(label, value).map_err(|_| {
            machine_usage(
                "invalid_ref",
                "a restack ref name is not valid",
                json!({"field": label}),
            )
        })?;
    }
    if let Some(main) = &args.main {
        validate_ref_component("main", main).map_err(|_| {
            machine_usage(
                "invalid_ref",
                "a restack ref name is not valid",
                json!({"field": "main"}),
            )
        })?;
    }
    Ok(())
}

fn parse_params(params: Option<&str>, dry_run: bool) -> Result<MachineParams, CliError> {
    let Some(params) = params else {
        if dry_run {
            return Ok(MachineParams {
                remove_branches: Vec::new(),
                plan_digest: None,
            });
        }
        return Err(machine_usage(
            "params_required",
            "a machine restack preview requires --params",
            json!({"expected": {"removeBranches": []}}),
        ));
    };
    let parsed: MachineParams = serde_json::from_str(params).map_err(|_| {
        machine_usage(
            "invalid_params",
            "--params must match the schema-v3 restack machine parameters",
            json!({"expected": {"removeBranches": ["feature/BRANCH"], "planDigest": "apply only"}}),
        )
    })?;
    for (index, branch) in parsed.remove_branches.iter().enumerate() {
        validate_ref_component("removeBranches entry", branch).map_err(|_| {
            machine_usage(
                "invalid_params",
                "removeBranches contains an invalid Git branch name",
                json!({"index": index}),
            )
        })?;
    }
    Ok(parsed)
}

fn validate_apply_params(apply: bool, params: &MachineParams) -> Result<(), CliError> {
    match (apply, params.plan_digest.as_deref()) {
        (false, None) => Ok(()),
        (false, Some(_)) => Err(machine_usage(
            "invalid_params",
            "planDigest is accepted only with --apply",
            json!({"field": "planDigest"}),
        )),
        (true, Some(digest)) if valid_plan_digest(digest) => Ok(()),
        (true, _) => Err(machine_usage(
            "plan_digest_required",
            "--apply requires the lowercase SHA-256 planDigest from a preview",
            json!({"field": "planDigest"}),
        )),
    }
}

fn valid_plan_digest(digest: &str) -> bool {
    digest.len() == 64
        && digest
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}
