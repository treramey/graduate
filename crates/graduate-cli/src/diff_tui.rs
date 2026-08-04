//! Streaming terminal list for environment promotion reports.

use std::cmp::Ordering;
use std::time::Duration;

use crossterm::event::{Event, EventStream, KeyCode, KeyEventKind, KeyModifiers};
use futures_util::StreamExt;
use graduate::promotion::{
    systemic_not_found, JiraIssueState, PromotionAgeReport, PromotionBranch, ReportDate,
};
use ratatui::layout::{Constraint, HorizontalAlignment, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, Borders, Cell, Clear, Padding, Paragraph, Row, Table, TableState, Wrap,
};
use ratatui::Frame;
use tokio::sync::mpsc;

use crate::browser::BrowserLauncher;
use crate::diff::{
    age_bucket_label, age_bucket_reading, current_report_date, share_percent, DiffUpdate,
    PromotionReport,
};
use crate::error::CliError;
use crate::terminal::StderrTerminal;
use crate::terminal_text;
use crate::theme::{self, Palette};

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
            branches,
        }
    }
}

fn update(model: &mut DiffModel, message: Message) -> Result<Effect, CliError> {
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
                PromotionAgeReport::new(&branches, model.as_of).map_err(|error| {
                    CliError::Git(format!("could not build age report: {error}"))
                })?,
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

fn message_for_key(model: &DiffModel, code: KeyCode, modifiers: KeyModifiers) -> Option<Message> {
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

fn finish_after_update_channel_closes(model: DiffModel) -> Result<PromotionReport, CliError> {
    if model.finished {
        Ok(model.completed_report())
    } else {
        Err(CliError::Git(
            "promotion report ended before the scan completed".to_owned(),
        ))
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

fn draw(terminal: &mut StderrTerminal, model: &mut DiffModel) -> Result<(), CliError> {
    terminal.terminal_mut().draw(|frame| render(frame, model))?;
    Ok(())
}

fn render(frame: &mut Frame<'_>, model: &mut DiffModel) {
    let area = theme::constrain_content_width(frame.area());
    if frame.area().height <= MASTER_DETAIL_MAX_HEIGHT {
        let [_top_padding, title, _title_margin, main, _footer_margin, footer] =
            Layout::vertical([
                Constraint::Length(SPACE_1X),
                Constraint::Length(1),
                Constraint::Length(SPACE_1X),
                Constraint::Fill(1),
                Constraint::Length(SPACE_1X),
                Constraint::Length(1),
            ])
            .areas(area);
        render_title(frame, title, model, true);
        render_report(frame, main, model);
        render_footer(frame, footer, model);
        if model.age_report.is_some() {
            render_age_report(frame, model);
        } else if model.history_open {
            render_history(frame, model);
        }
        return;
    }
    let [_top_padding, header, _header_padding, title, main, _footer_margin, footer] =
        Layout::vertical([
            Constraint::Length(SPACE_2X),
            Constraint::Length(theme::GRADUATE_ART_HEIGHT),
            Constraint::Length(SPACE_1X),
            Constraint::Length(3),
            Constraint::Fill(1),
            Constraint::Length(SPACE_1X),
            Constraint::Length(3),
        ])
        .areas(area);
    theme::render_brand_header(frame, header);
    render_title(frame, title, model, false);
    render_report(frame, main, model);
    render_footer(frame, footer, model);
    if model.age_report.is_some() {
        render_age_report(frame, model);
    } else if model.history_open {
        render_history(frame, model);
    }
}

fn render_age_report(frame: &mut Frame<'_>, model: &mut DiffModel) {
    let Some(age) = model.age_report.as_ref() else {
        return;
    };
    let outer = frame.area();
    frame
        .buffer_mut()
        .set_style(outer, Style::new().add_modifier(Modifier::DIM));
    let height = outer.height.saturating_sub(4).clamp(1, 34);
    let area = modal_area(outer, height);
    let card = Block::default()
        .borders(Borders::ALL)
        .border_style(Palette::muted())
        .style(Palette::overlay())
        .padding(Padding::new(SPACE_1X, SPACE_1X, 0, 0));
    let content = card.inner(area);
    let [title, context, ages, oldest_title, oldest, footer] = Layout::vertical([
        Constraint::Length(2),
        Constraint::Length(2),
        Constraint::Length(12),
        Constraint::Length(2),
        Constraint::Fill(1),
        Constraint::Length(2),
    ])
    .areas(content);
    let mut age_rows = age
        .buckets
        .iter()
        .map(|bucket| {
            Row::new([
                age_bucket_label(bucket.year),
                bucket.commits.to_string(),
                format!("{:.1}%", share_percent(bucket.commits, age.total_commits)),
                age_bucket_reading(age, bucket),
            ])
        })
        .collect::<Vec<_>>();
    age_rows.push(
        Row::new([
            "Written in last 90 days".to_owned(),
            age.last_90_days.commits.to_string(),
            format!(
                "{:.1}%",
                share_percent(age.last_90_days.commits, age.total_commits)
            ),
            "Genuinely in flight".to_owned(),
        ])
        .style(Palette::text().bold()),
    );
    age_rows.push(
        Row::new([
            "Older than one year".to_owned(),
            age.older_than_one_year.commits.to_string(),
            format!(
                "{:.1}%",
                share_percent(age.older_than_one_year.commits, age.total_commits)
            ),
            "Will not ship without a decision".to_owned(),
        ])
        .style(Palette::text().bold()),
    );
    let age_table = Table::new(
        age_rows,
        [
            Constraint::Length(24),
            Constraint::Length(10),
            Constraint::Length(8),
            Constraint::Min(24),
        ],
    )
    .header(Row::new(["WRITTEN IN", "COMMITS", "SHARE", "READING"]).style(Palette::muted().bold()))
    .column_spacing(1)
    .row_highlight_style(Palette::action_focus())
    .highlight_symbol("› ");
    if content.height < 22 {
        let [compact_title, compact_ages] =
            Layout::vertical([Constraint::Length(1), Constraint::Fill(1)]).areas(content);
        frame.render_widget(Clear, area);
        frame.render_widget(card, area);
        frame.render_widget(
            Paragraph::new(Line::styled(
                "The age of unshipped work — the decisive measure",
                Palette::primary().bold(),
            ))
            .alignment(HorizontalAlignment::Center),
            compact_title,
        );
        frame.render_stateful_widget(age_table, compact_ages, &mut model.age_list_state);
        return;
    }
    let oldest_rows = age.oldest_branches.iter().take(6).map(|branch| {
        Row::new([
            terminal_text::escape(&branch.branch),
            branch.commits.to_string(),
            branch.oldest.to_string(),
            branch.newest.to_string(),
        ])
    });
    let oldest_table = Table::new(
        oldest_rows,
        [
            Constraint::Min(28),
            Constraint::Length(8),
            Constraint::Length(10),
            Constraint::Length(10),
        ],
    )
    .header(Row::new(["BRANCH", "COMMITS", "OLDEST", "NEWEST"]).style(Palette::muted().bold()))
    .column_spacing(1);
    let oldest_year = age
        .oldest_year()
        .map_or_else(String::new, |year| year.to_string());

    frame.render_widget(Clear, area);
    frame.render_widget(card, area);
    frame.render_widget(
        Paragraph::new(Line::styled(
            "The age of unshipped work — the decisive measure",
            Palette::primary().bold(),
        ))
        .alignment(HorizontalAlignment::Center),
        title,
    );
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::raw("All "),
            Span::styled(age.total_commits.to_string(), Palette::text().bold()),
            Span::raw(" unique authored commits in "),
            Span::styled(
                terminal_text::escape(&model.environment),
                Palette::text().bold(),
            ),
            Span::raw(" but not "),
            Span::styled(terminal_text::escape(&model.main), Palette::text().bold()),
            Span::styled(format!("  ·  as of {}", age.as_of), Palette::muted()),
        ]))
        .alignment(HorizontalAlignment::Center),
        context,
    );
    frame.render_stateful_widget(age_table, ages, &mut model.age_list_state);
    let oldest_heading = if age.oldest_branches.len() > 6 {
        format!(
            "Top 6 of {} branches carrying {oldest_year} commits",
            age.oldest_branches.len()
        )
    } else {
        format!("Branches carrying {oldest_year} commits")
    };
    frame.render_widget(
        Paragraph::new(Line::styled(oldest_heading, Palette::primary().bold())),
        oldest_title,
    );
    frame.render_widget(oldest_table, oldest);
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("↑/↓", Palette::primary()),
            Span::raw(" scroll   "),
            Span::styled("Esc/a", Palette::primary()),
            Span::raw(" close age report"),
        ]))
        .alignment(HorizontalAlignment::Center),
        footer,
    );
}

