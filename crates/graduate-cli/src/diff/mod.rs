//! Environment-to-main promotion report orchestration and Git adapters.

use std::collections::HashSet;
use std::io::{self, IsTerminal};
use std::path::{Path, PathBuf};

use graduate::promotion::{
    jira_key_from_branch, EnvironmentInventory, JiraIssueState, PromotionBranch,
};
use tokio::sync::mpsc;

use crate::cli::{DiffArgs, DiffReport, ReportFormat};
use crate::shared::browser::SystemBrowserLauncher;
use crate::shared::config::Config;
use crate::shared::environment_git::{
    gitoxide_error, inspect_environment, promotion_candidates, promotion_inventory,
    validate_ref_component, KNOWN_ENVIRONMENTS,
};
use crate::shared::error::CliError;
use crate::shared::git_process::fetch_remote as fetch_remote_name;
use branches::{
    environment_merge_markers, environment_subjects, measure_branch,
    recover_deleted_branch_tickets, MeasureContext,
};
use output::write_report;
use params::parse_selected_branches;
use scan_channel::{collect_plain, coordinate_scan};

mod age_csv;
mod branches;
mod merge_check;
mod output;
mod params;
mod readiness_csv;
mod report_csv;
mod report_json;
mod report_readiness;
mod report_table;
mod scan_channel;
#[cfg(test)]
mod tests;
pub(crate) mod tui;

pub(crate) use output::current_report_date;
pub(crate) use report_json::{age_bucket_label, age_bucket_reading, share_percent};

#[derive(Clone, Debug)]
pub(crate) enum DiffUpdate {
    Skeleton {
        environment: String,
        main: String,
        branches: Vec<String>,
    },
    Inventory(EnvironmentInventory),
    Measured(PromotionBranch),
    Jira {
        branch: String,
        state: JiraIssueState,
    },
    Finished,
    Failed(String),
}

pub(crate) struct PromotionReport {
    pub(crate) environment: String,
    pub(crate) main: String,
    pub(crate) inventory: EnvironmentInventory,
    pub(crate) branches: Vec<PromotionBranch>,
}

struct ScanOptions {
    repository: PathBuf,
    environment: String,
    main: Option<String>,
    remote: String,
    jira_configured: bool,
    fetch_before_scan: bool,
    selected_branches: Option<Vec<String>>,
    /// Run the in-memory merge of every branch tip onto main. Only the
    /// readiness report asks for it.
    check_merge_onto_main: bool,
}

pub(crate) async fn run(args: DiffArgs, config_path: &Path) -> Result<(), CliError> {
    validate_ref_component("environment", &args.environment)?;
    validate_ref_component("remote", &args.remote)?;
    if let Some(main) = &args.main {
        validate_ref_component("main branch", main)?;
    }
    let selected_branches = parse_selected_branches(args.params.as_deref())?;
    let report_kind = args.report.unwrap_or(DiffReport::Branches);
    let interactive = args.report.is_none()
        && args.params.is_none()
        && args.output_format.is_none()
        && args.output.is_none()
        && io::stdin().is_terminal()
        && io::stderr().is_terminal();
    if !args.no_fetch && !interactive {
        fetch_remote(&args, interactive)?;
    }

    let credentials = if matches!(report_kind, DiffReport::Age) {
        None
    } else {
        Config::load(config_path)?.jira_credentials()?
    };
    let (updates_tx, updates_rx) = mpsc::unbounded_channel();
    let scan = ScanOptions {
        repository: std::env::current_dir()?,
        environment: args.environment.clone(),
        main: args.main.clone(),
        remote: args.remote.clone(),
        jira_configured: credentials.is_some(),
        fetch_before_scan: !args.no_fetch && interactive,
        selected_branches,
        check_merge_onto_main: matches!(report_kind, DiffReport::Readiness),
    };
    let coordinator = tokio::spawn(coordinate_scan(scan, credentials, updates_tx));

    let report_result = if interactive {
        tui::run(updates_rx, &SystemBrowserLauncher).await
    } else {
        collect_plain(updates_rx).await
    };
    let coordinator_result = coordinator
        .await
        .map_err(|error| CliError::Git(format!("promotion scan task failed: {error}")))?;
    coordinator_result?;
    let report = report_result?;
    if !interactive {
        write_report(
            &report,
            report_kind,
            args.output_format.unwrap_or(ReportFormat::Json),
            args.output.as_deref(),
        )?;
    }
    Ok(())
}

