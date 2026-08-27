//! Actions, runtime events, and terminal predicates.

use crossterm::event::{Event, KeyCode, KeyEventKind, KeyModifiers};

use super::{MIN_TERMINAL_HEIGHT, MIN_TERMINAL_WIDTH, REDUCED_MOTION_ENV};
use crate::error::CliError;
use crate::terminal::StderrTerminal;

pub(super) enum Action {
    None,
    Continue,
    ConnectJira,
    Save,
    EditJira,
    Back,
    Cancel,
}

pub(super) enum RuntimeEvent {
    Terminal(Event),
    Tick,
}

pub(super) fn reduced_motion_requested() -> bool {
    let value = std::env::var(REDUCED_MOTION_ENV).ok();
    reduced_motion_value(value.as_deref())
}

pub(super) fn reduced_motion_value(value: Option<&str>) -> bool {
    value.is_some_and(|value| {
        let value = value.trim();
        value == "1"
            || value.eq_ignore_ascii_case("true")
            || value.eq_ignore_ascii_case("yes")
            || value.eq_ignore_ascii_case("on")
    })
}

pub(super) fn is_cancel_event(event: &Event) -> bool {
    matches!(
        event,
        Event::Key(key)
            if matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat)
                && key.modifiers.contains(KeyModifiers::CONTROL)
                && key.code == KeyCode::Char('c')
    )
}

pub(super) fn is_back_event(event: &Event) -> bool {
    matches!(
        event,
        Event::Key(key)
            if matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat)
                && key.code == KeyCode::Esc
    )
}

pub(super) fn terminal_is_undersized(terminal: &mut StderrTerminal) -> Result<bool, CliError> {
    let size = terminal.terminal_mut().size()?;
    Ok(size_is_undersized(size.width, size.height))
}

pub(super) const fn size_is_undersized(width: u16, height: u16) -> bool {
    width < MIN_TERMINAL_WIDTH || height < MIN_TERMINAL_HEIGHT
}