fn render_report(frame: &mut Frame<'_>, area: Rect, model: &mut DiffModel) {
    if area.width >= MASTER_DETAIL_MIN_WIDTH && frame.area().height <= MASTER_DETAIL_MAX_HEIGHT {
        let [table, _gutter, inspector] = Layout::horizontal([
            Constraint::Percentage(62),
            Constraint::Length(SPACE_2X),
            Constraint::Fill(1),
        ])
        .areas(area);
        render_table(frame, table, model, false);
        render_inspector(frame, inspector, model);
    } else {
        let [details, _gutter, table] = Layout::vertical([
            Constraint::Length(8),
            Constraint::Length(SPACE_1X),
            Constraint::Fill(1),
        ])
        .areas(area);
        render_details(frame, details, model);
        render_table(frame, table, model, true);
    }
}

fn render_history(frame: &mut Frame<'_>, model: &mut DiffModel) {
    let Some(report) = model
        .rows
        .get(model.selected)
        .and_then(|row| row.report.as_ref())
    else {
        return;
    };
    let outer = frame.area();
    frame
        .buffer_mut()
        .set_style(outer, Style::new().add_modifier(Modifier::DIM));
    let desired_height = u16::try_from(report.commits.len())
        .unwrap_or(u16::MAX)
        .saturating_add(13);
    let height = desired_height
        .min(24)
        .min(outer.height.saturating_sub(4))
        .max(MIN_HISTORY_HEIGHT.min(outer.height));
    let area = modal_area(outer, height);
    let rows = report.commits.iter().map(|commit| {
        Row::new([
            terminal_text::escape(&commit.short_id),
            terminal_text::escape(&commit.subject),
            terminal_text::escape(&commit.author),
            commit.date.clone(),
        ])
    });
    let count = report.commits.len();
    let noun = if count == 1 { "commit" } else { "commits" };
    let summary = Line::from(vec![
        Span::styled(
            terminal_text::escape(&report.branch),
            Palette::text().bold(),
        ),
        Span::styled(
            format!("  ·  {count} {noun}  ·  newest first"),
            Palette::muted(),
        ),
    ]);
    let position = if count == 0 {
        "0 of 0".to_owned()
    } else {
        format!("{} of {count}", model.history_selected + 1)
    };
    let card = Block::default()
        .borders(Borders::ALL)
        .border_style(Palette::muted())
        .style(Palette::overlay())
        .padding(Padding::new(SPACE_1X, SPACE_1X, 0, 0));
    let content = card.inner(area);
    let [title, context, headings, top_divider, commits, _position_margin_top, bottom_divider, _footer_margin_top, footer, _footer_margin_bottom] =
        Layout::vertical([
            Constraint::Length(2),
            Constraint::Length(2),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Fill(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .areas(content);
    let widths = [
        Constraint::Length(8),
        Constraint::Min(20),
        Constraint::Length(15),
        Constraint::Length(10),
    ];
    let heading_widths = [
        Constraint::Length(1),
        Constraint::Length(8),
        Constraint::Min(20),
        Constraint::Length(15),
        Constraint::Length(10),
    ];
    let heading = Table::new(std::iter::empty::<Row<'static>>(), heading_widths)
        .header(Row::new(["", "SHA", "SUBJECT", "AUTHOR", "DATE"]).style(Palette::muted().bold()));
    let table = Table::new(rows, widths)
        .row_highlight_style(Palette::action_focus())
        .highlight_symbol("› ");
    frame.render_widget(Clear, area);
    frame.render_widget(card, area);
    frame.render_widget(
        Paragraph::new(Line::styled(
            format!("Commits ahead of {}", terminal_text::escape(&model.main)),
            Palette::primary().bold(),
        ))
        .alignment(HorizontalAlignment::Center),
        title,
    );
    frame.render_widget(
        Paragraph::new(summary).alignment(HorizontalAlignment::Center),
        context,
    );
    frame.render_widget(heading, headings);
    render_history_divider(frame, top_divider);
    frame.render_stateful_widget(table, commits, &mut model.history_list_state);
    render_history_position_divider(frame, bottom_divider, &position);
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("↑/↓", Palette::primary()),
            Span::raw(" move   "),
            Span::styled("Esc/h", Palette::primary()),
            Span::raw(" close history"),
        ]))
        .alignment(HorizontalAlignment::Center),
        footer,
    );
}

fn modal_area(outer: Rect, height: u16) -> Rect {
    let viewport = theme::constrain_content_width(outer);
    Rect::new(
        viewport.x,
        outer.y + outer.height.saturating_sub(height) / 2,
        viewport.width.max(1),
        height,
    )
}

fn render_history_divider(frame: &mut Frame<'_>, area: Rect) {
    frame.render_widget(
        Block::default()
            .borders(Borders::TOP)
            .border_style(Palette::muted()),
        area,
    );
}

fn render_history_position_divider(frame: &mut Frame<'_>, area: Rect, position: &str) {
    frame.render_widget(
        Block::default()
            .borders(Borders::TOP)
            .border_style(Palette::muted())
            .title(Line::from(format!(" {position} ")).alignment(HorizontalAlignment::Center)),
        area,
    );
}

fn render_title(frame: &mut Frame<'_>, area: Rect, model: &DiffModel, center_summary: bool) {
    let measured = model.rows.iter().filter(|row| row.report.is_some()).count();
    let pending_jira = model
        .rows
        .iter()
        .filter(|row| {
            row.report
                .as_ref()
                .is_some_and(|report| matches!(report.jira, JiraIssueState::Loading { .. }))
        })
        .count();
    let status = if model.finished {
        format!("{} branches  ·  complete", model.rows.len())
    } else if measured == model.rows.len() && pending_jira > 0 {
        format!(
            "{} loading Jira for {pending_jira} tickets",
            SPINNER[model.frame % SPINNER.len()]
        )
    } else {
        format!(
            "{} measuring {measured}/{}",
            SPINNER[model.frame % SPINNER.len()],
            model.rows.len()
        )
    };
    let summary = Line::from(vec![
        Span::raw("In "),
        Span::styled(
            terminal_text::escape(&model.environment),
            Palette::text().bold(),
        ),
        Span::raw(" but not "),
        Span::styled(terminal_text::escape(&model.main), Palette::text().bold()),
        Span::styled(format!("  ·  {status}"), Palette::muted()),
    ]);
    let summary = if center_summary {
        summary.alignment(HorizontalAlignment::Right)
    } else {
        summary
    };
    if center_summary {
        frame.render_widget(
            Paragraph::new(Line::styled("GRADUATE", Palette::primary().bold())),
            area,
        );
        frame.render_widget(Paragraph::new(summary), area);
    } else {
        frame.render_widget(
            Paragraph::new(vec![
                Line::styled("Promotion report", Palette::primary().bold()),
                summary,
            ]),
            area,
        );
    }
}

