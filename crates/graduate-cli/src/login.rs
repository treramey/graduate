//! Login workflow, Jira verification boundary, and persistence orchestration.

use std::future::Future;
use std::path::Path;
use std::pin::Pin;

use graduate::jira::{JiraCredentials, JiraIdentity};
use graduate::login::{
    CompletedLogin, LoginDefaults, OnboardingError, OnboardingScreen, OnboardingState, SecretInput,
    TokenPage,
};

use crate::browser::{BrowserLauncher, SystemBrowserLauncher};
use crate::cli::LoginArgs;
use crate::config::{Config, ATLASSIAN_EMAIL_ENV, ATLASSIAN_HOST_ENV, ATLASSIAN_TOKEN_ENV};
use crate::error::CliError;
use crate::jira::JiraClient;
use crate::login_tui;

pub(crate) type VerificationFuture<'a> =
    Pin<Box<dyn Future<Output = Result<JiraIdentity, CliError>> + Send + 'a>>;

pub(crate) trait ConnectionVerifier: Send + Sync {
    fn verify<'a>(&'a self, connection: &'a JiraCredentials) -> VerificationFuture<'a>;
}

pub(crate) struct RemoteConnectionVerifier;

impl ConnectionVerifier for RemoteConnectionVerifier {
    fn verify<'a>(&'a self, connection: &'a JiraCredentials) -> VerificationFuture<'a> {
        Box::pin(async move { JiraClient::new(connection)?.verify(connection).await })
    }
}

pub(crate) enum ConnectionOutcome {
    Connected,
    Rejected,
    Invalid(OnboardingError),
}

pub(crate) struct OnboardingWorkflow<'a> {
    verifier: &'a dyn ConnectionVerifier,
    state: OnboardingState,
}

impl<'a> OnboardingWorkflow<'a> {
    pub(crate) fn new(
        existing: &Config,
        verifier: &'a dyn ConnectionVerifier,
        open_browser: bool,
    ) -> Self {
        Self {
            verifier,
            state: OnboardingState::new(
                LoginDefaults {
                    hostname: existing.hostname.clone(),
                    atlassian_user_email: existing.atlassian_user_email.clone(),
                    atlassian_token: existing.atlassian_token.clone(),
                },
                open_browser,
            ),
        }
    }

    pub(crate) fn hostname_default(&self) -> Option<&str> {
        self.state.hostname_default()
    }

    pub(crate) fn email_default(&self) -> Option<&str> {
        self.state.email_default()
    }

    pub(crate) fn can_retain_token(&self) -> bool {
        self.state.can_retain_token()
    }

    pub(crate) const fn screen(&self) -> OnboardingScreen {
        self.state.screen()
    }

    pub(crate) fn continue_from_details(
        &mut self,
        hostname: &str,
        email: &str,
    ) -> Result<OnboardingScreen, OnboardingError> {
        self.state.continue_from_jira_details(hostname, email)
    }

    pub(crate) fn back(&mut self) -> Result<Option<OnboardingScreen>, CliError> {
        self.state.back().map_err(Into::into)
    }

    pub(crate) fn edit_jira_details(&mut self) -> Result<OnboardingScreen, CliError> {
        self.state.edit_jira_details().map_err(Into::into)
    }

    pub(crate) fn token_page(&mut self) -> Result<TokenPage, CliError> {
        self.state.token_page().map_err(Into::into)
    }

    pub(crate) async fn connect(
        &mut self,
        token: SecretInput,
    ) -> Result<ConnectionOutcome, CliError> {
        let credentials = match self.state.prepare_connection(token) {
            Ok(credentials) => credentials,
            Err(error) if error.field().is_some() => {
                return Ok(ConnectionOutcome::Invalid(error));
            }
            Err(error) => return Err(error.into()),
        };
        match self.verifier.verify(&credentials).await {
            Ok(identity) => {
                self.state
                    .accept_verified(credentials, identity)
                    .map_err(CliError::from)?;
                Ok(ConnectionOutcome::Connected)
            }
            Err(error) if error.is_authentication() => Ok(ConnectionOutcome::Rejected),
            Err(error) => Err(error),
        }
    }

    pub(crate) fn verified_login(&self) -> Option<&CompletedLogin> {
        self.state.completed()
    }

    pub(crate) fn finish(self) -> Result<CompletedLogin, CliError> {
        self.state.finish().map_err(Into::into)
    }
}

impl From<OnboardingError> for CliError {
    fn from(error: OnboardingError) -> Self {
        match error {
            OnboardingError::InvalidState => {
                Self::InvalidInput("invalid onboarding workflow state".to_owned())
            }
            OnboardingError::JiraValidation(error) => Self::InvalidInput(error.to_string()),
        }
    }
}

pub(crate) async fn run(args: LoginArgs, path: &Path) -> Result<(), CliError> {
    let verifier = RemoteConnectionVerifier;
    let browser = SystemBrowserLauncher;
    run_with(args, path, &verifier, &browser).await
}

async fn run_with(
    args: LoginArgs,
    path: &Path,
    verifier: &dyn ConnectionVerifier,
    browser: &dyn BrowserLauncher,
) -> Result<(), CliError> {
    if args.from_env {
        return run_from_environment(args, path, verifier).await;
    }
    let existing = Config::load(path)?;
    let workflow = OnboardingWorkflow::new(&existing, verifier, !args.no_open);
    let completed = login_tui::run(workflow, browser).await?;
    save_completed(path, &completed)?;
    println!(
        "Connected {} to Jira at {}. Configuration saved to {}.",
        completed.credentials().email().as_str(),
        completed.credentials().site().as_str(),
        path.display()
    );
    Ok(())
}