fn scan_repository(
    options: &ScanOptions,
    updates: &mpsc::UnboundedSender<DiffUpdate>,
) -> Result<(), CliError> {
    let repository = gix::discover(&options.repository).map_err(gitoxide_error)?;
    let inspection = inspect_environment(
        &repository,
        &options.remote,
        &options.environment,
        options.main.as_deref(),
    )?;
    let names = environment_names(&options.environment, &inspection.main);
    let environment_markers = environment_merge_markers(
        &repository,
        &inspection.prefix,
        &names,
        &inspection.main_ancestors,
    )?;
    let environment_subjects = environment_subjects(&names, &options.remote);
    let measure_context = MeasureContext {
        environment: &options.environment,
        main_ancestors: &inspection.main_ancestors,
        environment_ancestors: &inspection.environment_ancestors,
        environment_markers: &environment_markers,
        environment_subjects: &environment_subjects,
    };

    let mut candidates = promotion_candidates(
        &repository,
        &inspection,
        options.selected_branches.as_deref(),
    )?;
    candidates.sort_by(|left, right| left.0.cmp(&right.0));
    let covered_keys = candidates
        .iter()
        .filter_map(|(branch, _)| jira_key_from_branch(branch))
        .collect::<HashSet<_>>();
    let recovered = if options.selected_branches.is_none() {
        recover_deleted_branch_tickets(
            &repository,
            inspection.environment_id,
            &inspection.main_ancestors,
            &covered_keys,
            options.jira_configured,
        )?
    } else {
        Vec::new()
    };
    let mut branches = candidates
        .iter()
        .map(|(name, _)| name.clone())
        .chain(recovered.iter().map(|row| row.branch.clone()))
        .collect::<Vec<_>>();
    branches.sort();
    updates
        .send(DiffUpdate::Skeleton {
            environment: options.environment.clone(),
            main: inspection.main.clone(),
            branches,
        })
        .map_err(|_| CliError::ReportCancelled)?;
    let branch_scoped = options.selected_branches.is_some();
    let mut scoped_commit_ids = HashSet::new();
    for (branch, id) in candidates {
        let jira = match jira_key_from_branch(&branch) {
            Some(key) if options.jira_configured => JiraIssueState::Loading { key },
            Some(key) => JiraIssueState::NotConfigured { key },
            None => JiraIssueState::NoTicket,
        };
        let mut row = measure_branch(&repository, &measure_context, branch, id, jira)?;
        if options.check_merge_onto_main {
            row.merge_onto_main = Some(merge_check::merge_onto_main(
                &repository,
                &inspection.main_ancestors,
                inspection.main_id,
                id,
            )?);
        }
        if branch_scoped {
            scoped_commit_ids.extend(row.commits.iter().map(|commit| commit.id.clone()));
        }
        updates
            .send(DiffUpdate::Measured(row))
            .map_err(|_| CliError::ReportCancelled)?;
    }
    for row in recovered {
        updates
            .send(DiffUpdate::Measured(row))
            .map_err(|_| CliError::ReportCancelled)?;
    }
    let mut inventory = promotion_inventory(&repository, &inspection)?;
    if branch_scoped {
        inventory
            .ahead
            .retain(|commit| scoped_commit_ids.contains(&commit.id));
    }
    updates
        .send(DiffUpdate::Inventory(inventory))
        .map_err(|_| CliError::ReportCancelled)?;
    Ok(())
}

/// Environment branch names to detect merges for: the requested environment
/// first, then the other known environments, never the main branch.
fn environment_names<'a>(environment: &'a str, main: &str) -> Vec<&'a str> {
    let mut names = vec![environment];
    names.extend(
        KNOWN_ENVIRONMENTS
            .iter()
            .copied()
            .filter(|name| *name != environment),
    );
    names.retain(|name| *name != main);
    names
}

fn fetch_remote(args: &DiffArgs, interactive: bool) -> Result<(), CliError> {
    fetch_remote_name(&args.remote, interactive)
}
