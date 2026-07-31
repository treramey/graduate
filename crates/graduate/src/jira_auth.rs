//! Deterministic Jira authentication and onboarding state transitions.

use thiserror::Error;

use crate::jira::{
    AtlassianToken, JiraAccountDetails, JiraCredentials, JiraField, JiraIdentity,
    JiraValidationError,
};

/// URL where users create or manage Atlassian API tokens.
pub const ATLASSIAN_TOKEN_URL: &str = "https://id.atlassian.com/manage-profile/security/api-tokens";

/// Values loaded from an existing login before onboarding starts.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct LoginDefaults {
    pub hostname: Option<String>,
    pub atlassian_user_email: Option<String>,
    pub atlassian_token: Option<String>,
}

/// Credentials and identity accepted by Jira.
#[derive(Clone, PartialEq, Eq)]
pub struct CompletedLogin {
    credentials: JiraCredentials,
    identity: JiraIdentity,
}

impl CompletedLogin {
    #[must_use]
    pub const fn verified(credentials: JiraCredentials, identity: JiraIdentity) -> Self {
        Self {
            credentials,
            identity,
        }
    }

    #[must_use]
    pub const fn credentials(&self) -> &JiraCredentials {
        &self.credentials
    }

    #[must_use]
    pub const fn identity(&self) -> &JiraIdentity {
        &self.identity
    }
}

/// The visible onboarding screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OnboardingScreen {
    JiraDetails,
    JiraToken,
    Save,
}

/// Secret field submission behavior.
#[derive(Clone, PartialEq, Eq)]
pub enum SecretInput {
    Replace(String),
    Retain,
}

/// Token-management page presentation state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TokenPage {
    pub instruction: &'static str,
    pub url: String,
    pub open_browser: bool,
}

/// Pure onboarding state-machine errors.
#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum OnboardingError {
    #[error("invalid onboarding workflow state")]
    InvalidState,
    #[error(transparent)]
    JiraValidation(#[from] JiraValidationError),
}

impl OnboardingError {
    #[must_use]
    pub const fn field(&self) -> Option<JiraField> {
        match self {
            Self::InvalidState => None,
            Self::JiraValidation(error) => Some(error.field()),
        }
    }
}

/// I/O-independent login finite-state workflow.
#[derive(Clone, PartialEq, Eq)]
pub struct OnboardingState {
    open_browser: bool,
    screen: OnboardingScreen,
    hostname_default: Option<String>,
    email_default: Option<String>,
    details: Option<JiraAccountDetails>,
    jira_token: Option<AtlassianToken>,
    token_page_presented: bool,
    completed: Option<CompletedLogin>,
}

impl OnboardingState {
    #[must_use]
    pub fn new(defaults: LoginDefaults, open_browser: bool) -> Self {
        Self {
            open_browser,
            screen: OnboardingScreen::JiraDetails,
            hostname_default: defaults.hostname,
            email_default: defaults.atlassian_user_email,
            details: None,
            jira_token: defaults
                .atlassian_token
                .and_then(|value| AtlassianToken::parse(&value).ok()),
            token_page_presented: false,
            completed: None,
        }
    }

    #[must_use]
    pub fn hostname_default(&self) -> Option<&str> {
        self.hostname_default.as_deref()
    }

    #[must_use]
    pub fn email_default(&self) -> Option<&str> {
        self.email_default.as_deref()
    }

    #[must_use]
    pub fn can_retain_token(&self) -> bool {
        self.jira_token.is_some()
    }

    #[must_use]
    pub const fn screen(&self) -> OnboardingScreen {
        self.screen
    }

    pub fn continue_from_jira_details(
        &mut self,
        hostname: &str,
        email: &str,
    ) -> Result<OnboardingScreen, OnboardingError> {
        self.require_screen(OnboardingScreen::JiraDetails)?;
        let details = JiraAccountDetails::parse(hostname, email)?;
        self.hostname_default = Some(details.site().as_str().to_owned());
        self.email_default = Some(details.email().as_str().to_owned());
        self.details = Some(details);
        self.screen = OnboardingScreen::JiraToken;
        Ok(self.screen)
    }

    pub fn back(&mut self) -> Result<Option<OnboardingScreen>, OnboardingError> {
        self.screen = match self.screen {
            OnboardingScreen::JiraDetails => return Ok(None),
            OnboardingScreen::JiraToken => OnboardingScreen::JiraDetails,
            OnboardingScreen::Save => OnboardingScreen::JiraToken,
        };
        Ok(Some(self.screen))
    }

    /// Return directly from review to Jira details and invalidate the review.
    pub fn edit_jira_details(&mut self) -> Result<OnboardingScreen, OnboardingError> {
        self.require_screen(OnboardingScreen::Save)?;
        self.completed = None;
        self.screen = OnboardingScreen::JiraDetails;
        Ok(self.screen)
    }

    pub fn token_page(&mut self) -> Result<TokenPage, OnboardingError> {
        self.require_screen(OnboardingScreen::JiraToken)?;
        let page = TokenPage {
            instruction: "Create or manage your Atlassian API token:",
            url: ATLASSIAN_TOKEN_URL.to_owned(),
            open_browser: self.open_browser && !self.token_page_presented,
        };
        self.token_page_presented = true;
        Ok(page)
    }

