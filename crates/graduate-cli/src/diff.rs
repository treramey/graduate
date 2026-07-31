//! Environment-to-main promotion report orchestration and Git adapters.

use std::collections::{HashMap, HashSet, VecDeque};
use std::io::{self, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;

use gix::bstr::ByteSlice;
use graduate::jira::JiraCredentials;
use graduate::promotion::{jira_key_from_branch, JiraIssueState, PromotionBranch};
use tokio::sync::mpsc;
use tokio::task::JoinSet;

use crate::browser::SystemBrowserLauncher;
use crate::cli::{DiffArgs, ReportFormat};
use crate::config::Config;
use crate::diff_tui;
use crate::error::CliError;
use crate::jira::JiraClient;

#[derive(Clone, Debug)]
pub(crate) enum DiffUpdate {
    Skeleton {
        environment: String,
        main: String,
        branches: Vec<String>,
    },
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
    pub(crate) branches: Vec<PromotionBranch>,
}

struct ScanOptions {
    repository: PathBuf,
    environment: String,
    main: Option<String>,
    remote: String,
    jira_configured: bool,
}

pub(crate) async fn run(args: DiffArgs, config_path: &Path) -> Result<(), CliError> {
    validate_ref_component("environment", &args.environment)?;
    validate_ref_component("remote", &args.remote)?;
    if let Some(main) = &args.main {
        validate_ref_component("main branch", main)?;
    }
    let interactive = args.output_format.is_none()
        && args.output.is_none()
        && io::stdin().is_terminal()
        && io::stderr().is_terminal();
    if !args.no_fetch {
        fetch_remote(&args, interactive)?;
    }

    let credentials = Config::load(config_path)?.jira_credentials()?;
    let (updates_tx, updates_rx) = mpsc::unbounded_channel();
    let scan = ScanOptions {
        repository: std::env::current_dir()?,
        environment: args.environment.clone(),
        main: args.main.clone(),
        remote: args.remote.clone(),
        jira_configured: credentials.is_some(),
    };
    tokio::spawn(coordinate_scan(scan, credentials, updates_tx));

    let report = if interactive {
        diff_tui::run(updates_rx, &SystemBrowserLauncher).await?
    } else {
        collect_plain(updates_rx).await?
    };
    if !interactive {
        write_report(
            &report,
            args.output_format.unwrap_or(ReportFormat::Json),
            args.output.as_deref(),
        )?;
    }
    Ok(())
}

async fn coordinate_scan(
    options: ScanOptions,
    credentials: Option<JiraCredentials>,
    output: mpsc::UnboundedSender<DiffUpdate>,
) {
    let (scan_tx, mut scan_rx) = mpsc::unbounded_channel();
    tokio::task::spawn_blocking(move || {
        if let Err(error) = scan_repository(&options, &scan_tx) {
            let _ = scan_tx.send(DiffUpdate::Failed(error.to_string()));
        }
    });

    let mut jira_tasks = JoinSet::new();
    let jira_slots = Arc::new(tokio::sync::Semaphore::new(8));
    while let Some(update) = scan_rx.recv().await {
        if let DiffUpdate::Measured(row) = &update {
            if let (Some(credentials), Some(key)) = (credentials.clone(), row.jira.key()) {
                let branch = row.branch.clone();
                let key = key.to_owned();
                let jira_slots = Arc::clone(&jira_slots);
                jira_tasks.spawn(async move {
                    let permit = jira_slots.acquire_owned().await;
                    if permit.is_err() {
                        return (
                            branch,
                            JiraIssueState::Failed {
                                key,
                                message: "Jira query queue closed".to_owned(),
                            },
                        );
                    }
                    let result = match JiraClient::new(&credentials) {
                        Ok(client) => client.issue(&credentials, &key).await,
                        Err(error) => Err(error),
                    };
                    let state = match result {
                        Ok(issue) => JiraIssueState::Loaded(issue),
                        Err(error) => JiraIssueState::Failed {
                            key,
                            message: error.to_string(),
                        },
                    };
                    (branch, state)
                });
            }
        }
        let failed = matches!(update, DiffUpdate::Failed(_));
        if output.send(update).is_err() || failed {
            jira_tasks.abort_all();
            return;
        }
    }

    while let Some(result) = jira_tasks.join_next().await {
        match result {
            Ok((branch, state)) => {
                if output.send(DiffUpdate::Jira { branch, state }).is_err() {
                    return;
                }
            }
            Err(error) if error.is_cancelled() => return,
            Err(error) => {
                let _ = output.send(DiffUpdate::Failed(format!(
                    "Jira enrichment task failed: {error}"
                )));
                return;
            }
        }
    }
    let _ = output.send(DiffUpdate::Finished);
}

fn scan_repository(
    options: &ScanOptions,
    updates: &mpsc::UnboundedSender<DiffUpdate>,
) -> Result<(), CliError> {
    let repository = gix::discover(&options.repository).map_err(gitoxide_error)?;
    let prefix = format!("refs/remotes/{}/", options.remote);
    let environment_ref = format!("{prefix}{}", options.environment);
    let environment_id = reference_id(&repository, &environment_ref)
        .map_err(|_| CliError::Git(format!("{environment_ref} does not exist after fetching")))?;
    let main = resolve_main_branch(&repository, &prefix, options.main.as_deref())?;
    let main_ref = format!("{prefix}{main}");
    let main_id = reference_id(&repository, &main_ref)?;
    let environment_ancestors = ancestors(&repository, environment_id)?;
    let main_ancestors = ancestors(&repository, main_id)?;

    let mut candidates = Vec::new();
    let references = repository.references().map_err(gitoxide_error)?;
    let references = references
        .prefixed(prefix.as_str())
        .map_err(gitoxide_error)?;
    for reference in references {
        let mut reference = reference.map_err(gitoxide_error)?;
        let full_name = reference.name().as_bstr().to_str_lossy();
        let Some(branch) = full_name.strip_prefix(&prefix).map(str::to_owned) else {
            continue;
        };
        if excluded_branch(&branch, &options.environment, &main) {
            continue;
        }
        let id = reference.peel_to_id().map_err(gitoxide_error)?.detach();
        if environment_ancestors.contains(&id) && !main_ancestors.contains(&id) {
            candidates.push((branch, id));
        }
    }
    candidates.sort_by(|left, right| left.0.cmp(&right.0));
    updates
        .send(DiffUpdate::Skeleton {
            environment: options.environment.clone(),
            main: main.clone(),
            branches: candidates.iter().map(|(name, _)| name.clone()).collect(),
        })
        .map_err(|_| CliError::ReportCancelled)?;

    for (branch, id) in candidates {
        let jira = match jira_key_from_branch(&branch) {
            Some(key) if options.jira_configured => JiraIssueState::Loading { key },
            Some(key) => JiraIssueState::NotConfigured { key },
            None => JiraIssueState::NoTicket,
        };
        let row = measure_branch(&repository, &main_ancestors, branch, id, jira)?;
        updates
            .send(DiffUpdate::Measured(row))
            .map_err(|_| CliError::ReportCancelled)?;
    }
    Ok(())
}

fn resolve_main_branch(
    repository: &gix::Repository,
    prefix: &str,
    explicit: Option<&str>,
) -> Result<String, CliError> {
    if let Some(explicit) = explicit {
        reference_id(repository, &format!("{prefix}{explicit}"))?;
        return Ok(explicit.to_owned());
    }
    if let Ok(reference) = repository.find_reference(&format!("{prefix}HEAD")) {
        if let Some(target) = reference.target().try_name() {
            let target = target.as_bstr().to_str_lossy();
            if let Some(branch) = target.strip_prefix(prefix) {
                return Ok(branch.to_owned());
            }
        }
    }
    for candidate in ["main", "master", "trunk", "develop"] {
        if reference_id(repository, &format!("{prefix}{candidate}")).is_ok() {
            return Ok(candidate.to_owned());
        }
    }
    Err(CliError::Git(
        "could not determine the main branch from the remote default or common names; pass --main <BRANCH>"
            .to_owned(),
    ))
}

fn reference_id(repository: &gix::Repository, name: &str) -> Result<gix::ObjectId, CliError> {
    let mut reference = repository.find_reference(name).map_err(gitoxide_error)?;
    reference
        .peel_to_id()
        .map(|id| id.detach())
        .map_err(gitoxide_error)
}

fn ancestors(
    repository: &gix::Repository,
    start: gix::ObjectId,
) -> Result<HashSet<gix::ObjectId>, CliError> {
    let mut found = HashSet::new();
    let mut pending = VecDeque::from([start]);
    while let Some(id) = pending.pop_front() {
        if !found.insert(id) {
            continue;
        }
        let commit = repository.find_commit(id).map_err(gitoxide_error)?;
        pending.extend(commit.parent_ids().map(|parent| parent.detach()));
    }
    Ok(found)
}

fn measure_branch(
    repository: &gix::Repository,
    main_ancestors: &HashSet<gix::ObjectId>,
    branch: String,
    tip: gix::ObjectId,
    jira: JiraIssueState,
) -> Result<PromotionBranch, CliError> {
    let tip_commit = repository.find_commit(tip).map_err(gitoxide_error)?;
    let author = tip_commit.author().map_err(gitoxide_error)?;
    let author_time = author.time().map_err(gitoxide_error)?;
    let last = unix_date(author_time.seconds);
    let last_author = author.name.to_str_lossy().into_owned();
    let mut unique = HashSet::new();
    let mut pending = VecDeque::from([tip]);
    let mut started_seconds = author_time.seconds;
    while let Some(id) = pending.pop_front() {
        if main_ancestors.contains(&id) || !unique.insert(id) {
            continue;
        }
        let commit = repository.find_commit(id).map_err(gitoxide_error)?;
        let commit_author = commit.author().map_err(gitoxide_error)?;
        started_seconds =
            started_seconds.min(commit_author.time().map_err(gitoxide_error)?.seconds);
        pending.extend(commit.parent_ids().map(|parent| parent.detach()));
    }
    Ok(PromotionBranch {
        branch,
        started: unix_date(started_seconds),
        last,
        ahead: unique.len(),
        last_author,
        jira,
    })
}

fn excluded_branch(branch: &str, environment: &str, main: &str) -> bool {
    branch == "HEAD"
        || branch == environment
        || branch == main
        || matches!(branch, "qa" | "staging" | "cycle")
        || branch.starts_with("backup/")
}

fn unix_date(seconds: i64) -> String {
    let days = seconds.div_euclid(86_400);
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let day_of_era = z - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let mut year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = month_prime + if month_prime < 10 { 3 } else { -9 };
    year += i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}")
}

