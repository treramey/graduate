//! Ratatui presentation and Crossterm runtime for interactive Jira authentication.

use std::io::{self, IsTerminal};
use std::time::Duration;

use crossterm::event::{Event, EventStream, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use futures_util::StreamExt;
use graduate::jira::JiraField;
use graduate::jira_auth::{CompletedLogin, OnboardingError, OnboardingScreen, SecretInput};
use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Layout, Position, Rect};
use ratatui::style::{Color, Style, Stylize};
use ratatui::text::{Line, Text};
use ratatui::widgets::{Block, Paragraph, Wrap};
use ratatui::Frame;
use tachyonfx::{fx, CellFilter, Effect, Interpolation, SimpleRng};
use tui_input::backend::crossterm::EventHandler;
use tui_input::{Input, InputRequest};

use crate::browser::BrowserLauncher;
use crate::error::CliError;
use crate::jira_auth::{ConnectionOutcome, OnboardingWorkflow};
use crate::terminal::StderrTerminal;
use crate::terminal_text;
use crate::theme::{
    constrain_content_width, footer_divider, render_brand_header, Palette, GRADUATE_ART_HEIGHT,
    MAX_CONTENT_WIDTH, MUTED_COLOR, PRIMARY_COLOR, SUCCESS_COLOR,
};

// The longest fixed login control row needs 73 cells; retain a small margin.
const MIN_TERMINAL_WIDTH: u16 = 76;
const MIN_TERMINAL_HEIGHT: u16 = 48;
const MAX_FORM_WIDTH: u16 = 80;
const SPACE_SM: u16 = 1;
const SPACE_MD: u16 = 2;
const REVIEW_LABEL_WIDTH: usize = 11;
const ANIMATION_TICK_RATE: Duration = Duration::from_millis(40);
const PLAYFUL_FRAME_RATE: Duration = Duration::from_millis(80);
const CURSOR_BLINK_HALF_PERIOD: Duration = Duration::from_millis(500);
const ENTRANCE_DURATION_MS: u32 = 240;
const REDUCED_MOTION_DURATION_MS: u32 = 140;
const ANIMATION_RNG_SEED: u32 = 0x4752_4144;
const FOCUS_BORDER_CELLS_PER_SECOND: f32 = 30.0;
const STAGE_SEPARATOR: &str = " ─── ";
const SPINNER_FRAMES: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
pub(crate) const REDUCED_MOTION_ENV: &str = "GRADUATE_REDUCED_MOTION";

#[derive(Clone, Copy, PartialEq, Eq)]
enum ConnectionStatus {
    NotConnected,
    Pending,
    Connected,
}

struct BufferAnimation {
    effect: Option<Effect>,
    elapsed: Duration,
}

impl BufferAnimation {
    fn entrance(reduced_motion: bool) -> Self {
        let effect = if reduced_motion {
            fx::fade_from_fg(
                MUTED_COLOR,
                (REDUCED_MOTION_DURATION_MS, Interpolation::CubicOut),
            )
            .with_filter(CellFilter::AnyOf(vec![
                CellFilter::FgColor(PRIMARY_COLOR),
                CellFilter::FgColor(MUTED_COLOR),
            ]))
        } else {
            fx::coalesce((ENTRANCE_DURATION_MS, Interpolation::CubicOut))
                .with_rng(SimpleRng::new(ANIMATION_RNG_SEED))
                .with_filter(CellFilter::Text)
        };
        Self {
            effect: Some(effect),
            elapsed: Duration::ZERO,
        }
    }

    fn connection(reduced_motion: bool) -> Self {
        let effect = if reduced_motion {
            fx::fade_from_fg(
                MUTED_COLOR,
                (REDUCED_MOTION_DURATION_MS, Interpolation::CubicOut),
            )
            .with_filter(CellFilter::FgColor(SUCCESS_COLOR))
        } else {
            fx::coalesce((ENTRANCE_DURATION_MS, Interpolation::CubicOut))
                .with_rng(SimpleRng::new(ANIMATION_RNG_SEED))
                .with_filter(CellFilter::Text)
        };
        Self {
            effect: Some(effect),
            elapsed: Duration::ZERO,
        }
    }

    const fn is_active(&self) -> bool {
        self.effect.is_some()
    }

