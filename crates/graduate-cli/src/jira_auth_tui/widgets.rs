//! Field, button, feedback, and footer widgets.

use graduate::jira_auth::OnboardingScreen;
use ratatui::buffer::Buffer;
use ratatui::layout::{Position, Rect};
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::widgets::{Block, BorderType, Paragraph, Widget, Wrap};
use ratatui::Frame;

use super::{ConnectionStatus, OnboardingModel};
use crate::terminal_text;
use crate::theme::{Palette, MUTED_COLOR};

#[derive(Default)]
pub(super) struct FieldPresentation {
    pub(super) focused: bool,
    pub(super) cursor_visible: bool,
    pub(super) cursor: usize,
    pub(super) masked: bool,
    pub(super) can_retain_secret: bool,
    pub(super) invalid: bool,
}

pub(super) fn render_field(
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

pub(super) struct SetupButton<'a> {
    label: &'a str,
    focused: bool,
    status: ConnectionStatus,
    pending_symbol: &'a str,
}

impl<'a> SetupButton<'a> {
    pub(super) const fn new(
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

pub(super) fn render_feedback(frame: &mut Frame<'_>, area: Rect, model: &OnboardingModel) {
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

pub(super) fn render_footer(frame: &mut Frame<'_>, area: Rect, model: &OnboardingModel) {
    let controls = match model.stage {
        OnboardingScreen::JiraDetails => "↑↓/tab navigate • enter select • esc cancel",
        OnboardingScreen::JiraToken => "↑↓/tab navigate • enter select • esc back",
        OnboardingScreen::Save => "enter save • j change account • esc change token",
    };
    frame.render_widget(Paragraph::new(controls).style(Palette::muted()), area);
}