fn fetch_remote(args: &DiffArgs, interactive: bool) -> Result<(), CliError> {
    let pat = std::env::var("GIT_PAT")
        .ok()
        .filter(|value| !value.is_empty());
    if pat.is_some() {
        eprintln!(
            "Contacting {} with the supplied PAT (non-interactive)…",
            args.remote
        );
    } else if !interactive {
        eprintln!(
            "Contacting {} (unattended; cached credentials only)…",
            args.remote
        );
    } else {
        eprintln!(
            "Contacting {}… An authentication window may open.",
            args.remote
        );
    }

    if let Some(pat) = pat {
        let mut authenticated = Command::new("git");
        authenticated
            .args([
                "-c",
                "credential.helper=",
                "-c",
                "credential.helper=!f() { echo username=x-access-token; echo \"password=$GIT_PAT\"; }; f",
                "fetch",
                "--prune",
                &args.remote,
            ])
            .env("GIT_PAT", pat)
            .env("GIT_TERMINAL_PROMPT", "0")
            .stdout(Stdio::null());
        return check_fetch(authenticated.status(), true, !interactive);
    }
    if !interactive {
        let mut unattended = Command::new("git");
        unattended
            .args([
                "-c",
                "credential.interactive=false",
                "fetch",
                "--prune",
                &args.remote,
            ])
            .env("GIT_TERMINAL_PROMPT", "0")
            .stdout(Stdio::null());
        return check_fetch(unattended.status(), false, true);
    }
    let mut command = Command::new("git");
    command
        .args(["fetch", "--prune", &args.remote])
        .stdout(Stdio::null());
    check_fetch(command.status(), false, false)
}

