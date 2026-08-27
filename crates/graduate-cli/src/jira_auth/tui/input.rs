//! Key handling, focus movement, and text input for the onboarding model.

use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use graduate::jira_auth::OnboardingScreen;
use tui_input::backend::crossterm::EventHandler;
use tui_input::{Input, InputRequest};

use super::events::Action;
use super::{ConnectionStatus, OnboardingModel};

impl OnboardingModel {
    pub(super) fn handle_event(&mut self, event: Event) -> Action {
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

    pub(super) fn handle_key(&mut self, key: KeyEvent) -> Action {
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

    pub(super) fn focus_count(&self) -> usize {
        match self.stage {
            OnboardingScreen::JiraDetails => 3,
            OnboardingScreen::JiraToken => 2,
            OnboardingScreen::Save => 1,
        }
    }

    pub(super) fn focus_next(&mut self) {
        self.focus = (self.focus + 1) % self.focus_count();
        self.reset_cursor_blink();
    }

    pub(super) fn focus_previous(&mut self) {
        self.focus = (self.focus + self.focus_count() - 1) % self.focus_count();
        self.reset_cursor_blink();
    }

    pub(super) fn focused_input_mut(&mut self) -> Option<&mut Input> {
        match (self.stage, self.focus) {
            (OnboardingScreen::JiraDetails, 0) => Some(&mut self.hostname),
            (OnboardingScreen::JiraDetails, 1) => Some(&mut self.email),
            (OnboardingScreen::JiraToken, 0) => Some(&mut self.jira_token),
            _ => None,
        }
    }

    pub(super) fn insert_into_focused_input(&mut self, value: &str) -> bool {
        let Some(input) = self.focused_input_mut() else {
            return false;
        };
        let mut changed = false;
        for character in value.chars().filter(|character| !character.is_control()) {
            changed |= input.handle(InputRequest::InsertChar(character)).is_some();
        }
        changed
    }

    pub(super) fn input_changed(&mut self) {
        self.reset_cursor_blink();
        self.jira_status = ConnectionStatus::NotConnected;
        self.error = None;
    }

    pub(super) fn activate_or_advance(&mut self) -> Action {
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

    pub(super) fn text_input_focused(&self) -> bool {
        matches!(
            (self.stage, self.focus),
            (OnboardingScreen::JiraDetails, 0 | 1) | (OnboardingScreen::JiraToken, 0)
        )
    }
}
