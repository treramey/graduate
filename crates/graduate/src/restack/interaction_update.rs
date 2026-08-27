//! Interaction action transitions.

use super::interaction::{
    RestackInteraction, RestackInteractionAction, RestackInteractionEffect, RestackInteractionStage,
};

impl RestackInteraction {
    /// Apply one action and return any requested workflow effect.
    pub fn update(&mut self, action: RestackInteractionAction) -> RestackInteractionEffect {
        match action {
            RestackInteractionAction::Cancel => RestackInteractionEffect::Cancel,
            RestackInteractionAction::MoveUp
                if self.stage == RestackInteractionStage::Selection =>
            {
                self.cursor = self.cursor.saturating_sub(1);
                RestackInteractionEffect::None
            }
            RestackInteractionAction::MoveDown
                if self.stage == RestackInteractionStage::Selection =>
            {
                self.cursor = self
                    .cursor
                    .saturating_add(1)
                    .min(self.snapshot.features.len().saturating_sub(1));
                RestackInteractionEffect::None
            }
            RestackInteractionAction::MovePageUp
                if self.stage == RestackInteractionStage::Selection =>
            {
                self.cursor = self.cursor.saturating_sub(10);
                RestackInteractionEffect::None
            }
            RestackInteractionAction::MovePageDown
                if self.stage == RestackInteractionStage::Selection =>
            {
                self.cursor = self
                    .cursor
                    .saturating_add(10)
                    .min(self.snapshot.features.len().saturating_sub(1));
                RestackInteractionEffect::None
            }
            RestackInteractionAction::MoveFirst
                if self.stage == RestackInteractionStage::Selection =>
            {
                self.cursor = 0;
                RestackInteractionEffect::None
            }
            RestackInteractionAction::MoveLast
                if self.stage == RestackInteractionStage::Selection =>
            {
                self.cursor = self.snapshot.features.len().saturating_sub(1);
                RestackInteractionEffect::None
            }
            RestackInteractionAction::MoveTo(index)
                if self.stage == RestackInteractionStage::Selection =>
            {
                self.cursor = index.min(self.snapshot.features.len().saturating_sub(1));
                RestackInteractionEffect::None
            }
            RestackInteractionAction::MoveUp if self.stage == RestackInteractionStage::Review => {
                self.review_scroll = self.review_scroll.saturating_sub(1);
                RestackInteractionEffect::None
            }
            RestackInteractionAction::MoveDown if self.stage == RestackInteractionStage::Review => {
                self.review_scroll = self.review_scroll.saturating_add(1);
                RestackInteractionEffect::None
            }
            RestackInteractionAction::MovePageUp
                if self.stage == RestackInteractionStage::Review =>
            {
                self.review_scroll = self.review_scroll.saturating_sub(10);
                RestackInteractionEffect::None
            }
            RestackInteractionAction::MovePageDown
                if self.stage == RestackInteractionStage::Review =>
            {
                self.review_scroll = self.review_scroll.saturating_add(10);
                RestackInteractionEffect::None
            }
            RestackInteractionAction::MoveFirst
                if self.stage == RestackInteractionStage::Review =>
            {
                self.review_scroll = 0;
                RestackInteractionEffect::None
            }
            RestackInteractionAction::MoveLast if self.stage == RestackInteractionStage::Review => {
                self.review_scroll = usize::MAX;
                RestackInteractionEffect::None
            }
            RestackInteractionAction::Toggle
                if self.stage == RestackInteractionStage::Selection =>
            {
                self.toggle_current()
            }
            RestackInteractionAction::KeepAll
                if self.stage == RestackInteractionStage::Selection =>
            {
                self.retained.fill(true);
                RestackInteractionEffect::None
            }
            RestackInteractionAction::RemoveAll
                if self.stage == RestackInteractionStage::Selection =>
            {
                self.retained.fill(false);
                RestackInteractionEffect::None
            }
            RestackInteractionAction::ToggleDetails
                if self.stage == RestackInteractionStage::Review =>
            {
                self.review_details = !self.review_details;
                self.review_scroll = 0;
                RestackInteractionEffect::None
            }
            RestackInteractionAction::AcceptInventoryFallback
                if self.stage == RestackInteractionStage::UnsupportedHistory =>
            {
                self.stage = RestackInteractionStage::Selection;
                RestackInteractionEffect::None
            }
            RestackInteractionAction::Continue
                if self.stage == RestackInteractionStage::Selection =>
            {
                match self.selection() {
                    Ok(selection) => RestackInteractionEffect::Preview(selection),
                    Err(error) => RestackInteractionEffect::Rejected(error),
                }
            }
            RestackInteractionAction::Continue if self.stage == RestackInteractionStage::Review => {
                self.stage = RestackInteractionStage::Confirmation;
                RestackInteractionEffect::None
            }
            RestackInteractionAction::Back if self.stage == RestackInteractionStage::Review => {
                self.stage = RestackInteractionStage::Selection;
                RestackInteractionEffect::Revise
            }
            RestackInteractionAction::Back
                if self.stage == RestackInteractionStage::Confirmation =>
            {
                self.stage = RestackInteractionStage::Review;
                RestackInteractionEffect::None
            }
            RestackInteractionAction::Confirm
                if self.stage == RestackInteractionStage::Confirmation =>
            {
                RestackInteractionEffect::Publish
            }
            RestackInteractionAction::MoveUp
            | RestackInteractionAction::MoveDown
            | RestackInteractionAction::MovePageUp
            | RestackInteractionAction::MovePageDown
            | RestackInteractionAction::MoveFirst
            | RestackInteractionAction::MoveLast
            | RestackInteractionAction::MoveTo(_)
            | RestackInteractionAction::Toggle
            | RestackInteractionAction::KeepAll
            | RestackInteractionAction::RemoveAll
            | RestackInteractionAction::ToggleDetails
            | RestackInteractionAction::AcceptInventoryFallback
            | RestackInteractionAction::Continue
            | RestackInteractionAction::Back
            | RestackInteractionAction::Confirm => RestackInteractionEffect::None,
        }
    }
}
