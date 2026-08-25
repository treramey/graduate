//! Ratatui presentation and Crossterm runtime for interactive Jira authentication.

use std::io::{self, IsTerminal};
use std::time::Duration;

use crossterm::event::{Event, EventStream, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use futures_util::StreamExt;
use graduate::jira::JiraField;
use graduate::jira_auth::{CompletedLogin, OnboardingError, OnboardingScreen, SecretInput};
use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Layout, Position, Rect};
use ratatui::style::{Style, Stylize};
use ratatui::text::{Line, Text};
use ratatui::widgets::{Block, BorderType, Paragraph, Widget, Wrap};
use ratatui::Frame;
use tui_input::backend::crossterm::EventHandler;
use tui_input::{Input, InputRequest};

use crate::browser::BrowserLauncher;
use crate::error::CliError;
use crate::jira_auth::{ConnectionOutcome, OnboardingWorkflow};
use crate::terminal::StderrTerminal;
use crate::terminal_text;
use crate::theme::{
    constrain_content_width, render_brand_header, Palette, GRADUATE_ART_HEIGHT, MUTED_COLOR,
};

const MIN_TERMINAL_WIDTH: u16 = 60;
const MIN_TERMINAL_HEIGHT: u16 = 24;
const REVIEW_LABEL_WIDTH: usize = 10;
const ANIMATION_TICK_RATE: Duration = Duration::from_millis(40);
const PLAYFUL_FRAME_RATE: Duration = Duration::from_millis(80);
const CURSOR_BLINK_HALF_PERIOD: Duration = Duration::from_millis(500);
const SPINNER_FRAMES: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
pub(crate) const REDUCED_MOTION_ENV: &str = "GRADUATE_REDUCED_MOTION";

#[derive(Clone, Copy, PartialEq, Eq)]
enum ConnectionStatus {
    NotConnected,
    Pending,
    Connected,
}

struct OnboardingModel {
    pending_animation_elapsed: Duration,
    cursor_blink_elapsed: Duration,
    reduced_motion: bool,
    stage: OnboardingScreen,
    focus: usize,
    hostname: Input,
    email: Input,
    display_name: String,
    jira_token: Input,
    can_retain_jira_token: bool,
    jira_instruction: String,
    jira_url: String,
    jira_page_can_open: bool,
    jira_page_loaded: bool,
    jira_status: ConnectionStatus,
    error: Option<String>,
    warning: Option<String>,
}

impl OnboardingModel {
    fn new(workflow: &OnboardingWorkflow<'_>, reduced_motion: bool) -> Self {
        Self {
            pending_animation_elapsed: Duration::ZERO,
            cursor_blink_elapsed: Duration::ZERO,
            reduced_motion,
            stage: workflow.screen(),
            focus: 0,
            hostname: workflow.hostname_default().unwrap_or_default().into(),
            email: workflow.email_default().unwrap_or_default().into(),
            display_name: String::new(),
            jira_token: Input::default(),
            can_retain_jira_token: workflow.can_retain_token(),
            jira_instruction: String::new(),
            jira_url: String::new(),
            jira_page_can_open: false,
            jira_page_loaded: false,
            jira_status: ConnectionStatus::NotConnected,
            error: None,
            warning: None,
        }
    }

    fn handle_event(&mut self, event: Event) -> Action {
        match event {
            Event::Key(key) => self.handle_key(key),
            Event::Paste(value) => {
                if self.insert_into_focused_input(&value) {
                    self.input_changed();
                }
                Action::None
            }
            _ => Action::None,
        }
    }

