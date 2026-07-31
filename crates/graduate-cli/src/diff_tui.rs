//! Streaming terminal list for environment promotion reports.

use std::time::Duration;

use crossterm::event::{Event, EventStream, KeyCode, KeyEventKind, KeyModifiers};
use futures_util::StreamExt;
use graduate::promotion::{JiraIssueState, PromotionBranch};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Cell, Paragraph, Row, Table, TableState, Wrap};
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
        }
    }

    fn apply(&mut self, update: DiffUpdate) -> Result<(), CliError> {
        match update {
            DiffUpdate::Skeleton {
                environment,
                main,
                branches,
            } => {
                self.environment = environment;
                self.main = main;
                self.rows = branches
                    .into_iter()
                    .map(|branch| BranchRow {
                        branch,
                        report: None,
                    })
                    .collect();
                self.selected = 0;
                self.table_state = TableState::default()
                    .with_selected((!self.rows.is_empty()).then_some(self.selected));
            }
            DiffUpdate::Measured(report) => {
                if let Some(row) = self.rows.iter_mut().find(|row| row.branch == report.branch) {
                    row.report = Some(report);
                }
            }
            DiffUpdate::Jira { branch, state } => {
                if let Some(report) = self
                    .rows
                    .iter_mut()
                    .find(|row| row.branch == branch)
                    .and_then(|row| row.report.as_mut())
                {
                    report.jira = state;
                }
            }
            DiffUpdate::Finished => self.finished = true,
            DiffUpdate::Failed(message) => return Err(CliError::Git(message)),
        }
        Ok(())
    }

    fn move_up(&mut self) {
        self.select(self.selected.saturating_sub(1));
    }

    fn move_down(&mut self) {
        if self.selected + 1 < self.rows.len() {
            self.select(self.selected + 1);
        }
    }

    fn select(&mut self, selected: usize) {
        if selected != self.selected {
            self.selected = selected;
            self.table_state.select(Some(selected));
            self.warning = None;
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
            update = updates.recv(), if updates_open => match update {
                Some(update) => {
                    model.apply(update)?;
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
                    if key.modifiers.contains(KeyModifiers::CONTROL)
                        && key.code == KeyCode::Char('c')
                    {
                        break Err(CliError::ReportCancelled);
                    }
                    match key.code {
                        KeyCode::Char('q') | KeyCode::Esc => {
                            break Ok(model.completed_report());
                        }
                        KeyCode::Up | KeyCode::Char('k') => model.move_up(),
                        KeyCode::Down | KeyCode::Char('j') => model.move_down(),
                        KeyCode::Home => model.select(0),
                        KeyCode::End => {
                            model.select(model.rows.len().saturating_sub(1));
                        }
                        KeyCode::Char('o') => {
                            if let Some(url) = model.selected_issue_url() {
                                if let Err(error) = browser.open(url) {
                                    model.warning = Some(format!(
                                        "Could not open Jira: {}",
                                        terminal_text::escape(&error.to_string())
                                    ));
                                }
                            } else {
                                model.warning = Some(
                                    "The selected branch does not have a loaded Jira ticket."
                                        .to_owned(),
                                );
                            }
                        }
                        _ => {}
                    }
                }
            },
            _ = ticker.tick(), if !model.finished => {
                model.frame = model.frame.wrapping_add(1);
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
    let [_top_padding, header, title, table, details, footer] = Layout::vertical([
        Constraint::Length(2),
        Constraint::Length(3),
        Constraint::Length(3),
        Constraint::Fill(1),
        Constraint::Length(7),
        Constraint::Length(2),
    ])
    .areas(area);
    theme::render_brand_header(frame, header);
    render_title(frame, title, model);
    render_table(frame, table, model);
    render_details(frame, details, model);
    render_footer(frame, footer, model);
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
        model.apply(DiffUpdate::Skeleton {
            environment: "qa".to_owned(),
            main: "main".to_owned(),
            branches: vec!["feature/PROJ-123-login".to_owned()],
        })?;
        let mut terminal = Terminal::new(TestBackend::new(110, 28))?;
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
        model.apply(DiffUpdate::Skeleton {
            environment: "qa".to_owned(),
            main: "main".to_owned(),
            branches: vec!["feature/PROJ-123-login".to_owned()],
        })?;
        model.apply(DiffUpdate::Measured(PromotionBranch {
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
        }))?;
        let mut terminal = Terminal::new(TestBackend::new(110, 28))?;
        terminal.draw(|frame| render(frame, &mut model))?;
        let rendered = terminal.backend().to_string();

        assert!(rendered.contains("Ready for QA"));
        assert!(rendered.contains("Add login"));
        assert!(rendered.contains("Fix versions: 1.2"));
        Ok(())
    }

    #[test]
    fn moving_to_another_branch_clears_the_open_ticket_warning() -> Result<(), CliError> {
        let mut model = DiffModel::new();
        model.apply(DiffUpdate::Skeleton {
            environment: "qa".to_owned(),
            main: "main".to_owned(),
            branches: vec![
                "feature/no-ticket".to_owned(),
                "feature/PROJ-123".to_owned(),
            ],
        })?;
        model.warning = Some("The selected branch does not have a loaded Jira ticket.".to_owned());

        model.move_down();

        assert_eq!(model.selected, 1);
        assert!(model.warning.is_none());
        Ok(())
    }

    #[test]
    fn moving_up_after_scrolling_moves_the_selection_within_the_viewport(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let mut model = DiffModel::new();
        model.apply(DiffUpdate::Skeleton {
            environment: "qa".to_owned(),
            main: "main".to_owned(),
            branches: (0..20).map(|index| format!("branch-{index:02}")).collect(),
        })?;
        let mut terminal = Terminal::new(TestBackend::new(110, 28))?;

        model.select(15);
        terminal.draw(|frame| render(frame, &mut model))?;
        let before = terminal.backend().to_string();
        model.move_up();
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