    pub fn prepare_connection(
        &mut self,
        token: SecretInput,
    ) -> Result<JiraCredentials, OnboardingError> {
        self.require_screen(OnboardingScreen::JiraToken)?;
        let atlassian_token = resolve_secret(token, self.jira_token.as_ref())?;
        let details = self.details.clone().ok_or(OnboardingError::InvalidState)?;
        Ok(JiraCredentials::from_parts(details, atlassian_token))
    }

    pub fn accept_verified(
        &mut self,
        credentials: JiraCredentials,
        identity: JiraIdentity,
    ) -> Result<OnboardingScreen, OnboardingError> {
        self.require_screen(OnboardingScreen::JiraToken)?;
        self.jira_token = Some(credentials.token().clone());
        self.completed = Some(CompletedLogin::verified(credentials, identity));
        self.screen = OnboardingScreen::Save;
        Ok(self.screen)
    }

    pub fn finish(self) -> Result<CompletedLogin, OnboardingError> {
        self.require_screen(OnboardingScreen::Save)?;
        self.completed.ok_or(OnboardingError::InvalidState)
    }

    /// Borrow the verified login while the review screen is active.
    #[must_use]
    pub fn completed(&self) -> Option<&CompletedLogin> {
        self.completed.as_ref()
    }

    fn require_screen(&self, expected: OnboardingScreen) -> Result<(), OnboardingError> {
        if self.screen == expected {
            Ok(())
        } else {
            Err(OnboardingError::InvalidState)
        }
    }
}

fn resolve_secret(
    input: SecretInput,
    existing: Option<&AtlassianToken>,
) -> Result<AtlassianToken, OnboardingError> {
    match input {
        SecretInput::Replace(value) => AtlassianToken::parse(&value).map_err(Into::into),
        SecretInput::Retain => existing
            .cloned()
            .ok_or(JiraValidationError::AtlassianTokenRequired)
            .map_err(Into::into),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workflow_starts_with_non_secret_defaults() {
        let state = OnboardingState::new(
            LoginDefaults {
                hostname: Some("example.atlassian.net".to_owned()),
                atlassian_user_email: Some("person@example.com".to_owned()),
                atlassian_token: Some("secret".to_owned()),
            },
            false,
        );

        assert_eq!(state.screen(), OnboardingScreen::JiraDetails);
        assert_eq!(state.hostname_default(), Some("example.atlassian.net"));
        assert_eq!(state.email_default(), Some("person@example.com"));
        assert!(state.can_retain_token());
    }

    #[test]
    fn token_page_opens_browser_only_once() -> Result<(), OnboardingError> {
        let mut state = OnboardingState::new(LoginDefaults::default(), true);
        state.continue_from_jira_details("example.atlassian.net", "person@example.com")?;

        assert!(state.token_page()?.open_browser);
        assert!(!state.token_page()?.open_browser);
        Ok(())
    }

    #[test]
    fn verified_credentials_can_be_finished() -> Result<(), OnboardingError> {
        let mut state = OnboardingState::new(LoginDefaults::default(), false);
        state.continue_from_jira_details("example.atlassian.net", " person@example.com ")?;
        let credentials = state.prepare_connection(SecretInput::Replace(" secret ".to_owned()))?;
        state.accept_verified(credentials, JiraIdentity::new("account-1", "Person")?)?;

        let completed = state.finish()?;
        assert_eq!(completed.identity().account_id(), "account-1");
        assert_eq!(completed.credentials().token().expose_secret(), "secret");
        Ok(())
    }

    #[test]
    fn retaining_requires_an_existing_token() -> Result<(), OnboardingError> {
        let mut state = OnboardingState::new(LoginDefaults::default(), false);
        state.continue_from_jira_details("example.atlassian.net", "person@example.com")?;

        let result = state.prepare_connection(SecretInput::Retain);

        assert!(matches!(
            result,
            Err(OnboardingError::JiraValidation(
                JiraValidationError::AtlassianTokenRequired
            ))
        ));
        Ok(())
    }

    #[test]
    fn invalid_details_do_not_advance_the_workflow() {
        let mut state = OnboardingState::new(LoginDefaults::default(), false);

        let result =
            state.continue_from_jira_details("http://example.atlassian.net", "person@example.com");

        assert!(matches!(
            result,
            Err(OnboardingError::JiraValidation(
                JiraValidationError::SiteMustUseHttps
            ))
        ));
        assert_eq!(state.screen(), OnboardingScreen::JiraDetails);
    }

    #[test]
    fn editing_from_review_is_one_core_transition() -> Result<(), OnboardingError> {
        let mut state = OnboardingState::new(LoginDefaults::default(), false);
        state.continue_from_jira_details("example.atlassian.net", "person@example.com")?;
        let credentials = state.prepare_connection(SecretInput::Replace("secret".to_owned()))?;
        state.accept_verified(credentials, JiraIdentity::new("account-1", "Person")?)?;

        assert_eq!(state.edit_jira_details()?, OnboardingScreen::JiraDetails);
        assert!(state.completed().is_none());
        assert!(state.can_retain_token());
        Ok(())
    }
}
