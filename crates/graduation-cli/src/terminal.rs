//! Shared stderr terminal initialization and restoration.

use std::io;

use crossterm::cursor::Show;
use crossterm::event::{DisableBracketedPaste, EnableBracketedPaste};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;

use crate::error::CliError;

pub(crate) struct StderrTerminal {
    terminal: Terminal<CrosstermBackend<io::Stderr>>,
    lifecycle: LifecycleState,
}

impl StderrTerminal {
    pub(crate) fn new() -> Result<Self, CliError> {
        let mut lifecycle = LifecycleState::default();
        enable_raw_mode()?;
        lifecycle.raw_mode = true;

        let mut stderr = io::stderr();
        if let Err(error) = execute!(stderr, EnterAlternateScreen) {
            let _ = restore_stderr(&mut lifecycle, &mut stderr);
            return Err(CliError::Io(error));
        }
        lifecycle.alternate_screen = true;
        if let Err(error) = execute!(stderr, EnableBracketedPaste) {
            let _ = restore_stderr(&mut lifecycle, &mut stderr);
            return Err(CliError::Io(error));
        }
        lifecycle.bracketed_paste = true;

        let mut terminal = match Terminal::new(CrosstermBackend::new(stderr)) {
            Ok(terminal) => terminal,
            Err(error) => {
                let mut stderr = io::stderr();
                let _ = restore_stderr(&mut lifecycle, &mut stderr);
                return Err(CliError::Io(error));
            }
        };
        lifecycle.cursor_hidden = true;
        if let Err(error) = terminal.hide_cursor() {
            let _ = restore_terminal(&mut lifecycle, &mut terminal);
            return Err(CliError::Io(error));
        }

        Ok(Self {
            terminal,
            lifecycle,
        })
    }

    pub(crate) fn terminal_mut(&mut self) -> &mut Terminal<CrosstermBackend<io::Stderr>> {
        &mut self.terminal
    }

    pub(crate) fn restore(&mut self) -> io::Result<()> {
        restore_terminal(&mut self.lifecycle, &mut self.terminal)
    }
}

impl Drop for StderrTerminal {
    fn drop(&mut self) {
        let _ = self.restore();
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CleanupStep {
    ShowCursor,
    DisableBracketedPaste,
    LeaveAlternateScreen,
    DisableRawMode,
}

#[derive(Debug, Default, PartialEq, Eq)]
struct LifecycleState {
    cursor_hidden: bool,
    bracketed_paste: bool,
    alternate_screen: bool,
    raw_mode: bool,
}

impl LifecycleState {
    fn restore_with(
        &mut self,
        mut restore: impl FnMut(CleanupStep) -> io::Result<()>,
    ) -> io::Result<()> {
        let mut first_error = None;
        for (pending, step) in [
            (
                &mut self.bracketed_paste,
                CleanupStep::DisableBracketedPaste,
            ),
            (
                &mut self.alternate_screen,
                CleanupStep::LeaveAlternateScreen,
            ),
            (&mut self.cursor_hidden, CleanupStep::ShowCursor),
            (&mut self.raw_mode, CleanupStep::DisableRawMode),
        ] {
            if !*pending {
                continue;
            }
            match restore(step) {
                Ok(()) => *pending = false,
                Err(error) => {
                    first_error.get_or_insert(error);
                }
            }
        }
        first_error.map_or(Ok(()), Err)
    }
}

fn restore_terminal(
    lifecycle: &mut LifecycleState,
    terminal: &mut Terminal<CrosstermBackend<io::Stderr>>,
) -> io::Result<()> {
    lifecycle.restore_with(|step| match step {
        CleanupStep::ShowCursor => terminal.show_cursor(),
        CleanupStep::DisableBracketedPaste => {
            execute!(terminal.backend_mut(), DisableBracketedPaste)
        }
        CleanupStep::LeaveAlternateScreen => {
            execute!(terminal.backend_mut(), LeaveAlternateScreen)
        }
        CleanupStep::DisableRawMode => disable_raw_mode(),
    })
}

fn restore_stderr(lifecycle: &mut LifecycleState, stderr: &mut io::Stderr) -> io::Result<()> {
    lifecycle.restore_with(|step| match step {
        CleanupStep::ShowCursor => execute!(stderr, Show),
        CleanupStep::DisableBracketedPaste => execute!(stderr, DisableBracketedPaste),
        CleanupStep::LeaveAlternateScreen => execute!(stderr, LeaveAlternateScreen),
        CleanupStep::DisableRawMode => disable_raw_mode(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restoration_attempts_every_step_and_returns_the_first_error() {
        let mut lifecycle = LifecycleState {
            cursor_hidden: true,
            bracketed_paste: true,
            alternate_screen: true,
            raw_mode: true,
        };
        let mut attempted = Vec::new();

        let error = lifecycle
            .restore_with(|step| {
                attempted.push(step);
                match step {
                    CleanupStep::DisableBracketedPaste => {
                        Err(io::Error::other("alternate screen failed"))
                    }
                    CleanupStep::ShowCursor
                    | CleanupStep::LeaveAlternateScreen
                    | CleanupStep::DisableRawMode => Ok(()),
                }
            })
            .err();

        assert_eq!(
            error.map(|error| error.to_string()).as_deref(),
            Some("alternate screen failed")
        );
        assert_eq!(
            attempted,
            [
                CleanupStep::DisableBracketedPaste,
                CleanupStep::LeaveAlternateScreen,
                CleanupStep::ShowCursor,
                CleanupStep::DisableRawMode,
            ]
        );
        assert_eq!(
            lifecycle,
            LifecycleState {
                cursor_hidden: false,
                bracketed_paste: true,
                alternate_screen: false,
                raw_mode: false,
            }
        );
    }
}