fn check_fetch(
    result: io::Result<std::process::ExitStatus>,
    used_pat: bool,
    unattended: bool,
) -> Result<(), CliError> {
    match result {
        Ok(status) if status.success() => Ok(()),
        Ok(_) if used_pat => Err(CliError::Git(
            "could not fetch the remote; the PAT was rejected, expired, or lacks repository read access"
                .to_owned(),
        )),
        Ok(_) if unattended => Err(CliError::Git(
            "could not fetch the remote with cached credentials; set GIT_PAT for a headless run"
                .to_owned(),
        )),
        Ok(_) => Err(CliError::Git(
            "could not fetch the remote; complete authentication and run the command again"
                .to_owned(),
        )),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Err(CliError::Git(
            "could not run git fetch because the git executable was not found".to_owned(),
        )),
        Err(error) => Err(CliError::Io(error)),
    }
}

fn validate_ref_component(label: &str, value: &str) -> Result<(), CliError> {
    if value.trim().is_empty() || value.starts_with('-') || value.chars().any(char::is_control) {
        return Err(CliError::InvalidInput(format!(
            "{label} must be a non-empty Git branch or remote name"
        )));
    }
    Ok(())
}

fn gitoxide_error(error: impl std::fmt::Display) -> CliError {
    CliError::Git(error.to_string())
}