fn render_table(frame: &mut Frame<'_>, area: Rect, model: &mut DiffModel, show_jira: bool) {
    let sort_label = |label: &str, key: SortKey| {
        if model.sort == key {
            format!("{label}{}", key.indicator())
        } else {
            label.to_owned()
        }
    };
    let mut labels = vec![
        (
            sort_label("BRANCH", SortKey::Branch),
            HorizontalAlignment::Left,
        ),
        (
            sort_label("STARTED", SortKey::Started),
            HorizontalAlignment::Left,
        ),
        (sort_label("LAST", SortKey::Last), HorizontalAlignment::Left),
        (
            sort_label("AHEAD", SortKey::Ahead),
            HorizontalAlignment::Right,
        ),
    ];
    if show_jira {
        labels.push(("JIRA".to_owned(), HorizontalAlignment::Left));
        labels.push(("STATUS".to_owned(), HorizontalAlignment::Left));
    }
    let header = Row::new(
        labels
            .into_iter()
            .map(|(label, alignment)| Cell::from(Line::from(label).alignment(alignment))),
    )
    .style(Palette::muted().add_modifier(Modifier::BOLD))
    .bottom_margin(1);
    let rows = model.rows.iter().map(|row| {
        let flagged = row
            .report
            .as_ref()
            .is_some_and(|report| !report.merged_environments.is_empty());
        let branch = terminal_text::escape(&row.branch);
        let branch = if flagged {
            format!("{branch} ⚠")
        } else {
            branch
        };
        let mut cells = match &row.report {
            Some(report) => vec![
                Cell::from(branch),
                Cell::from(report.started.clone()),
                Cell::from(report.last.clone()),
                Cell::from(
                    Line::from(report.ahead.to_string()).alignment(HorizontalAlignment::Right),
                ),
            ],
            None => vec![
                Cell::from(branch),
                placeholder_cell(),
                placeholder_cell(),
                Cell::from(
                    Line::styled("…", Palette::muted()).alignment(HorizontalAlignment::Right),
                ),
            ],
        };
        if show_jira {
            match &row.report {
                Some(report) => {
                    let (key, status) = jira_cells(&report.jira, flagged);
                    cells.push(key);
                    cells.push(status);
                }
                None => {
                    cells.push(placeholder_cell());
                    cells.push(Cell::from(Line::styled("measuring", Palette::muted())));
                }
            }
        }
        let cells = Row::new(cells);
        if flagged {
            cells.style(Palette::error())
        } else {
            cells
        }
    });
    let longest_branch = model
        .rows
        .iter()
        .map(|row| terminal_text::escape(&row.branch).chars().count())
        .max()
        .unwrap_or(0)
        .saturating_add(2);
    let branch_width = u16::try_from(longest_branch).unwrap_or(u16::MAX);
    let widths = if show_jira {
        vec![
            Constraint::Length(branch_width.max(24)),
            Constraint::Length(10),
            Constraint::Length(10),
            Constraint::Length(7),
            Constraint::Length(12),
            Constraint::Length(16),
        ]
    } else {
        vec![
            Constraint::Length(branch_width.max(20)),
            Constraint::Length(10),
            Constraint::Length(10),
            Constraint::Length(7),
        ]
    };
    let table = Table::new(rows, widths)
        .column_spacing(2)
        .header(header)
        .row_highlight_style(Palette::action_focus())
        .highlight_symbol("› ");
    frame.render_stateful_widget(table, area, &mut model.table_state);
}

fn placeholder_cell() -> Cell<'static> {
    Cell::from(Line::styled("…", Palette::muted()))
}

fn render_inspector(frame: &mut Frame<'_>, area: Rect, model: &DiffModel) {
    let selected_row = model.rows.get(model.selected);
    let selected = selected_row.and_then(|row| row.report.as_ref());
    let branch = selected_row.map_or("No branch selected", |row| row.branch.as_str());
    let position = if model.rows.is_empty() {
        "0 of 0".to_owned()
    } else {
        format!("{} of {}", model.selected + 1, model.rows.len())
    };
    let compact = area.height < 15;
    let padding = if compact {
        Padding::new(SPACE_1X, SPACE_1X, 0, 0)
    } else {
        Padding::uniform(SPACE_1X)
    };
    let card = Block::default()
        .borders(Borders::ALL)
        .border_style(Palette::muted())
        .title(Line::styled(
            format!(" {} ", terminal_text::escape(branch)),
            Palette::primary().bold(),
        ))
        .title(
            Line::styled(format!(" {position} "), Palette::muted())
                .alignment(HorizontalAlignment::Right),
        )
        .padding(padding);
    let content = card.inner(area);
    let mut lines = Vec::new();
    lines.extend(inspector_status(selected));
    if let Some(report) = selected {
        if !compact {
            lines.push(Line::default());
            lines.push(Line::styled("────────────────", Palette::muted()));
            lines.push(Line::default());
        }
        lines.extend(report_metadata(report));
    }
    frame.render_widget(card, area);
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: true }), content);
}

fn inspector_status(selected: Option<&PromotionBranch>) -> Vec<Line<'static>> {
    let Some(report) = selected else {
        return vec![detail_line("Status", "Measuring branch history…")];
    };
    match &report.jira {
        JiraIssueState::Loaded(issue) => vec![
            detail_line("Jira", &issue.key),
            detail_line("Summary", &issue.summary),
            detail_line("Status", &issue.status),
            detail_line(
                "Assignee",
                issue.assignee.as_deref().unwrap_or("Unassigned"),
            ),
            detail_line(
                "Versions",
                &if issue.fix_versions.is_empty() {
                    "None".to_owned()
                } else {
                    issue.fix_versions.join(", ")
                },
            ),
        ],
        JiraIssueState::Failed { message, .. } => vec![
            Line::from(vec![
                detail_label("Jira"),
                Span::styled("Error", Palette::error()),
            ]),
            detail_line("Reason", message),
        ],
        JiraIssueState::NotConfigured { key } => vec![
            detail_line("Jira", key),
            Line::from(vec![
                detail_label("State"),
                Span::styled("Not configured", Palette::warning()),
            ]),
            detail_line("Next", "gd auth setup jira"),
        ],
        JiraIssueState::Loading { key } => vec![detail_line("Jira", &format!("{key} · Loading…"))],
        JiraIssueState::NotFound { key } => vec![
            detail_line("Jira", key),
            Line::from(vec![
                detail_label("State"),
                Span::styled("Not found", Palette::muted()),
            ]),
            detail_line("Status", "This Jira ticket was not found"),
        ],
        JiraIssueState::NoTicket => vec![detail_line("Jira", "No ticket key in branch name")],
    }
}

fn jira_cells(state: &JiraIssueState, flagged: bool) -> (Cell<'static>, Cell<'static>) {
    // Only a Jira-validated ticket key may appear in the ticket column.
    let key = match state {
        JiraIssueState::Loaded(issue) => terminal_text::escape(&issue.key),
        _ => String::new(),
    };
    let (status, style) = match state {
        JiraIssueState::NoTicket | JiraIssueState::NotFound { .. } => {
            ("not found".to_owned(), Palette::muted())
        }
        JiraIssueState::NotConfigured { .. } => ("not configured".to_owned(), Palette::warning()),
        JiraIssueState::Loading { .. } => ("loading…".to_owned(), Palette::muted()),
        JiraIssueState::Loaded(issue) => {
            let status = terminal_text::escape(&issue.status);
            let style = jira_status_style(&status);
            (status, style)
        }
        JiraIssueState::Failed { .. } => ("Jira error".to_owned(), Palette::error()),
    };
    if flagged {
        // The row's warning color must own every cell of a flagged branch.
        (Cell::from(key), Cell::from(status))
    } else {
        (
            Cell::from(Line::styled(key, Palette::muted())),
            Cell::from(Line::styled(status, style)),
        )
    }
}