    fn advance(&mut self, elapsed: Duration) {
        if self.effect.is_some() {
            self.elapsed += elapsed;
        }
    }

    fn complete(&mut self) {
        self.effect = None;
        self.elapsed = Duration::ZERO;
    }

    fn render(&mut self, frame: &mut Frame<'_>, area: Rect) {
        let Some(effect) = self.effect.as_mut() else {
            return;
        };
        effect.process(self.elapsed, frame.buffer_mut(), area);
        self.elapsed = Duration::ZERO;
        if effect.done() {
            self.effect = None;
        }
    }
}

struct OnboardingModel {
    entrance_animation: BufferAnimation,
    connection_animation: Option<BufferAnimation>,
    pending_animation_elapsed: Duration,
    focus_border_elapsed: Duration,
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
            entrance_animation: BufferAnimation::entrance(reduced_motion),
            connection_animation: None,
            pending_animation_elapsed: Duration::ZERO,
            focus_border_elapsed: Duration::ZERO,
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
        if matches!(event, Event::Key(_)) {
            self.entrance_animation.complete();
            if let Some(animation) = self.connection_animation.as_mut() {
                animation.complete();
            }
        }
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
        self.entrance_animation.is_active()
            || self
                .connection_animation
                .as_ref()
                .is_some_and(BufferAnimation::is_active)
            || (!self.reduced_motion && self.jira_status == ConnectionStatus::Pending)
            || (!self.reduced_motion && self.text_input_focused())
    }

    fn advance_animations(&mut self, elapsed: Duration) {
        self.entrance_animation.advance(elapsed);
        if let Some(animation) = self.connection_animation.as_mut() {
            animation.advance(elapsed);
        }
        if self.jira_status == ConnectionStatus::Pending {
            self.pending_animation_elapsed += elapsed;
        }
        if !self.reduced_motion && self.text_input_focused() {
            self.focus_border_elapsed += elapsed;
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
            "interactive setup requires terminal-capable stdin and stderr; use `gd auth setup jira --from-env` for automation".to_owned(),
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
            model.connection_animation = Some(BufferAnimation::connection(model.reduced_motion));
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
    terminal
        .terminal_mut()
        .draw(|frame| render_animated(frame, model))?;
    Ok(())
}

struct AnimatedAreas {
    brand: Rect,
    jira_status: Rect,
    focused_input: Option<Rect>,
}

fn render_animated(frame: &mut Frame<'_>, model: &mut OnboardingModel) {
    let Some(areas) = render(frame, model) else {
        return;
    };
    model.entrance_animation.render(frame, areas.brand);
    if let Some(animation) = model.connection_animation.as_mut() {
        animation.render(frame, areas.jira_status);
    }
    if let Some(focused_input) = areas.focused_input {
        render_focus_border(
            frame.buffer_mut(),
            focused_input,
            model.focus_border_elapsed,
        );
    }
}

fn render(frame: &mut Frame<'_>, model: &OnboardingModel) -> Option<AnimatedAreas> {
    if size_is_undersized(frame.area().width, frame.area().height) {
        render_resize_message(frame, frame.area());
        return None;
    }
    let [_top_padding, header, body, footer] = Layout::vertical([
        Constraint::Length(2),
        Constraint::Length(GRADUATE_ART_HEIGHT + 2),
        Constraint::Fill(1),
        Constraint::Length(2),
    ])
    .areas(frame.area());
    let header = constrain_content_width(header);
    let body_width = if model.stage == OnboardingScreen::Save {
        MAX_CONTENT_WIDTH
    } else {
        MAX_FORM_WIDTH
    };
    let body = constrain_width_left(constrain_content_width(body), body_width);
    let footer = constrain_content_width(footer);

    let jira_status = render_header(frame, header, model);
    let focused_input = match model.stage {
        OnboardingScreen::JiraDetails => render_jira_details(frame, body, model),
        OnboardingScreen::JiraToken => render_jira_token(frame, body, model),
        OnboardingScreen::Save => {
            render_save(frame, body, model);
            None
        }
    };
    render_footer(frame, footer, model);
    Some(AnimatedAreas {
        brand: Rect::new(header.x, header.y, header.width, GRADUATE_ART_HEIGHT),
        jira_status,
        focused_input: (!model.reduced_motion && model.text_input_focused())
            .then_some(focused_input)
            .flatten(),
    })
}

fn constrain_width_left(area: Rect, maximum: u16) -> Rect {
    Rect::new(area.x, area.y, area.width.min(maximum), area.height)
}

struct FormSpacing {
    related: u16,
    section: u16,
}

const fn form_spacing(area: Rect, spacious_height: u16) -> FormSpacing {
    if area.height >= spacious_height {
        FormSpacing {
            related: SPACE_SM,
            section: SPACE_MD,
        }
    } else if area.height >= 16 {
        FormSpacing {
            related: SPACE_SM,
            section: SPACE_SM,
        }
    } else {
        FormSpacing {
            related: 0,
            section: 0,
        }
    }
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
        Line::from("Your entered authentication values are preserved.").dim(),
        Line::from("Ctrl-C cancels without saving.").dim(),
    ]);
    frame.render_widget(
        Paragraph::new(message)
            .centered()
            .wrap(Wrap { trim: true })
            .block(Block::bordered().title(" Graduate · Jira setup ")),
        area,
    );
}