async fn collect_plain(
    mut updates: mpsc::UnboundedReceiver<DiffUpdate>,
) -> Result<PromotionReport, CliError> {
    let mut rows = HashMap::new();
    let mut environment = String::new();
    let mut main = String::new();
    while let Some(update) = updates.recv().await {
        match update {
            DiffUpdate::Skeleton {
                environment: next_environment,
                main: next_main,
                ..
            } => {
                environment = next_environment;
                main = next_main;
            }
            DiffUpdate::Measured(row) => {
                rows.insert(row.branch.clone(), row);
            }
            DiffUpdate::Jira { branch, state } => {
                if let Some(row) = rows.get_mut(&branch) {
                    row.jira = state;
                }
            }
            DiffUpdate::Finished => break,
            DiffUpdate::Failed(message) => return Err(CliError::Git(message)),
        }
    }
    let mut rows = rows.into_values().collect::<Vec<_>>();
    rows.sort_by(|left, right| left.branch.cmp(&right.branch));
    Ok(PromotionReport {
        environment,
        main,
        branches: rows,
    })
}

fn write_report(
    report: &PromotionReport,
    format: ReportFormat,
    output: Option<&Path>,
) -> Result<(), CliError> {
    let content = match format {
        ReportFormat::Json => format!("{}\n", serde_json::to_string_pretty(&report_value(report))?),
        ReportFormat::Yaml => serde_yaml::to_string(&report_value(report))?,
        ReportFormat::Table => format_table(report),
        ReportFormat::Csv => format_csv(report)?,
    };
    if let Some(output) = output {
        let output = validate_output_path(output)?;
        std::fs::write(&output, content)?;
        eprintln!("Wrote {}", output.display());
    } else {
        print!("{content}");
    }
    Ok(())
}

fn report_value(report: &PromotionReport) -> serde_json::Value {
    let branches = report
        .branches
        .iter()
        .map(|branch| {
            let (issue, issue_key, jira_error) = match &branch.jira {
                JiraIssueState::Loaded(issue) => (
                    serde_json::json!({
                        "key": issue.key,
                        "self": issue.api_url,
                        "browseUrl": issue.url,
                        "fields": {
                            "summary": issue.summary,
                            "status": { "name": issue.status },
                            "assignee": issue.assignee.as_ref().map(|display_name| {
                                serde_json::json!({ "displayName": display_name })
                            }),
                            "fixVersions": issue.fix_versions.iter().map(|name| {
                                serde_json::json!({ "name": name })
                            }).collect::<Vec<_>>()
                        }
                    }),
                    serde_json::Value::Null,
                    serde_json::Value::Null,
                ),
                JiraIssueState::Failed { key, message } => (
                    serde_json::Value::Null,
                    serde_json::json!(key),
                    serde_json::json!(message),
                ),
                JiraIssueState::NotConfigured { key } | JiraIssueState::Loading { key } => (
                    serde_json::Value::Null,
                    serde_json::json!(key),
                    serde_json::Value::Null,
                ),
                JiraIssueState::NoTicket => (
                    serde_json::Value::Null,
                    serde_json::Value::Null,
                    serde_json::Value::Null,
                ),
            };
            serde_json::json!({
                "branch": branch.branch,
                "started": branch.started,
                "last": branch.last,
                "ahead": branch.ahead,
                "lastAuthor": branch.last_author,
                "jiraIssue": issue,
                "jiraIssueKey": issue_key,
                "jiraError": jira_error,
            })
        })
        .collect::<Vec<_>>();
    serde_json::json!({
        "environment": report.environment,
        "main": report.main,
        "branches": branches,
    })
}