fn jira_status_style(status: &str) -> Style {
    let lowered = status.to_ascii_lowercase();
    if matches!(lowered.as_str(), "done" | "closed" | "resolved") {
        Palette::success()
    } else if lowered.contains("cancel") {
        Palette::muted()
    } else {
        Palette::text()
    }
}

fn render_details(frame: &mut Frame<'_>, area: Rect, model: &DiffModel) {
    let selected_row = model.rows.get(model.selected);
    let selected = selected_row.and_then(|row| row.report.as_ref());
    let branch = selected_row.map_or("No branch selected", |row| row.branch.as_str());
    let position = if model.rows.is_empty() {
        "0 of 0".to_owned()
    } else {
        format!("{} of {}", model.selected + 1, model.rows.len())
    };
    let (status_lines, metadata_lines) = match selected {
        Some(report) => match &report.jira {
            JiraIssueState::Loaded(issue) => (
                vec![
                    detail_line("Jira", &issue.key),
                    detail_line("Summary", &issue.summary),
                    detail_line("Status", &issue.status),
                    detail_line(
                        "Versions",
                        &if issue.fix_versions.is_empty() {
                            "None".to_owned()
                        } else {
                            issue.fix_versions.join(", ")
                        },
                    ),
                ],
                vec![
                    detail_line(
                        "Assignee",
                        issue.assignee.as_deref().unwrap_or("Unassigned"),
                    ),
                    detail_line("Author", &report.last_author),
                    detail_line("Commits", &report.ahead.to_string()),
                    detail_line("Updated", &report.last),
                ],
            ),
            JiraIssueState::Failed { message, .. } => (
                vec![
                    Line::from(vec![
                        detail_label("Jira"),
                        Span::styled("Error", Palette::error()),
                    ]),
                    detail_line("Reason", message),
                ],
                report_metadata(report),
            ),
            JiraIssueState::NotConfigured { key } => (
                vec![
                    Line::from(vec![
                        detail_label("Jira"),
                        Span::styled(
                            format!("{} · Not configured", terminal_text::escape(key)),
                            Palette::warning(),
                        ),
                    ]),
                    detail_line("Next", "gd auth setup jira"),
                ],
                report_metadata(report),
            ),
            JiraIssueState::Loading { key } => (
                vec![detail_line("Jira", &format!("{key} · Loading…"))],
                report_metadata(report),
            ),
            JiraIssueState::NotFound { key } => (
                vec![
                    Line::from(vec![
                        detail_label("Jira"),
                        Span::styled(
                            format!("{} · Not found", terminal_text::escape(key)),
                            Palette::muted(),
                        ),
                    ]),
                    detail_line("Status", "This Jira ticket was not found"),
                ],
                report_metadata(report),
            ),
            JiraIssueState::NoTicket => (
                vec![detail_line("Jira", "No ticket key in branch name")],
                report_metadata(report),
            ),
        },
        None => (
            vec![detail_line("Status", "Measuring branch history…")],
            Vec::new(),
        ),
    };
    let branch_title =
        Line::from(format!(" {} ", terminal_text::escape(branch))).style(Palette::primary().bold());
    let position_title = Line::from(format!(" {position} ")).style(Palette::muted());
    let card = Block::default()
        .borders(Borders::ALL)
        .border_style(Palette::muted())
        .title(branch_title)
        .title(position_title.alignment(HorizontalAlignment::Right))
        .padding(Padding::uniform(SPACE_1X));
    let content = card.inner(area);
    let [status, metadata] =
        Layout::horizontal([Constraint::Percentage(55), Constraint::Fill(1)]).areas(content);
    frame.render_widget(card, area);
    frame.render_widget(
        Paragraph::new(status_lines).wrap(Wrap { trim: true }),
        status,
    );
    frame.render_widget(
        Paragraph::new(metadata_lines)
            .block(
                Block::default()
                    .borders(Borders::LEFT)
                    .border_style(Palette::muted())
                    .padding(Padding::new(SPACE_2X, 0, 0, 0)),
            )
            .wrap(Wrap { trim: true }),
        metadata,
    );
}

fn detail_label(label: &str) -> Span<'static> {
    Span::styled(format!("{label}  "), Palette::muted())
}

fn detail_line(label: &str, value: &str) -> Line<'static> {
    Line::from(vec![
        detail_label(label),
        Span::raw(terminal_text::escape(value)),
    ])
}

fn report_metadata(report: &PromotionBranch) -> Vec<Line<'static>> {
    vec![
        detail_line("Author", &report.last_author),
        detail_line("Commits", &report.ahead.to_string()),
        detail_line("Updated", &report.last),
    ]
}

fn environment_merge_warning(model: &DiffModel) -> Option<String> {
    let report = model.rows.get(model.selected)?.report.as_ref()?;
    if report.merged_environments.is_empty() {
        return None;
    }
    let environments = report.merged_environments.join(", ");
    let verb = if report.merged_environments.len() == 1 {
        "has"
    } else {
        "have"
    };
    Some(format!(
        "⚠ {environments} {verb} been merged into this branch; its ahead count and dates include environment commits"
    ))
}

fn systemic_not_found_hint(model: &DiffModel) -> Option<String> {
    if !model.finished {
        return None;
    }
    let summary = systemic_not_found(
        model
            .rows
            .iter()
            .filter_map(|row| row.report.as_ref().map(|report| &report.jira)),
    )?;
    Some(format!(
        "⚠ {} of {} ticket lookups returned not found; check your Jira site and project access",
        summary.not_found, summary.resolved
    ))
}

fn render_footer(frame: &mut Frame<'_>, area: Rect, model: &DiffModel) {
    let help = if let Some(warning) = &model.warning {
        Line::styled(terminal_text::escape(warning), Palette::warning())
    } else if let Some(warning) = environment_merge_warning(model) {
        Line::styled(terminal_text::escape(&warning), Palette::error())
    } else if let Some(hint) = systemic_not_found_hint(model) {
        Line::styled(terminal_text::escape(&hint), Palette::warning())
    } else {
        Line::from(vec![
            Span::styled("↑/↓", Palette::primary()),
            Span::raw(" move   "),
            Span::styled("o", Palette::primary()),
            Span::raw(" open Jira   "),
            Span::styled("h", Palette::primary()),
            Span::raw(" git history   "),
            Span::styled("a", Palette::primary()),
            Span::raw(" age report   "),
            Span::styled("s", Palette::primary()),
            Span::raw(" sort   "),
            Span::styled("q", Palette::primary()),
            Span::raw(" close"),
        ])
    };
    frame.render_widget(Paragraph::new(help), area);
}

#[cfg(test)]
mod tests {
    use graduate::promotion::PromotionCommit;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    use super::*;

    fn test_commit(subject: &str) -> PromotionCommit {
        PromotionCommit {
            id: format!("a1b2c3d-{subject}"),
            short_id: "a1b2c3d".to_owned(),
            subject: subject.to_owned(),
            author: "Pat".to_owned(),
            date: "2024-01-02".to_owned(),
        }
    }

    fn test_model() -> Result<DiffModel, graduate::promotion::ReportDateError> {
        Ok(DiffModel::new(ReportDate::parse("2026-08-04")?))
    }