    fn handle_key(&mut self, key: KeyEvent) -> Action {
        if !matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) {
            return Action::None;
        }
        self.reset_cursor_blink();
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            return Action::Cancel;
        }
        match key.code {
            KeyCode::Esc if self.stage == OnboardingScreen::JiraDetails => Action::Cancel,
            KeyCode::Esc => Action::Back,
            KeyCode::Tab if key.modifiers.contains(KeyModifiers::SHIFT) => {
                self.focus_previous();
                Action::None
            }
            KeyCode::Tab => {
                self.focus_next();
                Action::None
            }
            KeyCode::BackTab => {
                self.focus_previous();
                Action::None
            }
            KeyCode::Up => {
                self.focus_previous();
                Action::None
            }
            KeyCode::Down => {
                self.focus_next();
                Action::None
            }
            KeyCode::Char('j' | 'J')
                if self.stage == OnboardingScreen::Save
                    && matches!(key.modifiers, KeyModifiers::NONE | KeyModifiers::SHIFT) =>
            {
                Action::EditJira
            }
            KeyCode::Enter => self.activate_or_advance(),
            _ => {
                let changed = self
                    .focused_input_mut()
                    .and_then(|input| input.handle_event(&Event::Key(key)))
                    .is_some_and(|change| change.value);
                if changed {
                    self.input_changed();
                }
                Action::None
            }
        }
    }

    fn focus_count(&self) -> usize {
        match self.stage {
            OnboardingScreen::JiraDetails => 3,
            OnboardingScreen::JiraToken => 2,
            OnboardingScreen::Save => 1,
        }
    }

    fn focus_next(&mut self) {
        self.focus = (self.focus + 1) % self.focus_count();
        self.reset_cursor_blink();
    }

    fn focus_previous(&mut self) {
        self.focus = (self.focus + self.focus_count() - 1) % self.focus_count();
        self.reset_cursor_blink();
    }

    fn focused_input_mut(&mut self) -> Option<&mut Input> {
        match (self.stage, self.focus) {
            (OnboardingScreen::JiraDetails, 0) => Some(&mut self.hostname),
            (OnboardingScreen::JiraDetails, 1) => Some(&mut self.email),
            (OnboardingScreen::JiraToken, 0) => Some(&mut self.jira_token),
            _ => None,
        }
    }

    fn insert_into_focused_input(&mut self, value: &str) -> bool {
        let Some(input) = self.focused_input_mut() else {
            return false;
        };
        let mut changed = false;
        for character in value.chars().filter(|character| !character.is_control()) {
            changed |= input.handle(InputRequest::InsertChar(character)).is_some();
        }
        changed
    }

    fn input_changed(&mut self) {
        self.reset_cursor_blink();
        self.jira_status = ConnectionStatus::NotConnected;
        self.error = None;
    }

    fn activate_or_advance(&mut self) -> Action {
        match self.stage {
            OnboardingScreen::JiraDetails if self.focus == 2 => Action::Continue,
            OnboardingScreen::JiraToken if self.focus == 1 => Action::ConnectJira,
            OnboardingScreen::Save => Action::Save,
            _ => {
                self.focus_next();
                Action::None
            }
        }
    }

    fn show_validation_error(&mut self, error: &OnboardingError) {
        self.focus = match error.field() {
            Some(JiraField::Site) => 0,
            Some(JiraField::AtlassianEmail) => 1,
            Some(JiraField::AtlassianToken | JiraField::AccountId) | None => 0,
        };
        self.jira_status = ConnectionStatus::NotConnected;
        self.error = Some(error.to_string());
    }

    fn set_stage(&mut self, stage: OnboardingScreen) {
        self.stage = stage;
        self.focus = 0;
        self.error = None;
        self.warning = None;
        self.reset_cursor_blink();
    }

    fn text_input_focused(&self) -> bool {
        matches!(
            (self.stage, self.focus),
            (OnboardingScreen::JiraDetails, 0 | 1) | (OnboardingScreen::JiraToken, 0)
        )
    }

    fn animations_active(&self) -> bool {
        (!self.reduced_motion && self.jira_status == ConnectionStatus::Pending)
            || (!self.reduced_motion && self.text_input_focused())
    }

    fn advance_animations(&mut self, elapsed: Duration) {
        if self.jira_status == ConnectionStatus::Pending {
            self.pending_animation_elapsed += elapsed;
        }
        if !self.reduced_motion && self.text_input_focused() {
            self.cursor_blink_elapsed += elapsed;
        }
    }

    fn cursor_visible(&self) -> bool {
        self.reduced_motion
            || (self.cursor_blink_elapsed.as_millis() / CURSOR_BLINK_HALF_PERIOD.as_millis())
                .is_multiple_of(2)
    }

    fn reset_cursor_blink(&mut self) {
        self.cursor_blink_elapsed = Duration::ZERO;
    }

    fn pending_symbol(&self) -> &'static str {
        if self.reduced_motion || self.jira_status != ConnectionStatus::Pending {
            return "…";
        }
        let frame = (self.pending_animation_elapsed.as_millis() / PLAYFUL_FRAME_RATE.as_millis())
            % SPINNER_FRAMES.len() as u128;
        SPINNER_FRAMES[frame as usize]
    }
}

enum Action {
    None,
    Continue,
    ConnectJira,
    Save,
    EditJira,
    Back,
    Cancel,
}

enum RuntimeEvent {
    Terminal(Event),
    Tick,
}

pub(crate) async fn run(
    workflow: OnboardingWorkflow<'_>,
    browser: &dyn BrowserLauncher,
) -> Result<CompletedLogin, CliError> {
    if !io::stdin().is_terminal() || !io::stderr().is_terminal() {
        return Err(CliError::InvalidInput(
            "interactive setup must run in a terminal; use `gd auth setup jira --from-env` for automation".to_owned(),
        ));
    }

    let reduced_motion = reduced_motion_requested();
    let mut model = OnboardingModel::new(&workflow, reduced_motion);
    let mut terminal = StderrTerminal::new()?;
    let mut events = EventStream::new();
    let mut ticker = tokio::time::interval(ANIMATION_TICK_RATE);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    let result = run_loop(
        workflow,
        browser,
        &mut model,
        &mut terminal,
        &mut events,
        &mut ticker,
    )
    .await;
    let restore_result = terminal.restore();
    match (result, restore_result) {
        (Err(error), _) => Err(error),
        (Ok(_), Err(error)) => Err(CliError::Io(error)),
        (Ok(completed), Ok(())) => Ok(completed),
    }
}