fn format_table(report: &PromotionReport) -> String {
    let mut output = format!(
        "Branches in {} but not {}\n{:<36} {:<10} {:<10} {:>5}  {:<12} {:<14} LAST AUTHOR\n",
        crate::terminal_text::escape(&report.environment),
        crate::terminal_text::escape(&report.main),
        "BRANCH",
        "STARTED",
        "LAST",
        "AHEAD",
        "JIRA",
        "STATUS"
    );
    if report.branches.is_empty() {
        output.push_str("(nothing to promote)\n");
    }
    for row in &report.branches {
        let key = crate::terminal_text::escape(row.jira.key().unwrap_or("—"));
        let status = match &row.jira {
            JiraIssueState::Loaded(issue) => issue.status.as_str(),
            JiraIssueState::Failed { .. } => "error",
            JiraIssueState::NotConfigured { .. } => "not configured",
            JiraIssueState::Loading { .. } => "loading",
            JiraIssueState::NoTicket => "—",
        };
        output.push_str(&format!(
            "{:<36} {:<10} {:<10} {:>5}  {:<12} {:<14} {}\n",
            crate::terminal_text::escape(&row.branch),
            row.started,
            row.last,
            row.ahead,
            key,
            crate::terminal_text::escape(status),
            crate::terminal_text::escape(&row.last_author)
        ));
    }
    output
}

fn format_csv(report: &PromotionReport) -> Result<String, CliError> {
    let mut file = Vec::new();
    csv_row(
        &mut file,
        &[
            "branch",
            "started",
            "last",
            "ahead",
            "lastAuthor",
            "jiraIssue.key",
            "jiraIssue.fields.status.name",
            "jiraIssue.fields.summary",
            "jiraIssue.fields.assignee.displayName",
            "jiraIssue.fields.fixVersions",
            "jiraIssue.self",
            "jiraIssue.browseUrl",
        ],
    )?;
    for row in &report.branches {
        let ahead = row.ahead.to_string();
        let (key, status, summary, assignee, versions, api_url, browse_url) = match &row.jira {
            JiraIssueState::Loaded(issue) => (
                issue.key.clone(),
                issue.status.clone(),
                issue.summary.clone(),
                issue.assignee.clone().unwrap_or_default(),
                issue.fix_versions.join(", "),
                issue.api_url.clone(),
                issue.url.clone(),
            ),
            JiraIssueState::Failed { key, message } => (
                key.clone(),
                "error".to_owned(),
                message.clone(),
                String::new(),
                String::new(),
                String::new(),
                String::new(),
            ),
            JiraIssueState::NotConfigured { key } => (
                key.clone(),
                "not configured".to_owned(),
                String::new(),
                String::new(),
                String::new(),
                String::new(),
                String::new(),
            ),
            JiraIssueState::Loading { key } => (
                key.clone(),
                "loading".to_owned(),
                String::new(),
                String::new(),
                String::new(),
                String::new(),
                String::new(),
            ),
            JiraIssueState::NoTicket => (
                String::new(),
                String::new(),
                String::new(),
                String::new(),
                String::new(),
                String::new(),
                String::new(),
            ),
        };
        csv_row(
            &mut file,
            &[
                &row.branch,
                &row.started,
                &row.last,
                &ahead,
                &row.last_author,
                &key,
                &status,
                &summary,
                &assignee,
                &versions,
                &api_url,
                &browse_url,
            ],
        )?;
    }
    String::from_utf8(file)
        .map_err(|error| CliError::InvalidInput(format!("CSV was not valid UTF-8: {error}")))
}