fn render_header(frame: &mut Frame<'_>, area: Rect, model: &OnboardingModel) -> Rect {
    render_brand_header(frame, area);
    let pending_symbol = model.pending_symbol();
    let jira_status = if model.stage == OnboardingScreen::Save {
        ConnectionStatus::Connected
    } else {
        model.jira_status
    };
    let stages = Line::from(vec![
        stage_span(
            "Jira account",
            model.stage != OnboardingScreen::Save,
            jira_status,
            pending_symbol,
        ),
        ratatui::text::Span::styled(STAGE_SEPARATOR, Palette::muted()),
        stage_span(
            "Review & save",
            model.stage == OnboardingScreen::Save,
            ConnectionStatus::NotConnected,
            pending_symbol,
        ),
    ]);
    frame.render_widget(
        Paragraph::new(stages),
        Rect::new(
            area.x,
            area.y.saturating_add(GRADUATE_ART_HEIGHT + 1),
            area.width,
            1,
        ),
    );
    let jira_width = u16::try_from("Jira account".len() + 2).unwrap_or(area.width);
    Rect::new(area.x, area.y + GRADUATE_ART_HEIGHT + 1, jira_width, 1)
}

fn stage_span(
    label: &'static str,
    active: bool,
    status: ConnectionStatus,
    pending_symbol: &str,
) -> ratatui::text::Span<'static> {
    let text = match status {
        ConnectionStatus::Connected => format!("✓ {label}"),
        ConnectionStatus::Pending => format!("{pending_symbol} {label}"),
        ConnectionStatus::NotConnected if active => format!("● {label}"),
        ConnectionStatus::NotConnected => format!("○ {label}"),
    };
    let style = match status {
        ConnectionStatus::Connected => Palette::success().bold(),
        ConnectionStatus::Pending => Palette::pending().bold(),
        ConnectionStatus::NotConnected if active => Palette::primary().bold(),
        ConnectionStatus::NotConnected => Palette::muted(),
    };
    ratatui::text::Span::styled(text, style)
}

