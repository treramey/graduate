//! Spinner and cursor animation timing.

use std::time::Duration;

use super::{
    ConnectionStatus, OnboardingModel, CURSOR_BLINK_HALF_PERIOD, PLAYFUL_FRAME_RATE, SPINNER_FRAMES,
};

impl OnboardingModel {
    pub(super) fn animations_active(&self) -> bool {
        (!self.reduced_motion && self.jira_status == ConnectionStatus::Pending)
            || (!self.reduced_motion && self.text_input_focused())
    }

    pub(super) fn advance_animations(&mut self, elapsed: Duration) {
        if self.jira_status == ConnectionStatus::Pending {
            self.pending_animation_elapsed += elapsed;
        }
        if !self.reduced_motion && self.text_input_focused() {
            self.cursor_blink_elapsed += elapsed;
        }
    }

    pub(super) fn cursor_visible(&self) -> bool {
        self.reduced_motion
            || (self.cursor_blink_elapsed.as_millis() / CURSOR_BLINK_HALF_PERIOD.as_millis())
                .is_multiple_of(2)
    }

    pub(super) fn reset_cursor_blink(&mut self) {
        self.cursor_blink_elapsed = Duration::ZERO;
    }

    pub(super) fn pending_symbol(&self) -> &'static str {
        if self.reduced_motion || self.jira_status != ConnectionStatus::Pending {
            return "…";
        }
        let frame = (self.pending_animation_elapsed.as_millis() / PLAYFUL_FRAME_RATE.as_millis())
            % SPINNER_FRAMES.len() as u128;
        SPINNER_FRAMES[frame as usize]
    }
}