async fn run_loop(
    mut workflow: OnboardingWorkflow<'_>,
    browser: &dyn BrowserLauncher,
    model: &mut OnboardingModel,
    terminal: &mut StderrTerminal,
    events: &mut EventStream,
    ticker: &mut tokio::time::Interval,
) -> Result<CompletedLogin, CliError> {
    let mut undersized = terminal_is_undersized(terminal)?;
    loop {
        draw(terminal, model)?;
        let event = next_runtime_event(events, ticker, model.animations_active()).await?;
        match event {
            RuntimeEvent::Tick => {
                if !undersized {
                    model.advance_animations(ANIMATION_TICK_RATE);
                }
            }
            RuntimeEvent::Terminal(event) => {
                if let Event::Resize(width, height) = event {
                    undersized = size_is_undersized(width, height);
                    continue;
                }
                if undersized {
                    if is_cancel_event(&event) {
                        return Err(CliError::LoginCancelled);
                    }
                    continue;
                }
                match model.handle_event(event) {
                    Action::None => {}
                    Action::Cancel => return Err(CliError::LoginCancelled),
                    Action::Continue => {
                        match workflow
                            .continue_from_details(model.hostname.value(), model.email.value())
                        {
                            Ok(screen) => {
                                model.set_stage(screen);
                                present_token_page(model, &mut workflow, browser)?;
                            }
                            Err(error) => model.show_validation_error(&error),
                        }
                    }
                    Action::Back => back(model, &mut workflow)?,
                    Action::EditJira => edit_jira(model, &mut workflow)?,
                    Action::ConnectJira => {
                        verify_jira(
                            model,
                            &mut workflow,
                            terminal,
                            events,
                            ticker,
                            &mut undersized,
                        )
                        .await?;
                    }
                    Action::Save => return workflow.finish(),
                }
            }
        }
    }
}

fn back(
    model: &mut OnboardingModel,
    workflow: &mut OnboardingWorkflow<'_>,
) -> Result<(), CliError> {
    if model.stage == OnboardingScreen::JiraToken {
        model.jira_token = Input::default();
    }
    let Some(screen) = workflow.back()? else {
        return Err(CliError::LoginCancelled);
    };
    if screen == OnboardingScreen::JiraToken {
        model.jira_status = ConnectionStatus::NotConnected;
        model.jira_token = Input::default();
    }
    model.set_stage(screen);
    Ok(())
}

fn edit_jira(
    model: &mut OnboardingModel,
    workflow: &mut OnboardingWorkflow<'_>,
) -> Result<(), CliError> {
    let screen = workflow.edit_jira_details()?;
    model.jira_status = ConnectionStatus::NotConnected;
    model.jira_token = Input::default();
    model.set_stage(screen);
    Ok(())
}

fn present_token_page(
    model: &mut OnboardingModel,
    workflow: &mut OnboardingWorkflow<'_>,
    browser: &dyn BrowserLauncher,
) -> Result<(), CliError> {
    if model.jira_page_loaded {
        return Ok(());
    }
    let page = workflow.token_page()?;
    model.jira_instruction = page.instruction.to_owned();
    model.jira_url.clone_from(&page.url);
    model.jira_page_can_open = page.open_browser;
    model.jira_page_loaded = true;
    if page.open_browser {
        if let Err(error) = browser.open(&page.url) {
            model.warning = Some(format!(
                "Could not open token settings: {}. Use the URL shown below.",
                terminal_text::escape(&error.to_string())
            ));
        }
    }
    Ok(())
}

async fn verify_jira(
    model: &mut OnboardingModel,
    workflow: &mut OnboardingWorkflow<'_>,
    terminal: &mut StderrTerminal,
    events: &mut EventStream,
    ticker: &mut tokio::time::Interval,
    undersized: &mut bool,
) -> Result<(), CliError> {
    model.error = None;
    model.warning = None;
    model.jira_status = ConnectionStatus::Pending;
    model.pending_animation_elapsed = Duration::ZERO;
    draw(terminal, model)?;

    let token = if model.jira_token.value().is_empty() && model.can_retain_jira_token {
        SecretInput::Retain
    } else {
        SecretInput::Replace(model.jira_token.value().to_owned())
    };
    let outcome = {
        let verification = workflow.connect(token);
        tokio::pin!(verification);
        loop {
            tokio::select! {
                result = &mut verification => break Some(result?),
                event = events.next() => {
                    let event = event.ok_or(CliError::LoginCancelled)??;
                    if let Event::Resize(width, height) = event {
                        *undersized = size_is_undersized(width, height);
                        draw(terminal, model)?;
                    } else if is_cancel_event(&event) {
                        return Err(CliError::LoginCancelled);
                    } else if is_back_event(&event) {
                        break None;
                    }
                }
                _ = ticker.tick(), if !*undersized => {
                    model.advance_animations(ANIMATION_TICK_RATE);
                    draw(terminal, model)?;
                }
            }
        }
    };

    match outcome {
        Some(ConnectionOutcome::Connected) => {
            apply_verified_login(model, workflow)?;
            model.jira_status = ConnectionStatus::Connected;
            model.set_stage(OnboardingScreen::Save);
        }
        Some(ConnectionOutcome::Rejected) => {
            model.jira_status = ConnectionStatus::NotConnected;
            model.jira_token = Input::default();
            model.focus = 0;
            model.error = Some(
                "Could not connect to Jira: Jira rejected the site, Atlassian email, or API token."
                    .to_owned(),
            );
        }
        Some(ConnectionOutcome::Invalid(error)) => {
            model.jira_token = Input::default();
            model.show_validation_error(&error);
        }
        None => {
            model.jira_status = ConnectionStatus::NotConnected;
            model.warning = Some("Jira verification cancelled. Nothing was saved.".to_owned());
        }
    }
    Ok(())
}

