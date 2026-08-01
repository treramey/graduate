//! Streaming terminal list for environment promotion reports.

use std::time::Duration;

use crossterm::event::{Event, EventStream, KeyCode, KeyEventKind, KeyModifiers};
use futures_util::StreamExt;
use graduate::promotion::{JiraIssueState, PromotionBranch};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, Borders, Cell, Clear, List, ListItem, ListState, Paragraph, Row, Table, TableState, Wrap,
};
use ratatui::Frame;
use tokio::sync::mpsc;

use crate::browser::BrowserLauncher;
use crate::diff::{DiffUpdate, PromotionReport};
use crate::error::CliError;
use crate::terminal::StderrTerminal;
use crate::terminal_text;
use crate::theme::{self, Palette};

const TICK_RATE: Duration = Duration::from_millis(120);
const SPINNER: [&str; 4] = ["◐", "◓", "◑", "◒"];

struct BranchRow {
    branch: String,
    report: Option<PromotionBranch>,
}

struct DiffModel {
    environment: String,
    main: String,
    rows: Vec<BranchRow>,
    selected: usize,
    table_state: TableState,
    finished: bool,
    frame: usize,
    warning: Option<String>,
    history_open: bool,
    history_selected: usize,
    history_list_state: ListState,
}

enum Message {
    Scan(Box<DiffUpdate>),
    MoveUp,
    MoveDown,
    SelectFirst,
    SelectLast,
    OpenTicket,
    OpenTicketFailed(String),
    OpenHistory,
    CloseHistory,
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
    fn new() -> Self {
        Self {
            environment: String::new(),
            main: String::new(),
            rows: Vec::new(),
            selected: 0,
            table_state: TableState::default(),
            finished: false,
            frame: 0,
            warning: None,
            history_open: false,
            history_selected: 0,
            history_list_state: ListState::default(),
        }
    }

    fn select(&mut self, selected: usize) {
        if selected != self.selected {
            self.selected = selected;
            self.table_state.select(Some(selected));
            self.warning = None;
            self.history_open = false;
            self.history_selected = 0;
            self.history_list_state = ListState::default();
        }
    }

    fn selected_issue_url(&self) -> Option<&str> {
        let report = self.rows.get(self.selected)?.report.as_ref()?;
        match &report.jira {
            JiraIssueState::Loaded(issue) => Some(&issue.url),
            _ => None,
        }
    }

