//! Frame layout, workflow header, footer, and text helpers.

use graduate::restack::{
    RestackInteraction, RestackInteractionAction, RestackInteractionStage, RestackPlan,
    SelectionError,
};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Wrap};
use ratatui::Frame;

use super::confirmation::{confirmation_text, render_confirmation};
use super::review::{render_review, wrapped_text_height};
use super::selection::render_selection;
use super::unsupported_history::render_unsupported_history;
use super::RestackViewState;
use crate::shared::terminal_text::escape;
use crate::shared::theme::{constrain_content_width, Palette};

pub(super) const fn action_allowed_when_undersized(action: RestackInteractionAction) -> bool {
    matches!(
        action,
        RestackInteractionAction::Back | RestackInteractionAction::Cancel
    )
}

pub(super) fn render(
    frame: &mut Frame<'_>,
    interaction: &RestackInteraction,
    plan: Option<&RestackPlan>,
    rejection: Option<&str>,
    view: &mut RestackViewState,
) {
    let content_width = frame.area().width.min(115);
    let minimum_height = match interaction.stage() {
        RestackInteractionStage::Confirmation => confirmation_minimum_height(plan, content_width),
        RestackInteractionStage::UnsupportedHistory | RestackInteractionStage::Selection => 18,
        RestackInteractionStage::Review => 12,
    };
    if frame.area().width < 56 || frame.area().height < minimum_height {
        view.scrollable = false;
        view.undersized = true;
        render_too_small(frame, frame.area(), minimum_height);
        return;
    }
    view.undersized = false;
    let area = constrain_content_width(frame.area());
    let footer_height = footer_height(interaction.stage(), area.width);
    let available_height = area.height.saturating_sub(4 + footer_height);
    let rows = Layout::vertical([
        Constraint::Length(3),
        Constraint::Length(1),
        Constraint::Length(available_height),
        Constraint::Length(footer_height),
        Constraint::Min(0),
    ])
    .split(area);
    render_workflow_header(frame, rows[0], interaction.stage());
    view.scrollable = match interaction.stage() {
        RestackInteractionStage::UnsupportedHistory => {
            render_unsupported_history(frame, rows[2], interaction)
        }
        RestackInteractionStage::Selection => {
            render_selection(frame, rows[2], interaction, rejection, view)
        }
        RestackInteractionStage::Review => render_review(
            frame,
            rows[2],
            plan,
            interaction.review_scroll(),
            interaction.review_details(),
        ),
        RestackInteractionStage::Confirmation => {
            render_confirmation(frame, rows[2], plan);
            false
        }
    };
    render_footer(frame, rows[3], interaction.stage(), view);
}

pub(super) fn confirmation_minimum_height(plan: Option<&RestackPlan>, width: u16) -> u16 {
    let text_height = wrapped_text_height(&confirmation_text(plan), width);
    let text_height = text_height.min(usize::from(u16::MAX)) as u16;
    text_height.saturating_add(6).max(16)
}

fn render_too_small(frame: &mut Frame<'_>, area: Rect, minimum_height: u16) {
    frame.render_widget(
        Paragraph::new(vec![
            Line::styled(
                "Terminal too small for a safe restack review.",
                Palette::warning().bold(),
            ),
            Line::styled(
                format!("Resize to at least 56 columns × {minimum_height} rows."),
                Palette::muted(),
            ),
        ])
        .wrap(Wrap { trim: true }),
        area,
    );
}

fn render_workflow_header(frame: &mut Frame<'_>, area: Rect, stage: RestackInteractionStage) {
    let current = match stage {
        RestackInteractionStage::UnsupportedHistory | RestackInteractionStage::Selection => 0,
        RestackInteractionStage::Review => 1,
        RestackInteractionStage::Confirmation => 2,
    };
    let brand = Line::from(vec![
        Span::styled("GRADUATE", Palette::primary().bold()),
        Span::styled(
            format!("  v{}", env!("CARGO_PKG_VERSION")),
            Palette::muted(),
        ),
    ]);
    frame.render_widget(
        Paragraph::new(brand),
        Rect::new(area.x, area.y, area.width, 1),
    );
    let mut spans = Vec::new();
    for (index, label) in ["Select", "Review", "Publish"].into_iter().enumerate() {
        let style = if index == current {
            Palette::primary().bold()
        } else if index < current {
            Palette::success()
        } else {
            Palette::muted()
        };
        let marker = if index < current {
            "✓".to_owned()
        } else if index == current {
            format!("● {}", index + 1)
        } else {
            format!("○ {}", index + 1)
        };
        spans.push(Span::styled(format!("{marker} {label}"), style));
        if index < 2 {
            spans.push(Span::styled(" › ", Palette::muted()));
        }
    }
    frame.render_widget(
        Paragraph::new(Line::from(spans)),
        Rect::new(area.x, area.y.saturating_add(2), area.width, 1),
    );
}

