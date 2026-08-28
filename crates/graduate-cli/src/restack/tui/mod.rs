//! Terminal selection, review, and conflict handoff for interactive restacks.

use graduate::restack::{
    RestackInteraction, RestackInteractionAction, RestackInteractionEffect, RestackPlan,
    RestackSelection,
};
use ratatui::layout::{Constraint, Layout};
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, ListState, Paragraph};

use crate::shared::error::CliError;
use crate::shared::terminal::StderrTerminal;
use crate::shared::theme::{
    constrain_content_width, render_brand_header, Palette, GRADUATE_ART_HEIGHT,
};
use keys::{next_action, next_selection_action};
use render::{action_allowed_when_undersized, render, selection_error_message};

mod confirmation;
mod handoff;
mod keys;
mod render;
mod review;
mod review_details;
mod selection;
#[cfg(test)]
mod tests;
mod unsupported_history;

pub(crate) use handoff::{write_cancelled, write_conflict, write_preserved, write_success};

pub(crate) enum SelectionDecision {
    Preview(RestackSelection),
    Cancel,
}

pub(crate) enum ReviewDecision {
    Revise,
    Publish,
    Cancel,
}

pub(crate) struct ConflictHandoff<'a> {
    pub(crate) environment: &'a str,
    pub(crate) branch: &'a str,
    pub(crate) unresolved_paths: &'a [String],
    pub(crate) resume_token: &'a str,
    pub(crate) work_area: &'a str,
}

#[derive(Default)]
struct RestackViewState {
    feature_list: ListState,
    scrollable: bool,
    undersized: bool,
    filter: String,
    filtering: bool,
    show_shortcuts: bool,
}

pub(crate) fn draw_loading(terminal: &mut StderrTerminal, message: &str) -> Result<(), CliError> {
    terminal.terminal_mut().draw(|frame| {
        let area = constrain_content_width(frame.area());
        let rows = Layout::vertical([
            Constraint::Length(GRADUATE_ART_HEIGHT + 1),
            Constraint::Min(3),
        ])
        .split(area);
        render_brand_header(frame, rows[0]);
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled("● ", Palette::pending().add_modifier(Modifier::BOLD)),
                Span::raw(message.to_owned()),
            ]))
            .block(Block::default().borders(Borders::TOP).title(" RESTACK ")),
            rows[1],
        );
    })?;
    Ok(())
}

pub(crate) fn choose_features(
    terminal: &mut StderrTerminal,
    interaction: &mut RestackInteraction,
) -> Result<SelectionDecision, CliError> {
    let mut rejection = None;
    let mut view = RestackViewState {
        feature_list: ListState::default().with_selected(Some(interaction.cursor())),
        scrollable: false,
        undersized: false,
        filter: String::new(),
        filtering: false,
        show_shortcuts: false,
    };
    loop {
        terminal
            .terminal_mut()
            .draw(|frame| render(frame, interaction, None, rejection.as_deref(), &mut view))?;
        let Some(action) = next_selection_action(interaction, &mut view)? else {
            continue;
        };
        if view.undersized && !action_allowed_when_undersized(action) {
            continue;
        }
        match interaction.update(action) {
            RestackInteractionEffect::Preview(selection) => {
                return Ok(SelectionDecision::Preview(selection));
            }
            RestackInteractionEffect::Cancel => return Ok(SelectionDecision::Cancel),
            RestackInteractionEffect::Rejected(error) => {
                rejection = Some(selection_error_message(
                    &error,
                    &interaction.snapshot().main,
                ));
            }
            RestackInteractionEffect::None
            | RestackInteractionEffect::Revise
            | RestackInteractionEffect::Publish => rejection = None,
        }
    }
}

pub(crate) fn review_plan(
    terminal: &mut StderrTerminal,
    interaction: &mut RestackInteraction,
    plan: &RestackPlan,
) -> Result<ReviewDecision, CliError> {
    interaction.review_ready();
    let mut view = RestackViewState::default();
    loop {
        terminal
            .terminal_mut()
            .draw(|frame| render(frame, interaction, Some(plan), None, &mut view))?;
        let Some(action) = next_action(interaction.stage())? else {
            continue;
        };
        if view.undersized && !action_allowed_when_undersized(action) {
            continue;
        }
        if matches!(
            action,
            RestackInteractionAction::MoveUp
                | RestackInteractionAction::MoveDown
                | RestackInteractionAction::MovePageUp
                | RestackInteractionAction::MovePageDown
                | RestackInteractionAction::MoveFirst
                | RestackInteractionAction::MoveLast
        ) && !view.scrollable
        {
            continue;
        }
        match interaction.update(action) {
            RestackInteractionEffect::Revise => return Ok(ReviewDecision::Revise),
            RestackInteractionEffect::Publish => return Ok(ReviewDecision::Publish),
            RestackInteractionEffect::Cancel => return Ok(ReviewDecision::Cancel),
            RestackInteractionEffect::None
            | RestackInteractionEffect::Preview(_)
            | RestackInteractionEffect::Rejected(_) => {}
        }
    }
}