fn render_jira_details(frame: &mut Frame<'_>, area: Rect, model: &OnboardingModel) -> Option<Rect> {
    let spacing = form_spacing(area, 20);
    let [intro, _, hostname, host_help, _, email, _, action, feedback, _] = Layout::vertical([
        Constraint::Length(2),
        Constraint::Length(spacing.section),
        Constraint::Length(3),
        Constraint::Length(1),
        Constraint::Length(spacing.related),
        Constraint::Length(3),
        Constraint::Length(spacing.section),
        Constraint::Length(1),
        Constraint::Length(2),
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
    frame.render_widget(
        Paragraph::new("Your Atlassian workspace address, for example company.atlassian.net").dim(),
        host_help,
    );
    render_field(
        frame,
        email,
        "Atlassian email",
        &terminal_text::escape(model.email.value()),
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
    render_action(
        frame,
        action,
        "Continue to API token",
        model.focus == 2,
        ConnectionStatus::NotConnected,
        model.pending_symbol(),
    );
    render_feedback(frame, feedback, model);
    match model.focus {
        0 => Some(hostname),
        1 => Some(email),
        _ => None,
    }
}

fn render_jira_token(frame: &mut Frame<'_>, area: Rect, model: &OnboardingModel) -> Option<Rect> {
    let fallback_height = if !model.jira_page_can_open || model.warning.is_some() {
        3
    } else {
        0
    };
    let [intro, _, token, _, raw_url, _, status, feedback, _] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(3),
        Constraint::Length(1),
        Constraint::Length(fallback_height),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(2),
        Constraint::Fill(1),
    ])
    .areas(area);
    frame.render_widget(Paragraph::new("Connect Jira").bold(), intro);
    render_field(
        frame,
        token,
        "Atlassian API token",
        model.jira_token.value(),
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
    render_action(
        frame,
        status,
        "Connect Jira",
        model.focus == 1,
        model.jira_status,
        model.pending_symbol(),
    );
    render_feedback(frame, feedback, model);
    (model.focus == 0).then_some(token)
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
    let [intro, _, manifest, _, action, feedback, _] = Layout::vertical([
        Constraint::Length(2),
        Constraint::Length(1),
        Constraint::Length(6),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Fill(1),
    ])
    .areas(area);
    frame.render_widget(
        Paragraph::new(Text::from(vec![
            Line::from("Ready to save").bold(),
            Line::from("Confirm the Jira account Graduate will connect.").dim(),
        ])),
        intro,
    );
    render_connection_endpoint(
        frame,
        constrain_width_left(manifest, MAX_FORM_WIDTH),
        "JIRA",
        vec![
            detail_line("Site", &terminal_text::escape(model.hostname.value())),
            detail_line("Account", &terminal_text::escape(model.email.value())),
            detail_line("Identity", &terminal_text::escape(&model.display_name)),
            edit_line("J", "Edit Jira account"),
        ],
    );
    render_action(
        frame,
        constrain_width_left(action, 26),
        "Save configuration",
        true,
        ConnectionStatus::Connected,
        model.pending_symbol(),
    );
    render_feedback(frame, feedback, model);
}

fn render_connection_endpoint(
    frame: &mut Frame<'_>,
    area: Rect,
    label: &'static str,
    details: Vec<Line<'_>>,
) {
    let title = Line::from(vec![
        ratatui::text::Span::styled(format!(" {label}  "), Palette::primary().bold()),
        ratatui::text::Span::styled("✓ connected ", Palette::success()),
    ]);
    frame.render_widget(
        Paragraph::new(Text::from(details)).block(
            Block::bordered()
                .title(title)
                .border_style(Palette::muted()),
        ),
        area,
    );
}

fn detail_line<'a>(label: &'static str, value: &'a str) -> Line<'a> {
    Line::from(vec![
        ratatui::text::Span::styled(format!("{label:<REVIEW_LABEL_WIDTH$}"), Palette::muted()),
        ratatui::text::Span::styled(value, Palette::text()),
    ])
}

fn edit_line(shortcut: &'static str, label: &'static str) -> Line<'static> {
    Line::from(vec![
        ratatui::text::Span::styled(format!("{shortcut}  "), Palette::primary().bold()),
        ratatui::text::Span::styled(label, Palette::muted()),
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
    let border_style = if invalid {
        Palette::error()
    } else if focused {
        Palette::focus()
    } else {
        Palette::muted()
    };
    let title = if invalid {
        format!(" ✕ {label} (invalid) ")
    } else if focused && retained {
        format!(" › {label} (stored) ")
    } else if focused {
        format!(" › {label} ")
    } else if retained {
        format!(" {label} (stored) ")
    } else {
        format!(" {label} ")
    };
    frame.render_widget(
        Paragraph::new(display.as_str())
            .block(Block::bordered().title(title).border_style(border_style)),
        area,
    );

    if focused && cursor_visible && area.width > 2 && !retained {
        let cursor_offset = cursor.min(usize::from(area.width.saturating_sub(3))) as u16;
        if let Some(cell) = frame
            .buffer_mut()
            .cell_mut(Position::new(area.x + 1 + cursor_offset, area.y + 1))
        {
            cell.set_style(Style::new().reversed());
        }
    }
}

fn render_focus_border(buffer: &mut Buffer, area: Rect, elapsed: Duration) {
    if area.width < 2 || area.height < 2 {
        return;
    }
    let phase = (elapsed.as_secs_f32() * FOCUS_BORDER_CELLS_PER_SECOND) as usize;
    let mut border_index = 0;
    for x in area.x..area.right() {
        tint_focus_border_cell(buffer, Position::new(x, area.y), phase + border_index);
        border_index += 1;
    }
    for y in area.y + 1..area.bottom() - 1 {
        tint_focus_border_cell(
            buffer,
            Position::new(area.right() - 1, y),
            phase + border_index,
        );
        border_index += 1;
    }
    for x in (area.x..area.right()).rev() {
        tint_focus_border_cell(
            buffer,
            Position::new(x, area.bottom() - 1),
            phase + border_index,
        );
        border_index += 1;
    }
    for y in (area.y + 1..area.bottom() - 1).rev() {
        tint_focus_border_cell(buffer, Position::new(area.x, y), phase + border_index);
        border_index += 1;
    }
}

fn tint_focus_border_cell(buffer: &mut Buffer, position: Position, color_index: usize) {
    let Some(cell) = buffer.cell_mut(position) else {
        return;
    };
    if matches!(
        cell.symbol(),
        "─" | "│" | "┌" | "┐" | "└" | "┘" | "━" | "┃" | "┏" | "┓" | "┗" | "┛"
    ) {
        cell.set_fg(focus_border_color(color_index));
    }
}

fn focus_border_color(index: usize) -> Color {
    let stops = [
        (4, PRIMARY_COLOR),
        (2, Color::LightCyan),
        (4, PRIMARY_COLOR),
        (7, Color::Blue),
        (7, Color::Green),
        (7, Color::LightCyan),
    ];
    let cycle_length = stops.iter().map(|(length, _)| length).sum::<usize>();
    let mut offset = index % cycle_length;
    for (length, color) in stops {
        if offset < length {
            return color;
        }
        offset -= length;
    }
    PRIMARY_COLOR
}

fn render_action(
    frame: &mut Frame<'_>,
    area: Rect,
    label: &str,
    focused: bool,
    status: ConnectionStatus,
    pending_symbol: &str,
) {
    let focused_action = focused
        && status != ConnectionStatus::Pending
        && (status == ConnectionStatus::NotConnected || label == "Save configuration");
    let text = match status {
        ConnectionStatus::Pending => format!("{pending_symbol} Verifying {label}…"),
        ConnectionStatus::Connected if label != "Save configuration" => {
            format!("✓ {label} connected")
        }
        _ => format!("{label}  →"),
    };
    let style = if status == ConnectionStatus::Pending {
        Palette::pending().bold()
    } else if focused_action {
        Palette::action_focus().bold()
    } else if status == ConnectionStatus::Connected {
        Palette::success().bold()
    } else {
        Palette::muted()
    };
    let line = if focused_action {
        Line::from(vec![
            ratatui::text::Span::styled("▌", Palette::primary().bold()),
            ratatui::text::Span::styled(format!(" {text} "), style),
            ratatui::text::Span::styled("▐", Palette::primary().bold()),
        ])
    } else {
        Line::styled(format!("  {text}"), style)
    };
    frame.render_widget(Paragraph::new(line), area);
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
    if model.stage == OnboardingScreen::Save {
        let footer = Text::from(vec![
            footer_divider(area.width),
            Line::from(vec![
                ratatui::text::Span::styled(" J ", Palette::primary().bold()),
                ratatui::text::Span::styled("edit Jira  ", Palette::muted()),
                ratatui::text::Span::styled(" Enter ", Palette::primary().bold()),
                ratatui::text::Span::styled("save  ", Palette::muted()),
                ratatui::text::Span::styled(" Esc ", Palette::muted().bold()),
                ratatui::text::Span::styled("edit token", Palette::muted()),
            ]),
        ]);
        frame.render_widget(Paragraph::new(footer), area);
        return;
    }

    let action = match model.stage {
        OnboardingScreen::JiraDetails if model.focus < 2 => "next",
        OnboardingScreen::JiraDetails => "continue to API token",
        OnboardingScreen::JiraToken if model.focus == 0 => "next",
        OnboardingScreen::JiraToken => "connect Jira",
        OnboardingScreen::Save => "save",
    };
    let escape_action = if model.stage == OnboardingScreen::JiraDetails {
        "cancel"
    } else {
        "back"
    };
    let controls = vec![
        ratatui::text::Span::styled(" Tab ", Palette::muted().bold()),
        ratatui::text::Span::styled("next  ", Palette::muted()),
        ratatui::text::Span::styled(" Shift-Tab ", Palette::muted().bold()),
        ratatui::text::Span::styled("previous  ", Palette::muted()),
        ratatui::text::Span::styled(" Enter ", Palette::primary().bold()),
        ratatui::text::Span::styled(format!("{action}  "), Palette::muted()),
        ratatui::text::Span::styled(" Esc ", Palette::muted().bold()),
        ratatui::text::Span::styled(escape_action, Palette::muted()),
    ];
    frame.render_widget(
        Paragraph::new(Text::from(vec![
            footer_divider(area.width),
            Line::from(controls),
        ])),
        area,
    );
}

#[cfg(test)]
mod tests {
    use graduate::jira::JiraValidationError;
    use ratatui::backend::TestBackend;
    use ratatui::style::Modifier;
    use ratatui::Terminal;

    use crate::theme::GRADUATE_ART;

    use super::*;

    const TEST_WIDTH: u16 = 100;
    const TEST_HEIGHT: u16 = 50;

    const fn inactive_animation() -> BufferAnimation {
        BufferAnimation {
            effect: None,
            elapsed: Duration::ZERO,
        }
    }

    fn model(stage: OnboardingScreen) -> OnboardingModel {
        OnboardingModel {
            entrance_animation: inactive_animation(),
            connection_animation: None,
            pending_animation_elapsed: Duration::ZERO,
            focus_border_elapsed: Duration::ZERO,
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
            let _ = render(frame, model);
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
    fn jira_details_match_drags_field_button_and_layout_pattern(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let rendered = render_text(
            TEST_WIDTH,
            TEST_HEIGHT,
            &model(OnboardingScreen::JiraDetails),
        )?;

        assert!(rendered.contains(GRADUATE_ART[0]));
        assert!(rendered.contains("● Jira account ─── ○ Review & save"));
        assert!(rendered.contains("› Jira site"));
        assert!(rendered.contains("Atlassian email"));
        assert!(rendered.contains("Continue to API token  →"));
        let heading = rendered
            .lines()
            .find(|line| line.contains("Connect your Jira account"))
            .ok_or("Jira heading was not rendered")?;
        assert!(heading.starts_with("Connect your Jira account"));
        assert!(rendered.contains("Atlassian account Graduate should use"));
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
        assert!(rendered.contains("Connect Jira  →"));
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
    fn review_keeps_drags_manifest_and_save_action_without_tempo(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let mut model = model(OnboardingScreen::Save);
        model.jira_status = ConnectionStatus::Connected;

        let rendered = render_text(TEST_WIDTH, TEST_HEIGHT, &model)?;

        assert!(rendered.contains("✓ Jira account ─── ● Review & save"));
        assert!(rendered.contains("JIRA"));
        assert!(rendered.contains("✓ connected"));
        assert!(rendered.contains("Save configuration  →"));
        assert!(!rendered.contains("Tempo"));
        Ok(())
    }

    #[test]
    fn focused_button_highlight_wraps_only_the_action_label(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let mut model = model(OnboardingScreen::JiraDetails);
        model.focus = 2;
        let mut terminal = Terminal::new(TestBackend::new(TEST_WIDTH, TEST_HEIGHT))?;
        terminal.draw(|frame| {
            let _ = render(frame, &model);
        })?;

        let highlighted = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .filter(|cell| cell.fg == PRIMARY_COLOR && cell.modifier.contains(Modifier::REVERSED))
            .count();
        assert!(highlighted > 0);
        assert!(highlighted < usize::from(MAX_CONTENT_WIDTH / 2));
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
    fn split_pane_with_78_columns_renders_the_login_form() -> Result<(), Box<dyn std::error::Error>>
    {
        let rendered = render_text(78, 48, &model(OnboardingScreen::JiraDetails))?;

        assert!(!rendered.contains("Terminal too small"));
        assert!(rendered.contains(GRADUATE_ART[0]));
        assert!(rendered.contains("Connect your Jira account"));
        assert!(rendered.contains("Atlassian email"));
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
