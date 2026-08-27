//! Scan update streaming for interactive and plain output.

use std::collections::HashMap;
use std::sync::Arc;

use graduate::jira::JiraCredentials;
use graduate::promotion::{EnvironmentInventory, JiraIssueState};
use tokio::sync::mpsc;
use tokio::task::JoinSet;

use super::{scan_repository, DiffUpdate, PromotionReport, ScanOptions};
use crate::shared::error::CliError;
use crate::shared::git_process::fetch_remote as fetch_remote_name;
use crate::shared::jira::JiraClient;

pub(super) async fn coordinate_scan(
    options: ScanOptions,
    credentials: Option<JiraCredentials>,
    output: mpsc::UnboundedSender<DiffUpdate>,
) -> Result<(), CliError> {
    if options.fetch_before_scan {
        let remote = options.remote.clone();
        let fetch_result = tokio::task::spawn_blocking(move || fetch_remote_name(&remote, true))
            .await
            .map_err(|error| CliError::Git(format!("Git fetch task failed: {error}")))?;
        if let Err(error) = fetch_result {
            let _ = output.send(DiffUpdate::Failed(error.to_string()));
            return Err(error);
        }
    }
    let (scan_tx, mut scan_rx) = mpsc::unbounded_channel();
    let scan_task = tokio::task::spawn_blocking(move || scan_repository(&options, &scan_tx));

    let mut jira_tasks = JoinSet::new();
    let jira = match credentials.as_ref().map(JiraClient::new).transpose() {
        Ok(client) => client.map(Arc::new),
        Err(error) => {
            let _ = output.send(DiffUpdate::Failed(error.to_string()));
            return Err(error);
        }
    };
    let credentials = credentials.map(Arc::new);
    while let Some(update) = scan_rx.recv().await {
        if let DiffUpdate::Measured(row) = &update {
            if let (Some(credentials), Some(jira), Some(key)) =
                (credentials.clone(), jira.clone(), row.jira.key())
            {
                if jira_tasks.len() >= 8 {
                    if let Err(error) = forward_jira_result(&output, jira_tasks.join_next().await) {
                        jira_tasks.abort_all();
                        return if matches!(error, CliError::ReportCancelled) {
                            Ok(())
                        } else {
                            Err(error)
                        };
                    }
                }
                let branch = row.branch.clone();
                let key = key.to_owned();
                jira_tasks.spawn(async move {
                    let result = jira.issue(&credentials, &key).await;
                    let state = jira_issue_state(key, result);
                    (branch, state)
                });
            }
        }
        let failed = matches!(update, DiffUpdate::Failed(_));
        if output.send(update).is_err() || failed {
            jira_tasks.abort_all();
            return Ok(());
        }
    }

    match scan_task.await {
        Ok(Ok(())) => {}
        Ok(Err(error)) => {
            let _ = output.send(DiffUpdate::Failed(error.to_string()));
            jira_tasks.abort_all();
            return Err(error);
        }
        Err(error) => {
            let message = format!("Git scan task failed: {error}");
            let _ = output.send(DiffUpdate::Failed(message.clone()));
            jira_tasks.abort_all();
            return Err(CliError::Git(message));
        }
    }

    while let Some(result) = jira_tasks.join_next().await {
        if let Err(error) = forward_jira_result(&output, Some(result)) {
            jira_tasks.abort_all();
            return if matches!(error, CliError::ReportCancelled) {
                Ok(())
            } else {
                Err(error)
            };
        }
    }
    let _ = output.send(DiffUpdate::Finished);
    Ok(())
}

pub(super) fn jira_issue_state(
    key: String,
    result: Result<graduate::promotion::JiraIssueSummary, CliError>,
) -> JiraIssueState {
    match result {
        Ok(issue) => JiraIssueState::Loaded(issue),
        Err(CliError::JiraStatus(404)) => JiraIssueState::NotFound { key },
        Err(error) => JiraIssueState::Failed {
            key,
            message: error.to_string(),
        },
    }
}

fn forward_jira_result(
    output: &mpsc::UnboundedSender<DiffUpdate>,
    result: Option<Result<(String, JiraIssueState), tokio::task::JoinError>>,
) -> Result<(), CliError> {
    match result {
        Some(Ok((branch, state))) => output
            .send(DiffUpdate::Jira { branch, state })
            .map_err(|_| CliError::ReportCancelled),
        Some(Err(error)) => {
            let message = format!("Jira enrichment task failed: {error}");
            let _ = output.send(DiffUpdate::Failed(message.clone()));
            Err(CliError::Git(message))
        }
        None => Err(CliError::Git(
            "Jira enrichment queue ended unexpectedly".to_owned(),
        )),
    }
}

pub(super) async fn collect_plain(
    mut updates: mpsc::UnboundedReceiver<DiffUpdate>,
) -> Result<PromotionReport, CliError> {
    let mut rows = HashMap::new();
    let mut environment = String::new();
    let mut main = String::new();
    let mut inventory = EnvironmentInventory::default();
    let mut finished = false;
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
            DiffUpdate::Inventory(next_inventory) => inventory = next_inventory,
            DiffUpdate::Measured(row) => {
                rows.insert(row.branch.clone(), row);
            }
            DiffUpdate::Jira { branch, state } => {
                if let Some(row) = rows.get_mut(&branch) {
                    row.jira = state;
                }
            }
            DiffUpdate::Finished => {
                finished = true;
                break;
            }
            DiffUpdate::Failed(message) => return Err(CliError::Git(message)),
        }
    }
    if !finished {
        return Err(CliError::Git(
            "promotion report ended before the scan completed".to_owned(),
        ));
    }
    let mut rows = rows.into_values().collect::<Vec<_>>();
    rows.sort_by(|left, right| left.branch.cmp(&right.branch));
    Ok(PromotionReport {
        environment,
        main,
        inventory,
        branches: rows,
    })
}
