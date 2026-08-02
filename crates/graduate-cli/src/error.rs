use std::io;
use std::path::PathBuf;

use thiserror::Error;

pub(crate) const EXIT_FAILURE: u8 = 1;
pub(crate) const EXIT_USAGE: u8 = 2;

/// Process-facing Graduate failures.
#[derive(Debug, Error)]
pub(crate) enum CliError {
    #[error("{0}")]
    InvalidInput(String),
    #[error("configuration error: {0}")]
    Config(String),
    #[error(
        "Jira rejected the Atlassian email or API token; run `gd auth setup jira` to update them"
    )]
    Authentication,
    #[error("Jira verification failed with HTTP status {0}")]
    JiraStatus(u16),
    #[error("Jira returned an invalid response: {0}")]
    JiraResponse(String),
    #[error("HTTP request failed: {0}")]
    Http(#[from] reqwest::Error),
    #[error("invalid URL: {0}")]
    Url(#[from] url::ParseError),
    #[error("invalid JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("YAML serialization failed: {0}")]
    Yaml(#[from] serde_yaml::Error),
    #[error("terminal I/O failed: {0}")]
    Io(#[from] io::Error),
    #[error("{0}")]
    Git(String),
    #[error("promotion report was cancelled")]
    ReportCancelled,
    #[error("interactive setup was cancelled; configuration was not changed")]
    LoginCancelled,
    #[error(
        "generated file already exists: {path}; pass --force to replace it",
        path = .0.display()
    )]
    GeneratedFileExists(PathBuf),
}

impl CliError {
    pub(crate) const fn exit_code(&self) -> u8 {
        match self {
            Self::InvalidInput(_)
            | Self::LoginCancelled
            | Self::ReportCancelled
            | Self::GeneratedFileExists(_) => EXIT_USAGE,
            Self::Config(_)
            | Self::Authentication
            | Self::JiraStatus(_)
            | Self::JiraResponse(_)
            | Self::Http(_)
            | Self::Url(_)
            | Self::Json(_)
            | Self::Yaml(_)
            | Self::Io(_)
            | Self::Git(_) => EXIT_FAILURE,
        }
    }

    pub(crate) const fn is_authentication(&self) -> bool {
        matches!(self, Self::Authentication)
    }
}
