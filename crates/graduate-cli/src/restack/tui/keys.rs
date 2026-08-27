//! Key-to-action mapping for the selection and review stages.

use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use graduate::restack::{RestackInteraction, RestackInteractionAction, RestackInteractionStage};

use super::RestackViewState;
use crate::shared::error::CliError;

pub(super) fn next_selection_action(
    interaction: &RestackInteraction,
    view: &mut RestackViewState,
) -> Result<Option<RestackInteractionAction>, CliError> {
    let event = event::read()?;
    let Event::Key(key) = event else {
        return Ok(None);
    };
    if key.kind != KeyEventKind::Press {
        return Ok(None);
    }
    Ok(selection_action_for_key(interaction, view, key))
}

pub(super) fn selection_action_for_key(
    interaction: &RestackInteraction,
    view: &mut RestackViewState,
    key: KeyEvent,
) -> Option<RestackInteractionAction> {
    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
        return Some(RestackInteractionAction::Cancel);
    }
    if interaction.stage() == RestackInteractionStage::UnsupportedHistory {
        return action_for_key(interaction.stage(), key);
    }
    if view.filtering {
        match key.code {
            KeyCode::Esc => {
                view.filtering = false;
                view.filter.clear();
            }
            KeyCode::Enter => view.filtering = false,
            KeyCode::Backspace => {
                let _ = view.filter.pop();
            }
            KeyCode::Char(character)
                if !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                view.filter.push(character);
            }
            _ => return None,
        }
        return first_filtered_index(interaction, &view.filter)
            .map(RestackInteractionAction::MoveTo);
    }
    if key.code == KeyCode::Char('/') {
        view.filtering = true;
        return None;
    }
    if key.code == KeyCode::Char('?') {
        view.show_shortcuts = !view.show_shortcuts;
        return None;
    }
    let visible = filtered_feature_indices(interaction, &view.filter);
    let position = visible
        .iter()
        .position(|index| *index == interaction.cursor())
        .unwrap_or(0);
    let target = match key.code {
        KeyCode::Up | KeyCode::Char('k') => position.saturating_sub(1),
        KeyCode::Down | KeyCode::Char('j') => position.saturating_add(1),
        KeyCode::PageUp => position.saturating_sub(10),
        KeyCode::PageDown => position.saturating_add(10),
        KeyCode::Home => 0,
        KeyCode::End => visible.len().saturating_sub(1),
        _ => return action_for_key(RestackInteractionStage::Selection, key),
    }
    .min(visible.len().saturating_sub(1));
    visible
        .get(target)
        .copied()
        .map(RestackInteractionAction::MoveTo)
}

fn first_filtered_index(interaction: &RestackInteraction, filter: &str) -> Option<usize> {
    filtered_feature_indices(interaction, filter)
        .first()
        .copied()
}

pub(super) fn filtered_feature_indices(
    interaction: &RestackInteraction,
    filter: &str,
) -> Vec<usize> {
    let filter = filter.to_lowercase();
    interaction
        .snapshot()
        .features
        .iter()
        .enumerate()
        .filter(|(_, feature)| {
            filter.is_empty()
                || feature.name.to_lowercase().contains(&filter)
                || carried_by(interaction, &feature.name)
                    .any(|carried| carried.name.to_lowercase().contains(&filter))
        })
        .map(|(index, _)| index)
        .collect()
}

/// Carried branches shown under `carrier`: the first listed carrier owns the row.
pub(super) fn carried_by<'a>(
    interaction: &'a RestackInteraction,
    carrier: &'a str,
) -> impl Iterator<Item = &'a graduate::restack::CarriedFeature> + 'a {
    interaction
        .carried_features()
        .iter()
        .filter(move |carried| {
            carried
                .carriers
                .first()
                .is_some_and(|first| first == carrier)
        })
}

pub(super) fn next_action(
    stage: RestackInteractionStage,
) -> Result<Option<RestackInteractionAction>, CliError> {
    let event = event::read()?;
    let Event::Key(key) = event else {
        return Ok(None);
    };
    if key.kind != KeyEventKind::Press {
        return Ok(None);
    }
    Ok(action_for_key(stage, key))
}

pub(super) fn action_for_key(
    stage: RestackInteractionStage,
    key: KeyEvent,
) -> Option<RestackInteractionAction> {
    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
        return Some(RestackInteractionAction::Cancel);
    }
    match (stage, key.code) {
        (_, KeyCode::Char('q')) => Some(RestackInteractionAction::Cancel),
        (RestackInteractionStage::UnsupportedHistory, KeyCode::Char('r')) => {
            Some(RestackInteractionAction::AcceptInventoryFallback)
        }
        (
            RestackInteractionStage::UnsupportedHistory | RestackInteractionStage::Selection,
            KeyCode::Esc,
        ) => Some(RestackInteractionAction::Cancel),
        (RestackInteractionStage::Selection, KeyCode::Up | KeyCode::Char('k')) => {
            Some(RestackInteractionAction::MoveUp)
        }
        (RestackInteractionStage::Selection, KeyCode::Down | KeyCode::Char('j')) => {
            Some(RestackInteractionAction::MoveDown)
        }
        (RestackInteractionStage::Selection | RestackInteractionStage::Review, KeyCode::PageUp) => {
            Some(RestackInteractionAction::MovePageUp)
        }
        (
            RestackInteractionStage::Selection | RestackInteractionStage::Review,
            KeyCode::PageDown,
        ) => Some(RestackInteractionAction::MovePageDown),
        (RestackInteractionStage::Selection, KeyCode::Home) => {
            Some(RestackInteractionAction::MoveFirst)
        }
        (RestackInteractionStage::Selection, KeyCode::End) => {
            Some(RestackInteractionAction::MoveLast)
        }
        (RestackInteractionStage::Review, KeyCode::Up | KeyCode::Char('k')) => {
            Some(RestackInteractionAction::MoveUp)
        }
        (RestackInteractionStage::Review, KeyCode::Down | KeyCode::Char('j')) => {
            Some(RestackInteractionAction::MoveDown)
        }
        (RestackInteractionStage::Review, KeyCode::Home) => {
            Some(RestackInteractionAction::MoveFirst)
        }
        (RestackInteractionStage::Review, KeyCode::End) => Some(RestackInteractionAction::MoveLast),
        (RestackInteractionStage::Review, KeyCode::Char('d')) => {
            Some(RestackInteractionAction::ToggleDetails)
        }
        (RestackInteractionStage::Selection, KeyCode::Char(' ')) => {
            Some(RestackInteractionAction::Toggle)
        }
        (RestackInteractionStage::Selection, KeyCode::Char('a')) => {
            Some(RestackInteractionAction::KeepAll)
        }
        (RestackInteractionStage::Selection, KeyCode::Char('x')) => {
            Some(RestackInteractionAction::RemoveAll)
        }
        (RestackInteractionStage::Selection | RestackInteractionStage::Review, KeyCode::Enter) => {
            Some(RestackInteractionAction::Continue)
        }
        (RestackInteractionStage::Review | RestackInteractionStage::Confirmation, KeyCode::Esc) => {
            Some(RestackInteractionAction::Back)
        }
        (RestackInteractionStage::Confirmation, KeyCode::Char('y' | 'Y'))
            if key.modifiers.contains(KeyModifiers::CONTROL) =>
        {
            Some(RestackInteractionAction::Confirm)
        }
        _ => None,
    }
}