    #[test]
    fn every_modal_uses_the_wide_content_viewport() {
        assert_eq!(modal_area(Rect::new(0, 0, 160, 48), 34).width, 115);
        assert_eq!(modal_area(Rect::new(0, 0, 90, 48), 24).width, 90);
    }

    #[test]
    fn renders_skeleton_rows_before_measurements_finish() -> Result<(), Box<dyn std::error::Error>>
    {
        let mut model = test_model()?;
        update(
            &mut model,
            Message::Scan(Box::new(DiffUpdate::Skeleton {
                environment: "qa".to_owned(),
                main: "main".to_owned(),
                branches: vec!["feature/PROJ-123-login".to_owned()],
            })),
        )?;
        let mut terminal = Terminal::new(TestBackend::new(110, 48))?;
        terminal.draw(|frame| render(frame, &mut model))?;
        let rendered = terminal.backend().to_string();

        assert!(rendered.contains("In qa but not main"));
        assert!(rendered.contains("feature/PROJ-123-login"));
        assert!(rendered.contains("measuring"));
        Ok(())
    }

    #[test]
    fn short_wide_report_places_selected_branch_inspector_beside_the_table(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let mut model = test_model()?;
        let mut terminal = Terminal::new(TestBackend::new(110, 32))?;

        terminal.draw(|frame| render(frame, &mut model))?;
        let rendered = terminal.backend().to_string();
        let table_column = rendered
            .lines()
            .find_map(|line| line.find("BRANCH"))
            .ok_or("branch heading was not rendered")?;
        let inspector_column = rendered
            .lines()
            .find_map(|line| line.find("No branch selected"))
            .ok_or("selected branch inspector was not rendered")?;
        let summary_column = rendered
            .lines()
            .find_map(|line| line.find("In  but not"))
            .ok_or("report summary was not rendered")?;

        assert!(inspector_column > table_column + 40);
        assert!(summary_column > table_column + 40);
        assert!(rendered.contains("GRADUATE"));
        assert!(!rendered.contains("Promotion report"));
        assert!(!rendered.contains(theme::GRADUATE_ART[0]));
        Ok(())
    }

    #[test]
    fn tall_report_stacks_details_above_the_full_table() -> Result<(), Box<dyn std::error::Error>> {
        let mut model = test_model()?;
        let mut terminal = Terminal::new(TestBackend::new(110, 48))?;

        terminal.draw(|frame| render(frame, &mut model))?;
        let rendered = terminal.backend().to_string();
        let lines = rendered.lines().collect::<Vec<_>>();
        let details_row = lines
            .iter()
            .position(|line| line.contains("No branch selected"))
            .ok_or("detail card title was not rendered")?;
        let table_row = lines
            .iter()
            .position(|line| line.contains("BRANCH") && line.contains("JIRA"))
            .ok_or("full table heading was not rendered")?;

        assert!(details_row < table_row, "rows: {lines:#?}");
        assert!(!rendered.contains(" SELECTED "));
        assert!(rendered.contains(theme::GRADUATE_ART[0]));
        Ok(())
    }

    #[test]
    fn loaded_jira_details_are_visible() -> Result<(), Box<dyn std::error::Error>> {
        let mut model = test_model()?;
        update(
            &mut model,
            Message::Scan(Box::new(DiffUpdate::Skeleton {
                environment: "qa".to_owned(),
                main: "main".to_owned(),
                branches: vec!["feature/PROJ-123-login".to_owned()],
            })),
        )?;
        update(
            &mut model,
            Message::Scan(Box::new(DiffUpdate::Measured(PromotionBranch {
                branch: "feature/PROJ-123-login".to_owned(),
                started: "2024-01-01".to_owned(),
                last: "2024-01-02".to_owned(),
                ahead: 2,
                last_author: "Pat".to_owned(),
                commits: vec![test_commit("Add login"), test_commit("Add login tests")],
                merged_environments: Vec::new(),
                jira: JiraIssueState::Loaded(graduate::promotion::JiraIssueSummary {
                    key: "PROJ-123".to_owned(),
                    api_url: "https://example.atlassian.net/rest/api/3/issue/10001".to_owned(),
                    summary: "Add login".to_owned(),
                    status: "Ready for QA".to_owned(),
                    assignee: Some("Pat".to_owned()),
                    fix_versions: vec!["1.2".to_owned()],
                    url: "https://example.atlassian.net/browse/PROJ-123".to_owned(),
                }),
            }))),
        )?;
        let mut terminal = Terminal::new(TestBackend::new(110, 48))?;
        terminal.draw(|frame| render(frame, &mut model))?;
        let rendered = terminal.backend().to_string();

        assert!(rendered.contains("Ready for QA"));
        assert!(rendered.contains("Add login"));
        assert!(rendered.contains("Versions  1.2"));
        assert!(rendered.contains("feature/PROJ-123-login"));
        assert!(rendered.contains("1 of 1"));
        assert!(rendered.contains("Author  Pat"));
        assert!(rendered.contains("Commits  2"));
        assert!(rendered.contains("Updated  2024-01-02"));
        Ok(())
    }

    #[test]
    fn inspector_separates_jira_status_from_branch_metadata(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let mut model = test_model()?;
        update(
            &mut model,
            Message::Scan(Box::new(DiffUpdate::Skeleton {
                environment: "qa".to_owned(),
                main: "main".to_owned(),
                branches: vec!["feature/PROJ-123-login".to_owned()],
            })),
        )?;
        update(
            &mut model,
            Message::Scan(Box::new(DiffUpdate::Measured(PromotionBranch {
                branch: "feature/PROJ-123-login".to_owned(),
                started: "2024-01-01".to_owned(),
                last: "2024-01-02".to_owned(),
                ahead: 2,
                last_author: "Pat".to_owned(),
                commits: Vec::new(),
                merged_environments: Vec::new(),
                jira: JiraIssueState::NotConfigured {
                    key: "PROJ-123".to_owned(),
                },
            }))),
        )?;
        let mut terminal = Terminal::new(TestBackend::new(110, 32))?;

        terminal.draw(|frame| render(frame, &mut model))?;
        let rendered = terminal.backend().to_string();
        let lines = rendered.lines().collect::<Vec<_>>();
        let jira_row = lines
            .iter()
            .position(|line| line.contains("Jira  PROJ-123"))
            .ok_or("Jira status was not rendered")?;
        let author_row = lines
            .iter()
            .position(|line| line.contains("Author  Pat"))
            .ok_or("branch metadata was not rendered")?;

        assert!(author_row > jira_row);
        assert!(lines[jira_row..author_row]
            .iter()
            .any(|line| line.contains("────")));
        assert!(rendered.contains("Next  gd auth setup jira"));
        Ok(())
    }

    #[test]
    fn very_short_inspector_keeps_branch_metadata_visible() -> Result<(), Box<dyn std::error::Error>>
    {
        let mut model = test_model()?;
        update(
            &mut model,
            Message::Scan(Box::new(DiffUpdate::Skeleton {
                environment: "qa".to_owned(),
                main: "main".to_owned(),
                branches: vec!["feature/PROJ-123-login".to_owned()],
            })),
        )?;
        update(
            &mut model,
            Message::Scan(Box::new(DiffUpdate::Measured(PromotionBranch {
                branch: "feature/PROJ-123-login".to_owned(),
                started: "2024-01-01".to_owned(),
                last: "2024-01-02".to_owned(),
                ahead: 2,
                last_author: "Pat".to_owned(),
                commits: Vec::new(),
                merged_environments: Vec::new(),
                jira: JiraIssueState::NotConfigured {
                    key: "PROJ-123".to_owned(),
                },
            }))),
        )?;
        let mut terminal = Terminal::new(TestBackend::new(110, 18))?;

        terminal.draw(|frame| render(frame, &mut model))?;
        let rendered = terminal.backend().to_string();

        assert!(rendered.contains("Author  Pat"));
        assert!(rendered.contains("Commits  2"));
        assert!(rendered.contains("Updated  2024-01-02"));
        Ok(())
    }