async fn run_from_environment(
    args: LoginArgs,
    path: &Path,
    verifier: &dyn ConnectionVerifier,
) -> Result<(), CliError> {
    Config::load(path)?;
    let credentials = JiraCredentials::parse(
        &required_environment(ATLASSIAN_HOST_ENV)?,
        &required_environment(ATLASSIAN_EMAIL_ENV)?,
        &required_environment(ATLASSIAN_TOKEN_ENV)?,
    )
    .map_err(|error| CliError::InvalidInput(error.to_string()))?;
    if args.dry_run && !args.verify {
        println!(
            "Login inputs are valid. Jira verification and configuration changes are planned; nothing was saved."
        );
        return Ok(());
    }
    let identity = verifier.verify(&credentials).await?;
    if args.dry_run {
        println!(
            "Login inputs are valid and Jira verification succeeded. Configuration changes are planned; nothing was saved."
        );
        return Ok(());
    }
    let completed = CompletedLogin::verified(credentials, identity);
    save_completed(path, &completed)?;
    println!(
        "Verified Jira using environment credentials. Configuration saved to {}.",
        path.display()
    );
    Ok(())
}

fn save_completed(path: &Path, completed: &CompletedLogin) -> Result<(), CliError> {
    let mut config = Config::load(path)?;
    config.account_id = Some(completed.identity().account_id().to_owned());
    config.display_name = Some(completed.identity().display_name().to_owned());
    config.atlassian_user_email = Some(completed.credentials().email().as_str().to_owned());
    config.atlassian_token = Some(completed.credentials().token().expose_secret().to_owned());
    config.hostname = Some(completed.credentials().site().as_str().to_owned());
    config.save(path)
}

fn required_environment(name: &str) -> Result<String, CliError> {
    match std::env::var(name) {
        Ok(value)
            if !value.trim().is_empty()
                && !value.chars().any(|character| character.is_control()) =>
        {
            Ok(value)
        }
        Ok(value) if value.chars().any(|character| character.is_control()) => {
            Err(CliError::InvalidInput(format!(
                "{name} contains unsafe control characters for `gd login --from-env`"
            )))
        }
        Err(std::env::VarError::NotUnicode(_)) => Err(CliError::InvalidInput(format!(
            "{name} must contain valid Unicode for `gd login --from-env`"
        ))),
        Err(std::env::VarError::NotPresent) | Ok(_) => Err(CliError::InvalidInput(format!(
            "{name} must be set and non-empty for `gd login --from-env`"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    struct AcceptingVerifier;

    impl ConnectionVerifier for AcceptingVerifier {
        fn verify<'a>(&'a self, _connection: &'a JiraCredentials) -> VerificationFuture<'a> {
            Box::pin(async {
                JiraIdentity::new("account-1", "Person")
                    .map_err(|error| CliError::JiraResponse(error.to_string()))
            })
        }
    }

    struct RejectingVerifier;

    impl ConnectionVerifier for RejectingVerifier {
        fn verify<'a>(&'a self, _connection: &'a JiraCredentials) -> VerificationFuture<'a> {
            Box::pin(async { Err(CliError::Authentication) })
        }
    }

    struct CountingVerifier {
        calls: AtomicUsize,
    }

    impl ConnectionVerifier for CountingVerifier {
        fn verify<'a>(&'a self, _connection: &'a JiraCredentials) -> VerificationFuture<'a> {
            self.calls.fetch_add(1, Ordering::Relaxed);
            Box::pin(async {
                JiraIdentity::new("account-1", "Person")
                    .map_err(|error| CliError::JiraResponse(error.to_string()))
            })
        }
    }

    #[tokio::test]
    async fn workflow_verifies_before_completion() -> Result<(), CliError> {
        let verifier = AcceptingVerifier;
        let mut workflow = OnboardingWorkflow::new(&Config::default(), &verifier, false);
        workflow.continue_from_details("example.atlassian.net", "person@example.com")?;

        let outcome = workflow
            .connect(SecretInput::Replace("secret".to_owned()))
            .await?;

        assert!(matches!(outcome, ConnectionOutcome::Connected));
        assert_eq!(workflow.screen(), OnboardingScreen::Save);
        assert_eq!(workflow.finish()?.identity().account_id(), "account-1");
        Ok(())
    }

    #[tokio::test]
    async fn authentication_rejection_does_not_advance_to_save() -> Result<(), CliError> {
        let verifier = RejectingVerifier;
        let mut workflow = OnboardingWorkflow::new(&Config::default(), &verifier, false);
        workflow.continue_from_details("example.atlassian.net", "person@example.com")?;

        let outcome = workflow
            .connect(SecretInput::Replace("wrong-secret".to_owned()))
            .await?;

        assert!(matches!(outcome, ConnectionOutcome::Rejected));
        assert_eq!(workflow.screen(), OnboardingScreen::JiraToken);
        Ok(())
    }

    #[tokio::test]
    async fn invalid_core_input_never_reaches_the_verifier() -> Result<(), CliError> {
        let verifier = CountingVerifier {
            calls: AtomicUsize::new(0),
        };
        let mut workflow = OnboardingWorkflow::new(&Config::default(), &verifier, false);
        workflow.continue_from_details("example.atlassian.net", "person@example.com")?;

        let outcome = workflow
            .connect(SecretInput::Replace("  ".to_owned()))
            .await?;

        assert!(matches!(outcome, ConnectionOutcome::Invalid(_)));
        assert_eq!(verifier.calls.load(Ordering::Relaxed), 0);
        assert_eq!(workflow.screen(), OnboardingScreen::JiraToken);
        Ok(())
    }
}