    fn completed_report(self) -> PromotionReport {
        PromotionReport {
            environment: self.environment,
            main: self.main,
            branches: self.rows.into_iter().filter_map(|row| row.report).collect(),
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
        Message::OpenHistory => {
            if model
                .rows
                .get(model.selected)
                .and_then(|row| row.report.as_ref())
                .is_some()
            {
                model.history_open = true;
                model.history_selected = 0;
                model.history_list_state = ListState::default().with_selected(Some(0));
            } else {
                model.warning = Some("Branch history is still being measured.".to_owned());
            }
        }
        Message::CloseHistory => {
            model.history_open = false;
            model.history_selected = 0;
            model.history_list_state = ListState::default();
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
                .map_or(0, |report| report.commit_messages.len().saturating_sub(1));
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
        KeyCode::Char('q') | KeyCode::Esc if model.history_open => Some(Message::CloseHistory),
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
        KeyCode::Char('h') => Some(Message::OpenHistory),
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
    let mut model = DiffModel::new();
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
    let [_top_padding, header, _header_padding, title, table, _details_padding, details, footer] =
        Layout::vertical([
            Constraint::Length(2),
            Constraint::Length(theme::GRADUATE_ART_HEIGHT),
            Constraint::Length(1),
            Constraint::Length(3),
            Constraint::Fill(1),
            Constraint::Length(1),
            Constraint::Length(7),
            Constraint::Length(3),
        ])
        .areas(area);
    theme::render_brand_header(frame, header);
    render_title(frame, title, model);
    render_table(frame, table, model);
    render_details(frame, details, model);
    render_footer(frame, footer, model);
    if model.history_open {
        render_history(frame, model);
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
    let width = outer.width.saturating_sub(8).clamp(20, 90);
    let height = outer.height.saturating_sub(6).clamp(8, 24);
    let area = Rect::new(
        outer.x + outer.width.saturating_sub(width) / 2,
        outer.y + outer.height.saturating_sub(height) / 2,
        width,
        height,
    );
    let items = report
        .commit_messages
        .iter()
        .enumerate()
        .map(|(index, message)| {
            ListItem::new(Line::from(vec![
                Span::styled(format!("{:>3}. ", index + 1), Palette::muted()),
                Span::raw(terminal_text::escape(message)),
            ]))
        })
        .collect::<Vec<_>>();
    let title = format!(" Git history · {} ", terminal_text::escape(&report.branch));
    let list = List::new(items)
        .block(Block::default().borders(Borders::ALL).title(title))
        .highlight_style(Palette::action_focus())
        .highlight_symbol("› ");
    frame.render_widget(Clear, area);
    frame.render_stateful_widget(list, area, &mut model.history_list_state);
}

fn render_title(frame: &mut Frame<'_>, area: Rect, model: &DiffModel) {
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
        format!("{} branches", model.rows.len())
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
    frame.render_widget(
        Paragraph::new(vec![
            Line::styled("Promotion report", Palette::primary().bold()),
            Line::from(vec![
                Span::raw("In "),
                Span::styled(
                    terminal_text::escape(&model.environment),
                    Palette::text().bold(),
                ),
                Span::raw(" but not "),
                Span::styled(terminal_text::escape(&model.main), Palette::text().bold()),
                Span::styled(format!("  ·  {status}"), Palette::muted()),
            ]),
        ]),
        area,
    );
}

fn render_table(frame: &mut Frame<'_>, area: Rect, model: &mut DiffModel) {
    let header = Row::new(["BRANCH", "STARTED", "LAST", "AHEAD", "JIRA", "STATUS"])
        .style(Palette::muted().add_modifier(Modifier::BOLD))
        .bottom_margin(1);
    let rows = model.rows.iter().map(|row| {
        let values = match &row.report {
            Some(report) => {
                let (key, status) = jira_columns(&report.jira);
                vec![
                    terminal_text::escape(&report.branch),
                    report.started.clone(),
                    report.last.clone(),
                    report.ahead.to_string(),
                    key,
                    status,
                ]
            }
            None => vec![
                terminal_text::escape(&row.branch),
                "…".to_owned(),
                "…".to_owned(),
                "…".to_owned(),
                "…".to_owned(),
                "measuring".to_owned(),
            ],
        };
        Row::new(values.into_iter().map(Cell::from))
    });
    let widths = [
        Constraint::Min(24),
        Constraint::Length(10),
        Constraint::Length(10),
        Constraint::Length(6),
        Constraint::Length(12),
        Constraint::Length(16),
    ];
    let table = Table::new(rows, widths)
        .header(header)
        .row_highlight_style(Palette::action_focus())
        .highlight_symbol("› ");
    frame.render_stateful_widget(table, area, &mut model.table_state);
}

fn jira_columns(state: &JiraIssueState) -> (String, String) {
    match state {
        JiraIssueState::NoTicket => ("—".to_owned(), "no ticket".to_owned()),
        JiraIssueState::NotConfigured { key } => (key.clone(), "not configured".to_owned()),
        JiraIssueState::Loading { key } => (key.clone(), "loading…".to_owned()),
        JiraIssueState::Loaded(issue) => (issue.key.clone(), issue.status.clone()),
        JiraIssueState::Failed { key, .. } => (key.clone(), "Jira error".to_owned()),
    }
}

fn render_details(frame: &mut Frame<'_>, area: Rect, model: &DiffModel) {
    let selected = model
        .rows
        .get(model.selected)
        .and_then(|row| row.report.as_ref());
    let lines = match selected {
        Some(report) => match &report.jira {
            JiraIssueState::Loaded(issue) => vec![
                Line::from(vec![
                    Span::styled(
                        format!("{}  ", terminal_text::escape(&issue.key)),
                        Palette::primary().bold(),
                    ),
                    Span::raw(terminal_text::escape(&issue.summary)),
                ]),
                Line::from(format!(
                    "Status: {}  ·  Assignee: {}",
                    terminal_text::escape(&issue.status),
                    terminal_text::escape(issue.assignee.as_deref().unwrap_or("Unassigned"))
                )),
                Line::from(format!(
                    "Fix versions: {}",
                    terminal_text::escape(&if issue.fix_versions.is_empty() {
                        "None".to_owned()
                    } else {
                        issue.fix_versions.join(", ")
                    })
                )),
                Line::styled(
                    format!(
                        "Last author: {}",
                        terminal_text::escape(&report.last_author)
                    ),
                    Palette::muted(),
                ),
            ],
            JiraIssueState::Failed { message, .. } => vec![
                Line::styled("Jira details unavailable", Palette::error()),
                Line::raw(terminal_text::escape(message)),
            ],
            JiraIssueState::NotConfigured { .. } => vec![
                Line::raw("Configure Jira with `gd auth setup jira` to load ticket details."),
                Line::styled(
                    format!(
                        "Last author: {}",
                        terminal_text::escape(&report.last_author)
                    ),
                    Palette::muted(),
                ),
            ],
            JiraIssueState::Loading { .. } => vec![Line::raw("Loading Jira details…")],
            JiraIssueState::NoTicket => vec![
                Line::raw("No Jira key was found in this branch name."),
                Line::styled(
                    format!(
                        "Last author: {}",
                        terminal_text::escape(&report.last_author)
                    ),
                    Palette::muted(),
                ),
            ],
        },
        None => vec![Line::raw("Measuring branch history…")],
    };
    frame.render_widget(
        Paragraph::new(lines)
            .block(Block::default().borders(Borders::TOP).title(" Details "))
            .wrap(Wrap { trim: true }),
        area,
    );
}

fn render_footer(frame: &mut Frame<'_>, area: Rect, model: &DiffModel) {
    let help = if let Some(warning) = &model.warning {
        Line::styled(terminal_text::escape(warning), Palette::warning())
    } else {
        Line::from(vec![
            Span::styled("↑/↓", Palette::primary()),
            Span::raw(" move   "),
            Span::styled("o", Palette::primary()),
            Span::raw(" open Jira   "),
            Span::styled("h", Palette::primary()),
            Span::raw(" git history   "),
            Span::styled("q", Palette::primary()),
            Span::raw(" close"),
        ])
    };
    frame.render_widget(Paragraph::new(help), area);
}

#[cfg(test)]
mod tests {
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    use super::*;

    #[test]
    fn renders_skeleton_rows_before_measurements_finish() -> Result<(), Box<dyn std::error::Error>>
    {
        let mut model = DiffModel::new();
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
    fn loaded_jira_details_are_visible() -> Result<(), Box<dyn std::error::Error>> {
        let mut model = DiffModel::new();
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
                commit_messages: vec!["Add login".to_owned(), "Add login tests".to_owned()],
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
        assert!(rendered.contains("Fix versions: 1.2"));
        Ok(())
    }

    #[test]
    fn history_list_scrolls_to_any_number_of_commits() -> Result<(), Box<dyn std::error::Error>> {
        let mut model = DiffModel::new();
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
                commit_messages: (1..=50).map(|index| format!("Commit {index}")).collect(),
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
    fn details_are_raised_with_space_below_the_table() -> Result<(), Box<dyn std::error::Error>> {
        let mut model = DiffModel::new();
        let mut terminal = Terminal::new(TestBackend::new(110, 48))?;

        terminal.draw(|frame| render(frame, &mut model))?;
        let rendered = terminal.backend().to_string();
        let lines: Vec<_> = rendered.lines().collect();

        assert!(lines[37].trim_matches(['"', ' ']).is_empty());
        assert!(lines[38].contains("Details"), "rows: {lines:#?}");
        Ok(())
    }

    #[test]
    fn moving_to_another_branch_clears_the_open_ticket_warning() -> Result<(), CliError> {
        let mut model = DiffModel::new();
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
        let mut model = DiffModel::new();
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
            .split_whitespace()
            .find(|value| value.starts_with("branch-"))
    }

    #[test]
    fn update_channel_must_not_close_before_finished() {
        let result = finish_after_update_channel_closes(DiffModel::new());

        assert!(
            matches!(result, Err(CliError::Git(message)) if message.contains("before the scan completed"))
        );
    }
}