fn apply_verified_login(
    model: &mut OnboardingModel,
    workflow: &OnboardingWorkflow<'_>,
) -> Result<(), CliError> {
    let completed = workflow.verified_login().ok_or_else(|| {
        CliError::InvalidInput("verified Jira authentication state is missing".to_owned())
    })?;
    model.hostname = completed.credentials().site().as_str().into();
    model.email = completed.credentials().email().as_str().into();
    model.display_name = if completed.identity().display_name().is_empty() {
        completed.credentials().email().as_str().to_owned()
    } else {
        completed.identity().display_name().to_owned()
    };
    model.jira_token = Input::default();
    model.can_retain_jira_token = workflow.can_retain_token();
    Ok(())
}

async fn next_runtime_event(
    events: &mut EventStream,
    ticker: &mut tokio::time::Interval,
    animations_active: bool,
) -> Result<RuntimeEvent, CliError> {
    if animations_active {
        tokio::select! {
            event = events.next() => event
                .ok_or(CliError::LoginCancelled)?
                .map(RuntimeEvent::Terminal)
                .map_err(CliError::Io),
            _ = ticker.tick() => Ok(RuntimeEvent::Tick),
        }
    } else {
        events
            .next()
            .await
            .ok_or(CliError::LoginCancelled)?
            .map(RuntimeEvent::Terminal)
            .map_err(CliError::Io)
    }
}

fn reduced_motion_requested() -> bool {
    let value = std::env::var(REDUCED_MOTION_ENV).ok();
    reduced_motion_value(value.as_deref())
}

fn reduced_motion_value(value: Option<&str>) -> bool {
    value.is_some_and(|value| {
        let value = value.trim();
        value == "1"
            || value.eq_ignore_ascii_case("true")
            || value.eq_ignore_ascii_case("yes")
            || value.eq_ignore_ascii_case("on")
    })
}

fn is_cancel_event(event: &Event) -> bool {
    matches!(
        event,
        Event::Key(key)
            if matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat)
                && key.modifiers.contains(KeyModifiers::CONTROL)
                && key.code == KeyCode::Char('c')
    )
}

fn is_back_event(event: &Event) -> bool {
    matches!(
        event,
        Event::Key(key)
            if matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat)
                && key.code == KeyCode::Esc
    )
}

fn terminal_is_undersized(terminal: &mut StderrTerminal) -> Result<bool, CliError> {
    let size = terminal.terminal_mut().size()?;
    Ok(size_is_undersized(size.width, size.height))
}

const fn size_is_undersized(width: u16, height: u16) -> bool {
    width < MIN_TERMINAL_WIDTH || height < MIN_TERMINAL_HEIGHT
}

fn draw(terminal: &mut StderrTerminal, model: &mut OnboardingModel) -> Result<(), CliError> {
    terminal.terminal_mut().draw(|frame| {
        render(frame, model);
    })?;
    Ok(())
}

fn render(frame: &mut Frame<'_>, model: &OnboardingModel) {
    if size_is_undersized(frame.area().width, frame.area().height) {
        render_resize_message(frame, frame.area());
        return;
    }
    let [_top_padding, header, _gap, body, footer] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(GRADUATE_ART_HEIGHT),
        Constraint::Length(2),
        Constraint::Fill(1),
        Constraint::Length(1),
    ])
    .areas(frame.area());
    let header = constrain_content_width(header);
    let body = constrain_content_width(body);
    let footer = constrain_content_width(footer);

    render_brand_header(frame, header);
    match model.stage {
        OnboardingScreen::JiraDetails => render_jira_details(frame, body, model),
        OnboardingScreen::JiraToken => render_jira_token(frame, body, model),
        OnboardingScreen::Save => render_save(frame, body, model),
    }
    render_footer(frame, footer, model);
}

fn render_resize_message(frame: &mut Frame<'_>, area: Rect) {
    let message = Text::from(vec![
        Line::from("Terminal too small").bold(),
        Line::default(),
        Line::from(format!(
            "Current size: {} columns by {} rows.",
            area.width, area.height
        )),
        Line::from(format!(
            "Resize to at least {MIN_TERMINAL_WIDTH} columns by {MIN_TERMINAL_HEIGHT} rows to continue."
        )),
        Line::from("Your input is preserved.").dim(),
        Line::from("Ctrl-C cancels without saving.").dim(),
    ]);
    frame.render_widget(
        Paragraph::new(message).centered().wrap(Wrap { trim: true }),
        area,
    );
}