    #[test]
    fn history_list_scrolls_to_any_number_of_commits() -> Result<(), Box<dyn std::error::Error>> {
        let mut model = test_model()?;
        update(
            &mut model,
            Message::Scan(Box::new(DiffUpdate::Skeleton {
                environment: "qa".to_owned(),
                main: "main".to_owned(),
                branches: vec!["feature/PROJ-123-login".to_owned()],
            })),
        )?;
        update(
            &mut model,
            Message::Scan(Box::new(DiffUpdate::Measured(PromotionBranch {
                branch: "feature/PROJ-123-login".to_owned(),
                started: "2024-01-01".to_owned(),
                last: "2024-01-02".to_owned(),
                ahead: 50,
                last_author: "Pat".to_owned(),
                commits: (1..=50)
                    .map(|index| test_commit(&format!("Commit {index}")))
                    .collect(),
                merged_environments: Vec::new(),
                jira: JiraIssueState::NoTicket,
            }))),
        )?;
        update(&mut model, Message::OpenHistory)?;
        for _ in 1..50 {
            update(&mut model, Message::ScrollHistoryDown)?;
        }
        let mut terminal = Terminal::new(TestBackend::new(110, 48))?;

        terminal.draw(|frame| render(frame, &mut model))?;
        let rendered = terminal.backend().to_string();

        assert_eq!(model.history_selected, 49);
        assert!(rendered.contains("Commit 50"));
        Ok(())
    }

    #[test]
    fn history_sheet_uses_the_wide_modal_and_explains_the_comparison(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let mut model = test_model()?;
        model.main = "main".to_owned();
        model.rows.push(BranchRow {
            branch: "feature/PROJ-123-login".to_owned(),
            report: Some(PromotionBranch {
                branch: "feature/PROJ-123-login".to_owned(),
                started: "2024-01-01".to_owned(),
                last: "2024-01-02".to_owned(),
                ahead: 1,
                last_author: "Pat".to_owned(),
                commits: vec![test_commit("DEMO-101 Add authentication")],
                merged_environments: Vec::new(),
                jira: JiraIssueState::NoTicket,
            }),
        });
        update(&mut model, Message::OpenHistory)?;
        let mut terminal = Terminal::new(TestBackend::new(110, 48))?;

        terminal.draw(|frame| render(frame, &mut model))?;
        let rendered = terminal.backend().to_string();
        let lines = rendered.lines().collect::<Vec<_>>();
        let top = lines
            .iter()
            .position(|line| line.contains("Commits ahead of main"))
            .ok_or("history title was not rendered")?;
        let bottom = lines
            .iter()
            .position(|line| line.contains("Esc/h") && line.contains("close history"))
            .ok_or("history footer was not rendered")?;
        let headings = lines
            .iter()
            .find(|line| line.contains("SHA") && line.contains("SUBJECT"))
            .ok_or("history headings were not rendered")?;
        let commit = lines
            .iter()
            .find(|line| line.contains("a1b2c3d"))
            .ok_or("history commit was not rendered")?;
        let character_position = |line: &str, needle: &str| {
            line.find(needle)
                .map(|byte_index| line[..byte_index].chars().count())
        };

        assert!(rendered.contains("feature/PROJ-123-login  ·  1 commit  ·  newest first"));
        assert!(!lines[top + 1].contains("feature/PROJ-123-login"));
        assert!(lines[top + 2].contains("feature/PROJ-123-login"));
        assert!(!lines[top + 3].contains("feature/PROJ-123-login"));
        assert!(rendered.contains("SHA"));
        assert!(rendered.contains("SUBJECT"));
        assert!(rendered.contains("AUTHOR"));
        assert!(rendered.contains("DATE"));
        assert!(rendered.contains("a1b2c3d"));
        assert!(rendered.contains("DEMO-101 Add authentication"));
        assert!(rendered.contains("Pat"));
        assert!(rendered.contains("2024-01-02"));
        assert!(rendered.contains("1 of 1"));
        assert_eq!(
            character_position(headings, "SHA"),
            character_position(commit, "a1b2c3d")
        );
        assert_eq!(
            character_position(headings, "SUBJECT"),
            character_position(commit, "DEMO-101 Add authentication")
        );
        assert!(!lines[top].contains("2024-01-02"));
        assert!(bottom.saturating_sub(top) >= 8);
        assert!(bottom.saturating_sub(top) < 12);
        Ok(())
    }

    #[test]
    fn a_opens_and_closes_the_age_report_modal() -> Result<(), Box<dyn std::error::Error>> {
        let mut model = DiffModel::new(ReportDate::parse("2026-08-04")?);
        update(
            &mut model,
            Message::Scan(Box::new(DiffUpdate::Skeleton {
                environment: "qa".to_owned(),
                main: "main".to_owned(),
                branches: vec!["feature/legacy".to_owned()],
            })),
        )?;
        update(
            &mut model,
            Message::Scan(Box::new(DiffUpdate::Measured(PromotionBranch {
                branch: "feature/legacy".to_owned(),
                started: "2019-12-31".to_owned(),
                last: "2019-12-31".to_owned(),
                ahead: 1,
                last_author: "Pat".to_owned(),
                commits: vec![PromotionCommit {
                    id: "222222222222".to_owned(),
                    short_id: "2222222".to_owned(),
                    subject: "Legacy".to_owned(),
                    author: "Pat".to_owned(),
                    date: "2019-12-31".to_owned(),
                }],
                merged_environments: Vec::new(),
                jira: JiraIssueState::NoTicket,
            }))),
        )?;
        update(&mut model, Message::Scan(Box::new(DiffUpdate::Finished)))?;

        let message = message_for_key(&model, KeyCode::Char('a'), KeyModifiers::NONE)
            .ok_or("a did not map to the age report")?;
        update(&mut model, message)?;
        let mut terminal = Terminal::new(TestBackend::new(110, 48))?;
        terminal.draw(|frame| render(frame, &mut model))?;
        let rendered = terminal.backend().to_string();

        assert!(rendered.contains("The age of unshipped work"));
        assert!(rendered.contains("2019"));
        assert!(!rendered.contains("Before 2020"));
        assert!(rendered.contains("Will not ship without a decision"));
        assert!(rendered.contains("feature/legacy"));

        let mut short_terminal = Terminal::new(TestBackend::new(110, 18))?;
        short_terminal.draw(|frame| render(frame, &mut model))?;
        let short_rendered = short_terminal.backend().to_string();
        assert!(short_rendered.contains("The age of unshipped work"));
        assert!(short_rendered.contains("Will not ship without a decision"));

        let close = message_for_key(&model, KeyCode::Char('a'), KeyModifiers::NONE)
            .ok_or("a did not close the age report")?;
        update(&mut model, close)?;
        assert!(model.age_report.is_none());
        Ok(())
    }

    #[test]
    fn age_report_waits_for_a_complete_scan() -> Result<(), Box<dyn std::error::Error>> {
        let mut model = test_model()?;

        update(&mut model, Message::OpenAgeReport)?;

        assert!(model.age_report.is_none());
        assert_eq!(
            model.warning.as_deref(),
            Some("The age report is available when the scan completes.")
        );
        Ok(())
    }