fn render_footer(
    frame: &mut Frame<'_>,
    area: Rect,
    stage: RestackInteractionStage,
    view: &RestackViewState,
) {
    if stage == RestackInteractionStage::Selection && view.filtering {
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled("Filter: ", Palette::primary().bold()),
                Span::styled(format!("{}▏", escape(&view.filter)), Palette::text()),
            ])),
            area,
        );
        return;
    }
    let mut controls = Vec::new();
    match stage {
        RestackInteractionStage::UnsupportedHistory => {
            controls.extend(control("r", "Rebuild from inventory", true));
            controls.extend(control("Esc", "Cancel", false));
        }
        RestackInteractionStage::Selection => {
            controls.extend(control("Enter", "Review", true));
            controls.extend(control("Space", "Toggle", false));
            controls.extend(control("/", "Filter", false));
            controls.extend(control(
                "?",
                if view.show_shortcuts {
                    "Hide shortcuts"
                } else {
                    "Shortcuts"
                },
                false,
            ));
            controls.extend(control("Esc", "Cancel", false));
        }
        RestackInteractionStage::Review => {
            controls.extend(control("Enter", "Confirm publish", true));
            if view.scrollable {
                controls.extend(control("PgUp/Dn Home/End", "Scroll", false));
            }
            controls.extend(control("d", "Details", false));
            controls.extend(control("Esc", "Revise", false));
            controls.extend(control("q", "Cancel", false));
        }
        RestackInteractionStage::Confirmation => {
            controls.extend(control("Ctrl+Y", "Publish", true));
            controls.extend(control("Esc", "Review details", false));
            controls.extend(control("q", "Abandon plan", false));
        }
    }
    frame.render_widget(
        Paragraph::new(Line::from(controls)).wrap(Wrap { trim: true }),
        area,
    );
}

const fn footer_height(stage: RestackInteractionStage, width: u16) -> u16 {
    match stage {
        RestackInteractionStage::Selection if width < 90 => 2,
        RestackInteractionStage::Selection => 1,
        RestackInteractionStage::Review if width < 72 => 3,
        RestackInteractionStage::Review if width < 96 => 2,
        RestackInteractionStage::UnsupportedHistory
        | RestackInteractionStage::Review
        | RestackInteractionStage::Confirmation => 1,
    }
}

fn control(key: &'static str, label: &'static str, primary: bool) -> Vec<Span<'static>> {
    let key_style = if primary {
        Palette::primary().bold()
    } else {
        Palette::text().bold()
    };
    vec![
        Span::styled(key, key_style),
        Span::styled(format!(" {label}  "), Palette::muted()),
    ]
}

pub(super) fn truncate_text(value: &str, width: usize) -> String {
    if Line::raw(value).width() <= width {
        return value.to_owned();
    }
    let visible = width.saturating_sub(1);
    let mut truncated = String::new();
    for character in value.chars() {
        let mut candidate = truncated.clone();
        candidate.push(character);
        if Line::raw(&candidate).width() > visible {
            break;
        }
        truncated.push(character);
    }
    format!("{truncated}…")
}

pub(super) fn pad_text(value: &str, width: usize) -> String {
    let padding = width.saturating_sub(Line::raw(value).width());
    format!("{value}{}", " ".repeat(padding))
}

pub(super) fn selection_error_message(error: &SelectionError, main: &str) -> String {
    match error {
        SelectionError::Tainted { branch } => format!(
            "Recreate {} from {} and cherry-pick your commits",
            escape(branch),
            escape(main)
        ),
        SelectionError::RetainedDependency { branch, dependents } => format!(
            "Cannot remove {} while retained by {}.",
            escape(branch),
            dependents
                .iter()
                .map(|dependent| escape(dependent))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        SelectionError::Duplicate { branch }
        | SelectionError::Graduated { branch }
        | SelectionError::IndirectOnly { branch }
        | SelectionError::Unknown { branch } => {
            format!("Cannot remove {} from this inventory.", escape(branch))
        }
    }
}

pub(super) fn short_oid(oid: &str) -> String {
    escape(&oid.chars().take(7).collect::<String>())
}