fn render_jira_details(frame: &mut Frame<'_>, area: Rect, model: &OnboardingModel) {
    let [intro, _, hostname, _, email, _, action, _, feedback] = Layout::vertical([
        Constraint::Length(2),
        Constraint::Length(2),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(2),
        Constraint::Length(3),
        Constraint::Length(1),
        Constraint::Fill(1),
    ])
    .areas(area);
    frame.render_widget(
        Paragraph::new(Text::from(vec![
            Line::from("Connect your Jira account").bold(),
            Line::from("Enter the Atlassian account Graduate should use.").dim(),
        ])),
        intro,
    );
    render_field(
        frame,
        hostname,
        "Jira site",
        &terminal_text::escape(model.hostname.value()),
        "company.atlassian.net",
        FieldPresentation {
            cursor: model.hostname.cursor(),
            focused: model.focus == 0,
            cursor_visible: model.cursor_visible(),
            invalid: model
                .error
                .as_deref()
                .is_some_and(|error| error.contains("Jira site")),
            ..FieldPresentation::default()
        },
    );
    render_field(
        frame,
        email,
        "Atlassian email",
        &terminal_text::escape(model.email.value()),
        "you@example.com",
        FieldPresentation {
            cursor: model.email.cursor(),
            focused: model.focus == 1,
            cursor_visible: model.cursor_visible(),
            invalid: model
                .error
                .as_deref()
                .is_some_and(|error| error.contains("Atlassian email")),
            ..FieldPresentation::default()
        },
    );
    frame.render_widget(
        SetupButton::new(
            "Continue to API token",
            model.focus == 2,
            ConnectionStatus::NotConnected,
            model.pending_symbol(),
        ),
        action,
    );
    render_feedback(frame, feedback, model);
}

fn render_jira_token(frame: &mut Frame<'_>, area: Rect, model: &OnboardingModel) {
    let fallback_height = if !model.jira_page_can_open || model.warning.is_some() {
        3
    } else {
        0
    };
    let [intro, _, token, _, raw_url, _, status, _, feedback] = Layout::vertical([
        Constraint::Length(2),
        Constraint::Length(2),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(fallback_height),
        Constraint::Length(1),
        Constraint::Length(3),
        Constraint::Length(1),
        Constraint::Fill(1),
    ])
    .areas(area);
    frame.render_widget(
        Paragraph::new(Text::from(vec![
            Line::from("Connect Jira").bold(),
            Line::from("Paste an Atlassian API token.").dim(),
        ])),
        intro,
    );
    render_field(
        frame,
        token,
        "Atlassian API token",
        model.jira_token.value(),
        "paste token",
        FieldPresentation {
            cursor: model.jira_token.cursor(),
            focused: model.focus == 0,
            cursor_visible: model.cursor_visible(),
            masked: true,
            can_retain_secret: model.can_retain_jira_token,
            invalid: model
                .error
                .as_deref()
                .is_some_and(|error| error.contains("Atlassian API token")),
        },
    );
    render_token_url_fallback(
        frame,
        raw_url,
        &model.jira_instruction,
        &model.jira_url,
        model.jira_page_can_open,
        model.warning.is_some(),
    );
    frame.render_widget(
        SetupButton::new(
            "Connect Jira",
            model.focus == 1,
            model.jira_status,
            model.pending_symbol(),
        ),
        status,
    );
    render_feedback(frame, feedback, model);
}

fn render_token_url_fallback(
    frame: &mut Frame<'_>,
    area: Rect,
    instruction: &str,
    url: &str,
    can_open: bool,
    open_failed: bool,
) {
    if !can_open || open_failed {
        frame.render_widget(
            Paragraph::new(Text::from(vec![
                Line::from(terminal_text::escape(instruction)).dim(),
                Line::from(terminal_text::escape(url)).underlined(),
            ]))
            .wrap(Wrap { trim: false }),
            area,
        );
    }
}

fn render_save(frame: &mut Frame<'_>, area: Rect, model: &OnboardingModel) {
    let [intro, _, manifest, _, question, _, action, edit, _, feedback] = Layout::vertical([
        Constraint::Length(2),
        Constraint::Length(1),
        Constraint::Length(5),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(3),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Fill(1),
    ])
    .areas(area);
    frame.render_widget(
        Paragraph::new(Text::from(vec![
            Line::from("Review Jira setup").bold(),
            Line::from("Graduate will save this connection.").dim(),
        ])),
        intro,
    );
    frame.render_widget(
        Paragraph::new(Text::from(vec![
            Line::from(vec![
                ratatui::text::Span::styled(
                    format!("{:<REVIEW_LABEL_WIDTH$}", "Field"),
                    Palette::text().bold(),
                ),
                ratatui::text::Span::styled("Value", Palette::text().bold()),
            ]),
            Line::styled("─".repeat(usize::from(manifest.width)), Palette::muted()),
            detail_line("Site", &terminal_text::escape(model.hostname.value())),
            detail_line("Account", &terminal_text::escape(model.email.value())),
            detail_line("Identity", &terminal_text::escape(&model.display_name)),
        ])),
        manifest,
    );
    frame.render_widget(Paragraph::new("Does this look right?").bold(), question);
    frame.render_widget(
        SetupButton::new(
            "Save configuration",
            true,
            ConnectionStatus::NotConnected,
            model.pending_symbol(),
        ),
        action,
    );
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            ratatui::text::Span::styled("J ", Palette::primary().bold()),
            ratatui::text::Span::styled("Change Jira account", Palette::muted()),
        ])),
        edit,
    );
    render_feedback(frame, feedback, model);
}