    #[test]
    fn age_report_scrolls_through_every_authored_year() -> Result<(), Box<dyn std::error::Error>> {
        let mut model = test_model()?;
        model.finished = true;
        model.environment = "qa".to_owned();
        model.main = "main".to_owned();
        model.rows.push(BranchRow {
            branch: "feature/history".to_owned(),
            report: Some(PromotionBranch {
                branch: "feature/history".to_owned(),
                started: "2000-01-01".to_owned(),
                last: "2026-01-01".to_owned(),
                ahead: 27,
                last_author: "Pat".to_owned(),
                commits: (2000..=2026)
                    .rev()
                    .map(|year| PromotionCommit {
                        id: format!("commit-{year}"),
                        short_id: year.to_string(),
                        subject: format!("Work from {year}"),
                        author: "Pat".to_owned(),
                        date: format!("{year}-01-01"),
                    })
                    .collect(),
                merged_environments: Vec::new(),
                jira: JiraIssueState::NoTicket,
            }),
        });
        update(&mut model, Message::OpenAgeReport)?;

        let scroll = message_for_key(&model, KeyCode::Down, KeyModifiers::NONE)
            .ok_or("down did not map to age-report scrolling")?;
        update(&mut model, scroll)?;
        for _ in 1..28 {
            update(&mut model, Message::ScrollAgeDown)?;
        }
        let mut terminal = Terminal::new(TestBackend::new(110, 32))?;
        terminal.draw(|frame| render(frame, &mut model))?;
        let rendered = terminal.backend().to_string();

        assert_eq!(model.age_selected, 28);
        assert!(rendered.contains("Older than one year"));
        assert!(rendered.contains("2000"));
        Ok(())
    }

    #[test]
    fn environment_merged_branch_renders_red_with_a_footer_warning(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let mut model = test_model()?;
        update(
            &mut model,
            Message::Scan(Box::new(DiffUpdate::Skeleton {
                environment: "qa".to_owned(),
                main: "main".to_owned(),
                branches: vec![
                    "feature/PROJ-123-login".to_owned(),
                    "feature/PROJ-124-clean".to_owned(),
                ],
            })),
        )?;
        update(
            &mut model,
            Message::Scan(Box::new(DiffUpdate::Measured(PromotionBranch {
                branch: "feature/PROJ-123-login".to_owned(),
                started: "2021-12-09".to_owned(),
                last: "2024-01-02".to_owned(),
                ahead: 3675,
                last_author: "Pat".to_owned(),
                commits: Vec::new(),
                merged_environments: vec!["qa".to_owned()],
                jira: JiraIssueState::NoTicket,
            }))),
        )?;
        update(
            &mut model,
            Message::Scan(Box::new(DiffUpdate::Measured(PromotionBranch {
                branch: "feature/PROJ-124-clean".to_owned(),
                started: "2024-01-01".to_owned(),
                last: "2024-01-02".to_owned(),
                ahead: 2,
                last_author: "Pat".to_owned(),
                commits: Vec::new(),
                merged_environments: Vec::new(),
                jira: JiraIssueState::NoTicket,
            }))),
        )?;
        let mut terminal = Terminal::new(TestBackend::new(110, 48))?;

        terminal.draw(|frame| render(frame, &mut model))?;
        let rendered = terminal.backend().to_string();
        let red_cells = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .any(|cell| cell.style().fg == Some(ratatui::style::Color::Red));

        assert!(rendered.contains("⚠ qa has been merged into this branch"));
        assert!(rendered.contains("feature/PROJ-123-login ⚠"));
        assert!(red_cells);

        update(&mut model, Message::MoveDown)?;
        terminal.draw(|frame| render(frame, &mut model))?;
        let rendered = terminal.backend().to_string();

        assert!(!rendered.contains("has been merged into this branch"));
        assert!(rendered.contains("open Jira"));
        Ok(())
    }

    #[test]
    fn h_closes_the_open_history_sheet() -> Result<(), Box<dyn std::error::Error>> {
        let mut model = test_model()?;
        model.history_open = true;

        let message = message_for_key(&model, KeyCode::Char('h'), KeyModifiers::NONE);
        if let Some(message) = message {
            update(&mut model, message)?;
        }

        assert!(!model.history_open);
        Ok(())
    }

    #[test]
    fn narrow_report_stacks_details_above_the_table() -> Result<(), Box<dyn std::error::Error>> {
        let mut model = test_model()?;
        let mut terminal = Terminal::new(TestBackend::new(90, 48))?;

        terminal.draw(|frame| render(frame, &mut model))?;
        let rendered = terminal.backend().to_string();
        let lines: Vec<_> = rendered.lines().collect();
        let details_row = lines
            .iter()
            .position(|line| line.contains("No branch selected"))
            .ok_or("detail card title was not rendered")?;
        let table_row = lines
            .iter()
            .position(|line| line.contains("BRANCH"))
            .ok_or("table heading was not rendered")?;

        assert!(details_row < table_row, "rows: {lines:#?}");
        Ok(())
    }

    #[test]
    fn moving_to_another_branch_clears_the_open_ticket_warning(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let mut model = test_model()?;
        update(
            &mut model,
            Message::Scan(Box::new(DiffUpdate::Skeleton {
                environment: "qa".to_owned(),
                main: "main".to_owned(),
                branches: vec![
                    "feature/no-ticket".to_owned(),
                    "feature/PROJ-123".to_owned(),
                ],
            })),
        )?;
        update(&mut model, Message::OpenTicket)?;

        update(&mut model, Message::MoveDown)?;

        assert_eq!(model.selected, 1);
        assert!(model.warning.is_none());
        Ok(())
    }

    #[test]
    fn moving_up_after_scrolling_moves_the_selection_within_the_viewport(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let mut model = test_model()?;
        update(
            &mut model,
            Message::Scan(Box::new(DiffUpdate::Skeleton {
                environment: "qa".to_owned(),
                main: "main".to_owned(),
                branches: (0..20).map(|index| format!("branch-{index:02}")).collect(),
            })),
        )?;
        let mut terminal = Terminal::new(TestBackend::new(110, 48))?;

        update(&mut model, Message::SelectLast)?;
        for _ in 0..4 {
            update(&mut model, Message::MoveUp)?;
        }
        terminal.draw(|frame| render(frame, &mut model))?;
        let before = terminal.backend().to_string();
        update(&mut model, Message::MoveUp)?;
        terminal.draw(|frame| render(frame, &mut model))?;
        let after = terminal.backend().to_string();

        assert_eq!(first_visible_branch(&before), first_visible_branch(&after));
        Ok(())
    }

    fn first_visible_branch(rendered: &str) -> Option<&str> {
        rendered
            .split("BRANCH")
            .nth(1)?
            .split_whitespace()
            .find(|value| value.starts_with("branch-"))
    }

    #[test]
    fn update_channel_must_not_close_before_finished() -> Result<(), Box<dyn std::error::Error>> {
        let result = finish_after_update_channel_closes(test_model()?);

        assert!(
            matches!(result, Err(CliError::Git(message)) if message.contains("before the scan completed"))
        );
        Ok(())
    }

    fn measured(
        branch: &str,
        started: &str,
        last: &str,
        ahead: usize,
        jira: JiraIssueState,
    ) -> DiffUpdate {
        DiffUpdate::Measured(PromotionBranch {
            branch: branch.to_owned(),
            started: started.to_owned(),
            last: last.to_owned(),
            ahead,
            last_author: "Pat".to_owned(),
            commits: Vec::new(),
            merged_environments: Vec::new(),
            jira,
        })
    }

    fn character_position(line: &str, needle: &str) -> Option<usize> {
        line.find(needle)
            .map(|byte_index| line[..byte_index].chars().count())
    }

