//! Message handling and update-channel completion.

use crossterm::event::{KeyCode, KeyModifiers};
use graduate::promotion::PromotionAgeReport;
use ratatui::widgets::TableState;

use super::{BranchRow, DiffModel, Effect, Message, SortKey};
use crate::diff::{DiffUpdate, PromotionReport};
use crate::error::CliError;

pub(super) fn update(model: &mut DiffModel, message: Message) -> Result<Effect, CliError> {
    match message {
        Message::Scan(update) => match *update {
            DiffUpdate::Skeleton {
                environment,
                main,
                branches,
            } => {
                model.environment = environment;
                model.main = main;
                model.rows = branches
                    .into_iter()
                    .map(|branch| BranchRow {
                        branch,
                        report: None,
                    })
                    .collect();
                model.selected = 0;
                model.table_state = TableState::default()
                    .with_selected((!model.rows.is_empty()).then_some(model.selected));
            }
            DiffUpdate::Inventory(inventory) => model.inventory = inventory,
            DiffUpdate::Measured(report) => {
                if let Some(row) = model
                    .rows
                    .iter_mut()
                    .find(|row| row.branch == report.branch)
                {
                    row.report = Some(report);
                }
                if model.sort != SortKey::Branch {
                    model.apply_sort();
                }
            }
            DiffUpdate::Jira { branch, state } => {
                if let Some(report) = model
                    .rows
                    .iter_mut()
                    .find(|row| row.branch == branch)
                    .and_then(|row| row.report.as_mut())
                {
                    report.jira = state;
                }
            }
            DiffUpdate::Finished => model.finished = true,
            DiffUpdate::Failed(message) => return Err(CliError::Git(message)),
        },
        Message::MoveUp => model.select(model.selected.saturating_sub(1)),
        Message::MoveDown if model.selected + 1 < model.rows.len() => {
            model.select(model.selected + 1);
        }
        Message::MoveDown => {}
        Message::SelectFirst => model.select(0),
        Message::SelectLast => model.select(model.rows.len().saturating_sub(1)),
        Message::OpenTicket => {
            if let Some(url) = model.selected_issue_url() {
                return Ok(Effect::OpenUrl(url.to_owned()));
            }
            model.warning =
                Some("The selected branch does not have a loaded Jira ticket.".to_owned());
        }
        Message::OpenTicketFailed(message) => model.warning = Some(message),
        Message::CycleSort => {
            model.sort = model.sort.next();
            model.apply_sort();
        }
        Message::OpenHistory => {
            if model
                .rows
                .get(model.selected)
                .and_then(|row| row.report.as_ref())
                .is_some()
            {
                model.history_open = true;
                model.history_selected = 0;
                model.history_list_state = TableState::default().with_selected(Some(0));
            } else {
                model.warning = Some("Measuring branch history…".to_owned());
            }
        }
        Message::CloseHistory => {
            model.history_open = false;
            model.history_selected = 0;
            model.history_list_state = TableState::default();
        }
        Message::OpenAgeReport if !model.finished => {
            model.warning = Some("The age report is available when the scan completes.".to_owned());
        }
        Message::OpenAgeReport => {
            let branches = model
                .rows
                .iter()
                .filter_map(|row| row.report.clone())
                .collect::<Vec<_>>();
            model.age_report = Some(
                PromotionAgeReport::new(&model.inventory.ahead, &branches, model.as_of).map_err(
                    |error| CliError::Git(format!("could not build age report: {error}")),
                )?,
            );
            model.age_selected = 0;
            model.age_list_state = TableState::default().with_selected(Some(0));
            model.warning = None;
        }
        Message::CloseAgeReport => {
            model.age_report = None;
            model.age_selected = 0;
            model.age_list_state = TableState::default();
        }
        Message::ScrollAgeUp => {
            model.age_selected = model.age_selected.saturating_sub(1);
            model.age_list_state.select(Some(model.age_selected));
        }
        Message::ScrollAgeDown => {
            let maximum = model
                .age_report
                .as_ref()
                .map_or(0, |report| report.buckets.len().saturating_add(1));
            model.age_selected = model.age_selected.saturating_add(1).min(maximum);
            model.age_list_state.select(Some(model.age_selected));
        }
        Message::ScrollHistoryUp => {
            model.history_selected = model.history_selected.saturating_sub(1);
            model
                .history_list_state
                .select(Some(model.history_selected));
        }
        Message::ScrollHistoryDown => {
            let maximum = model
                .rows
                .get(model.selected)
                .and_then(|row| row.report.as_ref())
                .map_or(0, |report| report.commits.len().saturating_sub(1));
            model.history_selected = model.history_selected.saturating_add(1).min(maximum);
            model
                .history_list_state
                .select(Some(model.history_selected));
        }
        Message::Tick if !model.finished => model.frame = model.frame.wrapping_add(1),
        Message::Tick => {}
        Message::Finish => return Ok(Effect::Finish),
        Message::Cancel => return Ok(Effect::Cancel),
    }
    Ok(Effect::None)
}

pub(super) fn message_for_key(
    model: &DiffModel,
    code: KeyCode,
    modifiers: KeyModifiers,
) -> Option<Message> {
    if modifiers.contains(KeyModifiers::CONTROL) && code == KeyCode::Char('c') {
        return Some(Message::Cancel);
    }
    match code {
        KeyCode::Char('q') | KeyCode::Char('a') | KeyCode::Esc if model.age_report.is_some() => {
            Some(Message::CloseAgeReport)
        }
        KeyCode::Up | KeyCode::Char('k') if model.age_report.is_some() => {
            Some(Message::ScrollAgeUp)
        }
        KeyCode::Down | KeyCode::Char('j') if model.age_report.is_some() => {
            Some(Message::ScrollAgeDown)
        }
        _ if model.age_report.is_some() => None,
        KeyCode::Char('q') | KeyCode::Char('h') | KeyCode::Esc if model.history_open => {
            Some(Message::CloseHistory)
        }
        KeyCode::Up | KeyCode::Char('k') if model.history_open => Some(Message::ScrollHistoryUp),
        KeyCode::Down | KeyCode::Char('j') if model.history_open => {
            Some(Message::ScrollHistoryDown)
        }
        KeyCode::Char('q') | KeyCode::Esc => Some(Message::Finish),
        KeyCode::Up | KeyCode::Char('k') => Some(Message::MoveUp),
        KeyCode::Down | KeyCode::Char('j') => Some(Message::MoveDown),
        KeyCode::Home => Some(Message::SelectFirst),
        KeyCode::End => Some(Message::SelectLast),
        KeyCode::Char('o') => Some(Message::OpenTicket),
        KeyCode::Char('s') if !model.history_open => Some(Message::CycleSort),
        KeyCode::Char('h') => Some(Message::OpenHistory),
        KeyCode::Char('a') => Some(Message::OpenAgeReport),
        _ => None,
    }
}

pub(super) fn finish_after_update_channel_closes(
    model: DiffModel,
) -> Result<PromotionReport, CliError> {
    if model.finished {
        Ok(model.completed_report())
    } else {
        Err(CliError::Git(
            "promotion report ended before the scan completed".to_owned(),
        ))
    }
}