fn detail_line<'a>(label: &'static str, value: &'a str) -> Line<'a> {
    Line::from(vec![
        ratatui::text::Span::styled(format!("{label:<REVIEW_LABEL_WIDTH$}"), Palette::muted()),
        ratatui::text::Span::styled(value, Palette::text()),
    ])
}

#[derive(Default)]
struct FieldPresentation {
    focused: bool,
    cursor_visible: bool,
    cursor: usize,
    masked: bool,
    can_retain_secret: bool,
    invalid: bool,
}

fn render_field(
    frame: &mut Frame<'_>,
    area: Rect,
    label: &str,
    value: &str,
    placeholder: &str,
    presentation: FieldPresentation,
) {
    let FieldPresentation {
        focused,
        cursor_visible,
        cursor,
        masked,
        can_retain_secret,
        invalid,
    } = presentation;
    let retained = masked && value.is_empty() && can_retain_secret;
    let display = if retained {
        "••••••••••••".to_owned()
    } else if masked {
        "•".repeat(value.chars().count())
    } else {
        value.to_owned()
    };
    let prompt_style = if invalid {
        Palette::error()
    } else if focused {
        Palette::focus()
    } else {
        Palette::muted()
    };
    let prompt = if retained {
        format!("{label} (stored)> ")
    } else {
        format!("{label}> ")
    };
    let shown_value = if display.is_empty() {
        placeholder
    } else {
        &display
    };
    let value_style = if display.is_empty() {
        Palette::muted()
    } else {
        Palette::text()
    };
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            ratatui::text::Span::styled(prompt.clone(), prompt_style.bold()),
            ratatui::text::Span::styled(shown_value.to_owned(), value_style),
        ])),
        area,
    );

    if focused && cursor_visible && !retained {
        let prompt_width = u16::try_from(prompt.chars().count()).unwrap_or(area.width);
        let available = area.width.saturating_sub(prompt_width).saturating_sub(1);
        let cursor_offset = u16::try_from(cursor.min(usize::from(available))).unwrap_or(available);
        if let Some(cell) = frame
            .buffer_mut()
            .cell_mut(Position::new(area.x + prompt_width + cursor_offset, area.y))
        {
            cell.set_style(Style::new().reversed());
        }
    }
}

struct SetupButton<'a> {
    label: &'a str,
    focused: bool,
    status: ConnectionStatus,
    pending_symbol: &'a str,
}

impl<'a> SetupButton<'a> {
    const fn new(
        label: &'a str,
        focused: bool,
        status: ConnectionStatus,
        pending_symbol: &'a str,
    ) -> Self {
        Self {
            label,
            focused,
            status,
            pending_symbol,
        }
    }
}

impl Widget for SetupButton<'_> {
    fn render(self, area: Rect, buffer: &mut Buffer) {
        let focused = self.focused && self.status != ConnectionStatus::Pending;
        let text = match self.status {
            ConnectionStatus::Pending => format!("{} Verifying Jira…", self.pending_symbol),
            ConnectionStatus::Connected if self.label != "Save configuration" => {
                format!("✓ {} connected", self.label)
            }
            _ => self.label.to_owned(),
        };
        let content_style = match self.status {
            ConnectionStatus::Pending => Palette::pending().bg(MUTED_COLOR),
            ConnectionStatus::Connected => Palette::success().bg(MUTED_COLOR),
            ConnectionStatus::NotConnected if focused => Palette::action_focus(),
            ConnectionStatus::NotConnected => Palette::muted(),
        };
        let border_style = match self.status {
            ConnectionStatus::Pending => Palette::pending(),
            ConnectionStatus::Connected => Palette::success(),
            ConnectionStatus::NotConnected if focused => Palette::primary(),
            ConnectionStatus::NotConnected => Palette::muted(),
        };
        let width = u16::try_from(text.chars().count())
            .unwrap_or(area.width)
            .saturating_add(6)
            .min(area.width);
        let button = Rect::new(area.x, area.y, width, area.height);
        Block::bordered()
            .border_type(BorderType::Plain)
            .border_style(border_style)
            .render(button, buffer);
        let interior = Rect::new(
            button.x.saturating_add(1),
            button.y.saturating_add(1),
            button.width.saturating_sub(2),
            button.height.saturating_sub(2),
        );
        Paragraph::new(Line::styled(text, content_style))
            .centered()
            .style(content_style)
            .render(interior, buffer);
    }
}

fn render_feedback(frame: &mut Frame<'_>, area: Rect, model: &OnboardingModel) {
    let line = if let Some(error) = &model.error {
        Line::styled(
            format!("✕ Error: {}", terminal_text::escape(error)),
            Palette::error(),
        )
    } else if let Some(warning) = &model.warning {
        Line::styled(
            format!("! Warning: {}", terminal_text::escape(warning)),
            Palette::warning(),
        )
    } else {
        Line::default()
    };
    frame.render_widget(Paragraph::new(line).wrap(Wrap { trim: true }), area);
}

fn render_footer(frame: &mut Frame<'_>, area: Rect, model: &OnboardingModel) {
    let controls = match model.stage {
        OnboardingScreen::JiraDetails => "↑↓/tab navigate • enter select • esc cancel",
        OnboardingScreen::JiraToken => "↑↓/tab navigate • enter select • esc back",
        OnboardingScreen::Save => "enter save • j change account • esc change token",
    };
    frame.render_widget(Paragraph::new(controls).style(Palette::muted()), area);
}