    #[test]
    fn table_columns_stay_compact_and_ahead_counts_right_align(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let mut model = test_model()?;
        update(
            &mut model,
            Message::Scan(Box::new(DiffUpdate::Skeleton {
                environment: "qa".to_owned(),
                main: "main".to_owned(),
                branches: vec!["aa".to_owned(), "bb".to_owned()],
            })),
        )?;
        update(
            &mut model,
            Message::Scan(Box::new(measured(
                "aa",
                "2011-01-01",
                "2011-01-02",
                3,
                JiraIssueState::NoTicket,
            ))),
        )?;
        update(
            &mut model,
            Message::Scan(Box::new(measured(
                "bb",
                "2011-01-01",
                "2011-01-02",
                28,
                JiraIssueState::NoTicket,
            ))),
        )?;
        let mut terminal = Terminal::new(TestBackend::new(110, 48))?;

        terminal.draw(|frame| render(frame, &mut model))?;
        let rendered = terminal.backend().to_string();
        let lines = rendered.lines().collect::<Vec<_>>();
        let header = lines
            .iter()
            .find(|line| line.contains("BRANCH") && line.contains("AHEAD"))
            .ok_or("table heading was not rendered")?;
        let branch_column =
            character_position(header, "BRANCH").ok_or("BRANCH heading was not rendered")?;
        let started_column =
            character_position(header, "STARTED").ok_or("STARTED heading was not rendered")?;
        let ahead_end =
            character_position(header, "AHEAD").ok_or("AHEAD heading was not rendered")? + 4;
        let row_a = lines
            .iter()
            .find(|line| line.contains("aa") && line.contains("2011-01-01"))
            .ok_or("row aa was not rendered")?;
        let row_b = lines
            .iter()
            .find(|line| line.contains("bb") && line.contains("2011-01-01"))
            .ok_or("row bb was not rendered")?;

        assert_eq!(started_column - branch_column, 26);
        assert!(rendered.contains("2011-01-01  2011-01-02"));
        assert!(row_a.contains("not found"));
        assert_eq!(row_a.chars().nth(ahead_end), Some('3'));
        assert_eq!(row_b.chars().nth(ahead_end), Some('8'));
        Ok(())
    }

    #[test]
    fn s_cycles_the_sort_and_selection_follows_the_branch() -> Result<(), Box<dyn std::error::Error>>
    {
        let mut model = test_model()?;
        update(
            &mut model,
            Message::Scan(Box::new(DiffUpdate::Skeleton {
                environment: "qa".to_owned(),
                main: "main".to_owned(),
                branches: vec!["aa".to_owned(), "bb".to_owned()],
            })),
        )?;
        update(
            &mut model,
            Message::Scan(Box::new(measured(
                "aa",
                "2024-05-01",
                "2024-05-02",
                1,
                JiraIssueState::NoTicket,
            ))),
        )?;
        update(
            &mut model,
            Message::Scan(Box::new(measured(
                "bb",
                "2022-01-01",
                "2022-01-02",
                9,
                JiraIssueState::NoTicket,
            ))),
        )?;
        let mut terminal = Terminal::new(TestBackend::new(110, 48))?;

        let message = message_for_key(&model, KeyCode::Char('s'), KeyModifiers::NONE)
            .ok_or("s did not map to a sort message")?;
        update(&mut model, message)?;
        terminal.draw(|frame| render(frame, &mut model))?;
        let by_started = terminal.backend().to_string();

        assert_eq!(model.rows[0].branch, "bb");
        assert_eq!(model.selected, 1);
        assert!(by_started.contains("STARTED ▲"));

        update(&mut model, Message::CycleSort)?;
        update(&mut model, Message::CycleSort)?;
        terminal.draw(|frame| render(frame, &mut model))?;
        let by_ahead = terminal.backend().to_string();

        assert_eq!(model.rows[0].branch, "bb");
        assert!(by_ahead.contains("AHEAD ▼"));

        let report = model.completed_report();
        assert_eq!(report.branches[0].branch, "aa");
        Ok(())
    }

    #[test]
    fn unvalidated_jira_keys_leave_the_ticket_column_blank(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let mut model = test_model()?;
        update(
            &mut model,
            Message::Scan(Box::new(DiffUpdate::Skeleton {
                environment: "qa".to_owned(),
                main: "main".to_owned(),
                branches: vec!["CLAIMS-9".to_owned()],
            })),
        )?;
        update(
            &mut model,
            Message::Scan(Box::new(measured(
                "CLAIMS-9",
                "2024-01-01",
                "2024-01-02",
                2,
                JiraIssueState::NotFound {
                    key: "CLAIMS-9".to_owned(),
                },
            ))),
        )?;
        let mut terminal = Terminal::new(TestBackend::new(110, 48))?;

        terminal.draw(|frame| render(frame, &mut model))?;
        let rendered = terminal.backend().to_string();
        let row = rendered
            .lines()
            .find(|line| line.contains("2024-01-01"))
            .ok_or("table row was not rendered")?;

        assert_eq!(row.matches("CLAIMS-9").count(), 1);
        assert!(row.contains("not found"));
        Ok(())
    }

    #[test]
    fn mostly_not_found_lookups_show_a_configuration_hint() -> Result<(), Box<dyn std::error::Error>>
    {
        let mut model = test_model()?;
        update(
            &mut model,
            Message::Scan(Box::new(DiffUpdate::Skeleton {
                environment: "qa".to_owned(),
                main: "main".to_owned(),
                branches: vec!["AA-1".to_owned(), "BB-2".to_owned(), "CC-3".to_owned()],
            })),
        )?;
        for branch in ["AA-1", "BB-2", "CC-3"] {
            update(
                &mut model,
                Message::Scan(Box::new(measured(
                    branch,
                    "2024-01-01",
                    "2024-01-02",
                    1,
                    JiraIssueState::NotFound {
                        key: branch.to_owned(),
                    },
                ))),
            )?;
        }
        update(&mut model, Message::Scan(Box::new(DiffUpdate::Finished)))?;
        let mut terminal = Terminal::new(TestBackend::new(110, 48))?;

        terminal.draw(|frame| render(frame, &mut model))?;
        let rendered = terminal.backend().to_string();

        assert!(rendered.contains("3 of 3 ticket lookups returned not found"));
        Ok(())
    }

    #[test]
    fn done_jira_statuses_render_in_the_success_color() -> Result<(), Box<dyn std::error::Error>> {
        let mut model = test_model()?;
        update(
            &mut model,
            Message::Scan(Box::new(DiffUpdate::Skeleton {
                environment: "qa".to_owned(),
                main: "main".to_owned(),
                branches: vec![
                    "aaa-selected".to_owned(),
                    "feature/PROJ-123-login".to_owned(),
                ],
            })),
        )?;
        update(
            &mut model,
            Message::Scan(Box::new(measured(
                "feature/PROJ-123-login",
                "2024-01-01",
                "2024-01-02",
                2,
                JiraIssueState::Loaded(graduate::promotion::JiraIssueSummary {
                    key: "PROJ-123".to_owned(),
                    api_url: "https://example.atlassian.net/rest/api/3/issue/10001".to_owned(),
                    summary: "Add login".to_owned(),
                    status: "Done".to_owned(),
                    assignee: None,
                    fix_versions: Vec::new(),
                    url: "https://example.atlassian.net/browse/PROJ-123".to_owned(),
                }),
            ))),
        )?;
        let mut terminal = Terminal::new(TestBackend::new(110, 48))?;

        terminal.draw(|frame| render(frame, &mut model))?;
        let green_cells = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .any(|cell| cell.style().fg == Some(ratatui::style::Color::Green));

        assert!(green_cells);
        Ok(())
    }
}