fn validate_output_path(path: &Path) -> Result<PathBuf, CliError> {
    if path.as_os_str().is_empty()
        || path.is_absolute()
        || path.components().any(|component| {
            matches!(
                component,
                std::path::Component::ParentDir
                    | std::path::Component::RootDir
                    | std::path::Component::Prefix(_)
            )
        })
        || path.to_string_lossy().chars().any(char::is_control)
    {
        return Err(CliError::InvalidInput(
            "--output must be a relative file path within the current directory".to_owned(),
        ));
    }
    let current = std::env::current_dir()?.canonicalize()?;
    let destination = current.join(path);
    let parent = destination
        .parent()
        .ok_or_else(|| CliError::InvalidInput("--output must name a file".to_owned()))?;
    let parent = parent.canonicalize().map_err(|error| {
        CliError::InvalidInput(format!(
            "--output parent directory must already exist: {error}"
        ))
    })?;
    if !parent.starts_with(&current) {
        return Err(CliError::InvalidInput(
            "--output must stay within the current directory".to_owned(),
        ));
    }
    if std::fs::symlink_metadata(&destination)
        .is_ok_and(|metadata| metadata.file_type().is_symlink())
    {
        return Err(CliError::InvalidInput(
            "--output must not replace a symbolic link".to_owned(),
        ));
    }
    Ok(destination)
}