#[cfg(test)]
mod tests {
    use graduate::jira::JiraValidationError;
    use ratatui::backend::TestBackend;
    use ratatui::style::Modifier;
    use ratatui::Terminal;

    use crate::theme::{GRADUATE_ART, MUTED_COLOR, PRIMARY_COLOR};

    use super::*;

    const TEST_WIDTH: u16 = 100;
    const TEST_HEIGHT: u16 = 50;

    fn model(stage: OnboardingScreen) -> OnboardingModel {
        OnboardingModel {
            pending_animation_elapsed: Duration::ZERO,
            cursor_blink_elapsed: Duration::ZERO,
            reduced_motion: true,
            stage,
            focus: 0,
            hostname: "company.atlassian.net".into(),
            email: "person@example.com".into(),
            display_name: "Example Person".to_owned(),
            jira_token: Input::default(),
            can_retain_jira_token: false,
            jira_instruction: "Create or manage your Atlassian API token:".to_owned(),
            jira_url: "https://id.atlassian.com/manage-profile/security/api-tokens".to_owned(),
            jira_page_can_open: false,
            jira_page_loaded: true,
            jira_status: ConnectionStatus::NotConnected,
            error: None,
            warning: None,
        }
    }

    fn render_text(
        width: u16,
        height: u16,
        model: &OnboardingModel,
    ) -> Result<String, Box<dyn std::error::Error>> {
        let mut terminal = Terminal::new(TestBackend::new(width, height))?;
        terminal.draw(|frame| {
            render(frame, model);
        })?;
        let buffer = terminal.backend().buffer();
        let mut rendered = String::new();
        for row in 0..buffer.area.height {
            for column in 0..buffer.area.width {
                rendered.push_str(buffer[(column, row)].symbol());
            }
            rendered.push('\n');
        }
        Ok(rendered)
    }

    #[test]
    fn jira_details_use_compact_inline_prompts() -> Result<(), Box<dyn std::error::Error>> {
        let rendered = render_text(
            TEST_WIDTH,
            TEST_HEIGHT,
            &model(OnboardingScreen::JiraDetails),
        )?;

        assert!(rendered.contains(GRADUATE_ART[0]));
        assert!(rendered.contains("Jira site> company.atlassian.net"));
        assert!(rendered.contains("Atlassian email> person@example.com"));
        assert!(rendered.contains("Continue to API token"));
        assert!(!rendered.contains("Your Atlassian workspace address"));
        assert!(rendered.contains("┌"));
        assert!(rendered.contains("┘"));
        assert!(!rendered.contains("Review & save"));
        let heading = rendered
            .lines()
            .find(|line| line.contains("Connect your Jira account"))
            .ok_or("Jira heading was not rendered")?;
        assert!(heading.starts_with("Connect your Jira account"));
        assert!(rendered.contains("Enter the Atlassian account Graduate should use"));
        Ok(())
    }

    #[test]
    fn enter_moves_through_both_fields_to_the_continue_button() {
        let mut model = model(OnboardingScreen::JiraDetails);

        assert!(matches!(
            model.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
            Action::None
        ));
        assert_eq!(model.focus, 1);
        assert!(matches!(
            model.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
            Action::None
        ));
        assert_eq!(model.focus, 2);
        assert!(matches!(
            model.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
            Action::Continue
        ));
    }

    #[test]
    fn arrow_keys_move_focus_in_both_directions() {
        let mut model = model(OnboardingScreen::JiraDetails);

        let _ = model.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        assert_eq!(model.focus, 1);

        let _ = model.handle_key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
        assert_eq!(model.focus, 0);

        let _ = model.handle_key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
        assert_eq!(model.focus, 2);
    }

