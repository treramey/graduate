//! Page transitions and verified-login application.

use graduate::jira_auth::OnboardingScreen;
use tui_input::Input;

use super::{ConnectionStatus, OnboardingModel};
use crate::jira_auth::OnboardingWorkflow;
use crate::shared::browser::BrowserLauncher;
use crate::shared::error::CliError;
use crate::shared::terminal_text;

pub(super) fn back(
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

pub(super) fn edit_jira(
    model: &mut OnboardingModel,
    workflow: &mut OnboardingWorkflow<'_>,
) -> Result<(), CliError> {
    let screen = workflow.edit_jira_details()?;
    model.jira_status = ConnectionStatus::NotConnected;
    model.jira_token = Input::default();
    model.set_stage(screen);
    Ok(())
}

pub(super) fn present_token_page(
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

pub(super) fn apply_verified_login(
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