fn csv_row(writer: &mut impl Write, fields: &[&str]) -> Result<(), CliError> {
    for (index, field) in fields.iter().enumerate() {
        if index > 0 {
            writer.write_all(b",")?;
        }
        writer.write_all(b"\"")?;
        writer.write_all(field.replace('"', "\"\"").as_bytes())?;
        writer.write_all(b"\"")?;
    }
    writer.write_all(b"\n")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::process::Command;

    use super::*;

    #[test]
    fn dates_are_formatted_without_local_timezone_drift() {
        assert_eq!(unix_date(0), "1970-01-01");
        assert_eq!(unix_date(1_704_067_200), "2024-01-01");
    }

    #[test]
    fn csv_quotes_every_field_and_doubles_quotes() -> Result<(), CliError> {
        let mut output = Vec::new();
        csv_row(&mut output, &["branch", "O\"Brien, Pat"])?;
        assert_eq!(output, b"\"branch\",\"O\"\"Brien, Pat\"\n");
        Ok(())
    }

    #[test]
    fn output_rejects_paths_outside_the_current_directory() {
        assert!(validate_output_path(Path::new("../report.json")).is_err());
        assert!(validate_output_path(Path::new("/tmp/report.json")).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn output_rejects_a_parent_symlink_that_escapes_the_repository(
    ) -> Result<(), Box<dyn std::error::Error>> {
        use std::os::unix::fs::symlink;

        let current = std::env::current_dir()?;
        let directory = tempfile::tempdir_in(&current)?;
        let outside = tempfile::tempdir()?;
        symlink(outside.path(), directory.path().join("outside"))?;
        let relative = directory
            .path()
            .strip_prefix(&current)?
            .join("outside/report.json");

        assert!(validate_output_path(&relative).is_err());
        Ok(())
    }

    #[test]
    fn json_report_uses_camel_case_and_jira_api_field_shapes() {
        let report = PromotionReport {
            environment: "qa".to_owned(),
            main: "main".to_owned(),
            branches: vec![PromotionBranch {
                branch: "feature/PROJ-123-login".to_owned(),
                started: "2024-01-01".to_owned(),
                last: "2024-01-02".to_owned(),
                ahead: 2,
                last_author: "Pat".to_owned(),
                jira: JiraIssueState::Loaded(graduate::promotion::JiraIssueSummary {
                    key: "PROJ-123".to_owned(),
                    api_url: "https://example.atlassian.net/rest/api/3/issue/10001".to_owned(),
                    summary: "Add login".to_owned(),
                    status: "Ready for QA".to_owned(),
                    assignee: Some("Pat".to_owned()),
                    fix_versions: vec!["1.2".to_owned()],
                    url: "https://example.atlassian.net/browse/PROJ-123".to_owned(),
                }),
            }],
        };

        let value = report_value(&report);

        assert_eq!(value["branches"][0]["lastAuthor"], "Pat");
        assert_eq!(
            value["branches"][0]["jiraIssue"]["fields"]["assignee"]["displayName"],
            "Pat"
        );
        assert_eq!(
            value["branches"][0]["jiraIssue"]["fields"]["fixVersions"][0]["name"],
            "1.2"
        );
    }

    #[test]
    fn known_environment_and_backup_refs_are_excluded() {
        assert!(excluded_branch("qa", "qa", "main"));
        assert!(excluded_branch("backup/old", "qa", "main"));
        assert!(!excluded_branch("feature/PROJ-1", "qa", "main"));
    }

    #[test]
    fn scan_finds_a_feature_in_environment_but_not_main() -> Result<(), Box<dyn std::error::Error>>
    {
        let directory = tempfile::tempdir()?;
        run_git(directory.path(), &["init", "-q", "-b", "main"])?;
        run_git(directory.path(), &["config", "user.name", "Test Author"])?;
        run_git(
            directory.path(),
            &["config", "user.email", "test@example.com"],
        )?;
        std::fs::write(directory.path().join("file"), "base\n")?;
        run_git(directory.path(), &["add", "file"])?;
        commit(directory.path(), "base", "2024-01-01T00:00:00Z")?;
        run_git(
            directory.path(),
            &["checkout", "-q", "-b", "feature/PROJ-123-login"],
        )?;
        std::fs::write(directory.path().join("file"), "base\nfeature\n")?;
        run_git(directory.path(), &["add", "file"])?;
        commit(directory.path(), "feature", "2024-02-01T00:00:00Z")?;
        run_git(directory.path(), &["checkout", "-q", "-b", "qa", "main"])?;
        run_git(
            directory.path(),
            &[
                "merge",
                "-q",
                "--no-ff",
                "feature/PROJ-123-login",
                "-m",
                "promote",
            ],
        )?;
        for branch in ["main", "qa", "feature/PROJ-123-login"] {
            run_git(
                directory.path(),
                &[
                    "update-ref",
                    &format!("refs/remotes/origin/{branch}"),
                    &format!("refs/heads/{branch}"),
                ],
            )?;
        }
        run_git(
            directory.path(),
            &[
                "symbolic-ref",
                "refs/remotes/origin/HEAD",
                "refs/remotes/origin/main",
            ],
        )?;

        let (sender, mut receiver) = mpsc::unbounded_channel();
        scan_repository(
            &ScanOptions {
                repository: directory.path().to_path_buf(),
                environment: "qa".to_owned(),
                main: None,
                remote: "origin".to_owned(),
                jira_configured: false,
            },
            &sender,
        )?;
        drop(sender);
        let updates = std::iter::from_fn(|| receiver.try_recv().ok()).collect::<Vec<_>>();

        assert!(matches!(
            updates.first(),
            Some(DiffUpdate::Skeleton { branches, main, .. })
                if branches == &["feature/PROJ-123-login"] && main == "main"
        ));
        assert!(matches!(
            updates.get(1),
            Some(DiffUpdate::Measured(PromotionBranch {
                branch,
                ahead: 1,
                started,
                jira: JiraIssueState::NotConfigured { key },
                ..
            })) if branch == "feature/PROJ-123-login"
                && started == "2024-02-01"
                && key == "PROJ-123"
        ));
        Ok(())
    }

    fn run_git(path: &Path, arguments: &[&str]) -> Result<(), Box<dyn std::error::Error>> {
        let status = Command::new("git")
            .args(arguments)
            .current_dir(path)
            .status()?;
        if status.success() {
            Ok(())
        } else {
            Err(format!("git {} failed with {status}", arguments.join(" ")).into())
        }
    }

    fn commit(path: &Path, message: &str, date: &str) -> Result<(), Box<dyn std::error::Error>> {
        let status = Command::new("git")
            .args(["commit", "-q", "-m", message])
            .env("GIT_AUTHOR_DATE", date)
            .env("GIT_COMMITTER_DATE", date)
            .current_dir(path)
            .status()?;
        if status.success() {
            Ok(())
        } else {
            Err(format!("git commit failed with {status}").into())
        }
    }
}