    #[test]
    fn text_input_edits_at_the_unicode_aware_cursor() {
        let mut model = model(OnboardingScreen::JiraDetails);
        model.hostname = "café".into();

        let _ = model.handle_key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE));
        let _ = model.handle_key(KeyEvent::new(KeyCode::Char('!'), KeyModifiers::NONE));

        assert_eq!(model.hostname.value(), "caf!é");
    }

    #[test]
    fn token_screen_masks_input_and_renders_a_focusable_connect_button(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let mut model = model(OnboardingScreen::JiraToken);
        model.jira_token = "never-render-this-secret".into();
        model.focus = 1;

        let rendered = render_text(TEST_WIDTH, TEST_HEIGHT, &model)?;

        assert!(rendered.contains("Atlassian API token"));
        assert!(rendered.contains("••••"));
        assert!(!rendered.contains("never-render-this-secret"));
        assert!(rendered.contains("Connect Jira"));
        assert!(rendered.contains("┌"));
        Ok(())
    }

    #[test]
    fn retained_token_is_labeled_without_loading_the_secret(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let mut model = model(OnboardingScreen::JiraToken);
        model.can_retain_jira_token = true;

        let rendered = render_text(TEST_WIDTH, TEST_HEIGHT, &model)?;

        assert!(rendered.contains("Atlassian API token (stored)"));
        assert!(rendered.contains("••••••••••••"));
        Ok(())
    }

    #[test]
    fn review_uses_a_compact_field_table_and_confirmation() -> Result<(), Box<dyn std::error::Error>>
    {
        let mut model = model(OnboardingScreen::Save);
        model.jira_status = ConnectionStatus::Connected;

        let rendered = render_text(TEST_WIDTH, TEST_HEIGHT, &model)?;

        assert!(rendered.contains("Review Jira setup"));
        assert!(rendered.contains("Field     Value"));
        assert!(rendered.contains("Site      company.atlassian.net"));
        assert!(rendered.contains("Does this look right?"));
        assert!(rendered.contains("Save configuration"));
        assert!(rendered.contains("┌"));
        assert!(rendered.contains("J Change Jira account"));
        assert!(!rendered.contains("Tempo"));
        Ok(())
    }

    #[test]
    fn action_uses_the_main_tui_focus_style() -> Result<(), Box<dyn std::error::Error>> {
        for focus in [0, 2] {
            let mut model = model(OnboardingScreen::JiraDetails);
            model.focus = focus;
            let mut terminal = Terminal::new(TestBackend::new(TEST_WIDTH, TEST_HEIGHT))?;
            terminal.draw(|frame| render(frame, &model))?;

            let corners = terminal
                .backend()
                .buffer()
                .content
                .iter()
                .filter(|cell| matches!(cell.symbol(), "┌" | "┐" | "└" | "┘"))
                .collect::<Vec<_>>();
            assert_eq!(corners.len(), 4);
            if focus == 2 {
                assert!(corners.iter().all(|cell| cell.fg == PRIMARY_COLOR));
                assert!(corners.iter().all(|cell| cell.bg != MUTED_COLOR));
            } else {
                assert!(corners.iter().all(|cell| cell.fg == MUTED_COLOR));
                assert!(corners.iter().all(|cell| cell.bg != MUTED_COLOR));
            }
            let focused_cells = terminal
                .backend()
                .buffer()
                .content
                .iter()
                .filter(|cell| cell.modifier.contains(Modifier::REVERSED))
                .count();
            if focus == 2 {
                assert!(focused_cells >= "Continue to API token".len());
            } else {
                assert_eq!(focused_cells, 1);
            }

            let left_edge = terminal
                .backend()
                .buffer()
                .content
                .iter()
                .position(|cell| cell.symbol() == "┌")
                .ok_or("button left edge was not rendered")?;
            assert_eq!(left_edge % usize::from(TEST_WIDTH), 0);
        }
        Ok(())
    }

    #[test]
    fn resize_message_replaces_the_form_but_preserves_cancel_help(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let rendered = render_text(
            MIN_TERMINAL_WIDTH - 1,
            MIN_TERMINAL_HEIGHT - 1,
            &model(OnboardingScreen::JiraDetails),
        )?;

        assert!(rendered.contains("Terminal too small"));
        assert!(rendered.contains("Ctrl-C cancels without saving"));
        assert!(!rendered.contains("Atlassian email"));
        Ok(())
    }

    #[test]
    fn every_screen_fits_a_60_by_24_split_pane() -> Result<(), Box<dyn std::error::Error>> {
        for (stage, heading, action) in [
            (
                OnboardingScreen::JiraDetails,
                "Connect your Jira account",
                "Continue to API token",
            ),
            (OnboardingScreen::JiraToken, "Connect Jira", "Connect Jira"),
            (
                OnboardingScreen::Save,
                "Review Jira setup",
                "Save configuration",
            ),
        ] {
            let rendered = render_text(60, 24, &model(stage))?;

            assert!(!rendered.contains("Terminal too small"));
            assert!(rendered.contains(GRADUATE_ART[0]));
            assert!(rendered.contains(heading));
            assert!(rendered.contains(action));
            assert!(rendered.contains("┌"));
            assert!(rendered.contains("┘"));
        }
        Ok(())
    }

    #[test]
    fn untrusted_review_values_are_rendered_visibly() -> Result<(), Box<dyn std::error::Error>> {
        let mut model = model(OnboardingScreen::Save);
        model.display_name = "Person\nInjected\u{202e}".to_owned();

        let rendered = render_text(TEST_WIDTH, TEST_HEIGHT, &model)?;

        assert!(rendered.contains("Person\\nInjected\\u{202e}"));
        assert!(!rendered.contains("Person\nInjected"));
        Ok(())
    }

    #[test]
    fn reduced_motion_environment_accepts_only_explicit_truthy_values() {
        for value in [Some("1"), Some("true"), Some("YES"), Some("on")] {
            assert!(reduced_motion_value(value));
        }
        for value in [None, Some(""), Some("0"), Some("false"), Some("no")] {
            assert!(!reduced_motion_value(value));
        }
    }

    #[test]
    fn core_validation_errors_select_the_matching_field() {
        let mut model = model(OnboardingScreen::JiraDetails);
        let error = OnboardingError::JiraValidation(JiraValidationError::AtlassianEmailRequired);

        model.show_validation_error(&error);

        assert_eq!(model.focus, 1);
        assert_eq!(model.error.as_deref(), Some("Atlassian email is required"));
    }
}
