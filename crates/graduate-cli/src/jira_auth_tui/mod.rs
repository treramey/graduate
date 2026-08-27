//! Ratatui presentation and Crossterm runtime for interactive Jira authentication.

use std::io::{self, IsTerminal};
use std::time::Duration;

use crossterm::event::{Event, EventStream};
use futures_util::StreamExt;
use graduate::jira::JiraField;
use graduate::jira_auth::{CompletedLogin, OnboardingError, OnboardingScreen, SecretInput};
use tui_input::Input;

use crate::browser::BrowserLauncher;
use crate::error::CliError;
use crate::jira_auth::{ConnectionOutcome, OnboardingWorkflow};
use crate::terminal::StderrTerminal;
use events::{
    is_back_event, is_cancel_event, reduced_motion_requested, size_is_undersized,
    terminal_is_undersized, Action, RuntimeEvent,
};
use render::draw;
use transitions::{apply_verified_login, back, edit_jira, present_token_page};

mod animation;
mod events;
mod input;
mod render;
#[cfg(test)]
mod tests;
mod transitions;
mod widgets;

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
