//! Streaming terminal list for environment promotion reports.

use std::cmp::Ordering;
use std::time::Duration;

use crossterm::event::{Event, EventStream, KeyEventKind};
use futures_util::StreamExt;
use graduate::promotion::{
    EnvironmentInventory, JiraIssueState, PromotionAgeReport, PromotionBranch, ReportDate,
};
use ratatui::widgets::TableState;
use tokio::sync::mpsc;

use crate::diff::{current_report_date, DiffUpdate, PromotionReport};
use crate::shared::browser::BrowserLauncher;
use crate::shared::error::CliError;
use crate::shared::terminal::StderrTerminal;
use crate::shared::terminal_text;
use render::draw;
use update::{finish_after_update_channel_closes, message_for_key, update};

mod age_report;
mod history;
mod inspector;
mod render;
mod table;
#[cfg(test)]
mod tests;
mod update;

const TICK_RATE: Duration = Duration::from_millis(120);

const SPINNER: [&str; 4] = ["◐", "◓", "◑", "◒"];

const SPACE_1X: u16 = 1;

const SPACE_2X: u16 = SPACE_1X * 2;

const MIN_HISTORY_HEIGHT: u16 = 12;

const MASTER_DETAIL_MIN_WIDTH: u16 = 100;

const MASTER_DETAIL_MAX_HEIGHT: u16 = 32;

struct BranchRow {
    branch: String,
    report: Option<PromotionBranch>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SortKey {
    Branch,
    Started,
    Last,
    Ahead,
}

impl SortKey {
    fn next(self) -> Self {
        match self {
            Self::Branch => Self::Started,
            Self::Started => Self::Last,
            Self::Last => Self::Ahead,
            Self::Ahead => Self::Branch,
        }
    }

    fn indicator(self) -> &'static str {
        match self {
            Self::Ahead => " ▼",
            _ => " ▲",
        }
    }
}

fn compare_rows(sort: SortKey, a: &BranchRow, b: &BranchRow) -> Ordering {
    let by_branch = a.branch.cmp(&b.branch);
    match sort {
        SortKey::Branch => by_branch,
        SortKey::Started | SortKey::Last | SortKey::Ahead => match (&a.report, &b.report) {
            (Some(left), Some(right)) => match sort {
                SortKey::Started => left.started.cmp(&right.started).then(by_branch),
                SortKey::Last => left.last.cmp(&right.last).then(by_branch),
                _ => right.ahead.cmp(&left.ahead).then(by_branch),
            },
            (Some(_), None) => Ordering::Less,
            (None, Some(_)) => Ordering::Greater,
            (None, None) => by_branch,
        },
    }
}

struct DiffModel {
    environment: String,
    main: String,
    inventory: EnvironmentInventory,
    rows: Vec<BranchRow>,
    sort: SortKey,
    selected: usize,
    table_state: TableState,
    finished: bool,
    frame: usize,
    warning: Option<String>,
    history_open: bool,
    history_selected: usize,
    history_list_state: TableState,
    as_of: ReportDate,
    age_report: Option<PromotionAgeReport>,
    age_selected: usize,
    age_list_state: TableState,
}

enum Message {
    Scan(Box<DiffUpdate>),
    MoveUp,
    MoveDown,
    SelectFirst,
    SelectLast,
    OpenTicket,
    OpenTicketFailed(String),
    CycleSort,
    OpenHistory,
    CloseHistory,
    OpenAgeReport,
    CloseAgeReport,
    ScrollAgeUp,
    ScrollAgeDown,
    ScrollHistoryUp,
    ScrollHistoryDown,
    Tick,
    Finish,
    Cancel,
}

enum Effect {
    None,
    OpenUrl(String),
    Finish,
    Cancel,
}

impl DiffModel {
    fn new(as_of: ReportDate) -> Self {
        Self {
            environment: String::new(),
            main: String::new(),
            inventory: EnvironmentInventory::default(),
            rows: Vec::new(),
            sort: SortKey::Branch,
            selected: 0,
            table_state: TableState::default(),
            finished: false,
            frame: 0,
            warning: None,
            history_open: false,
            history_selected: 0,
            history_list_state: TableState::default(),
            as_of,
            age_report: None,
            age_selected: 0,
            age_list_state: TableState::default(),
        }
    }

    fn select(&mut self, selected: usize) {
        if selected != self.selected {
            self.selected = selected;
            self.table_state.select(Some(selected));
            self.warning = None;
            self.history_open = false;
            self.history_selected = 0;
            self.history_list_state = TableState::default();
            self.age_report = None;
            self.age_selected = 0;
            self.age_list_state = TableState::default();
        }
    }

    fn selected_issue_url(&self) -> Option<&str> {
        let report = self.rows.get(self.selected)?.report.as_ref()?;
        match &report.jira {
            JiraIssueState::Loaded(issue) => Some(&issue.url),
            _ => None,
        }
    }

    fn apply_sort(&mut self) {
        let selected_branch = self.rows.get(self.selected).map(|row| row.branch.clone());
        self.rows.sort_by(|a, b| compare_rows(self.sort, a, b));
        if let Some(branch) = selected_branch {
            if let Some(position) = self.rows.iter().position(|row| row.branch == branch) {
                self.selected = position;
                self.table_state.select(Some(position));
            }
        }
    }

    fn completed_report(self) -> PromotionReport {
        let mut branches: Vec<PromotionBranch> =
            self.rows.into_iter().filter_map(|row| row.report).collect();
        branches.sort_by(|a, b| a.branch.cmp(&b.branch));
        PromotionReport {
            environment: self.environment,
            main: self.main,
            inventory: self.inventory,
            branches,
        }
    }
}

pub(crate) async fn run(
    mut updates: mpsc::UnboundedReceiver<DiffUpdate>,
    browser: &dyn BrowserLauncher,
) -> Result<PromotionReport, CliError> {
    let mut terminal = StderrTerminal::new()?;
    let mut events = EventStream::new();
    let mut ticker = tokio::time::interval(TICK_RATE);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut model = DiffModel::new(current_report_date()?);
    let mut updates_open = true;

    let result = loop {
        draw(&mut terminal, &mut model)?;
        tokio::select! {
            received = updates.recv(), if updates_open => match received {
                Some(scan_update) => {
                    update(&mut model, Message::Scan(Box::new(scan_update)))?;
                    if model.finished {
                        updates_open = false;
                    }
                }
                None => {
                    break finish_after_update_channel_closes(model);
                }
            },
            event = events.next() => {
                let event = event.ok_or(CliError::ReportCancelled)??;
                if let Event::Key(key) = event {
                    if !matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) {
                        continue;
                    }
                    let Some(message) = message_for_key(&model, key.code, key.modifiers) else {
                        continue;
                    };
                    match update(&mut model, message)? {
                        Effect::None => {}
                        Effect::OpenUrl(url) => {
                            if let Err(error) = browser.open(&url) {
                                update(
                                    &mut model,
                                    Message::OpenTicketFailed(format!(
                                        "Could not open Jira: {}",
                                        terminal_text::escape(&error.to_string())
                                    )),
                                )?;
                            }
                        }
                        Effect::Finish => break Ok(model.completed_report()),
                        Effect::Cancel => break Err(CliError::ReportCancelled),
                    }
                }
            },
            _ = ticker.tick(), if !model.finished => {
                update(&mut model, Message::Tick)?;
            }
        }
    };
    let restore = terminal.restore();
    match (result, restore) {
        (Err(error), _) => Err(error),
        (Ok(_), Err(error)) => Err(CliError::Io(error)),
        (Ok(rows), Ok(())) => Ok(rows),
    }
}
