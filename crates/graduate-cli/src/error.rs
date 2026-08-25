use std::io;
use std::path::PathBuf;

use serde::Serialize;
use thiserror::Error;

pub(crate) const EXIT_FAILURE: u8 = 1;
pub(crate) const EXIT_USAGE: u8 = 2;

/// Stable schema-v1 failure emitted by hidden restack machine workflows.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MachineError {
    kind: &'static str,
    schema_version: u8,
    pub(crate) code: &'static str,
    message: &'static str,
    details: serde_json::Value,
    #[serde(skip)]
    exit_code: u8,
}

impl MachineError {
    pub(crate) fn usage(
        code: &'static str,
        message: &'static str,
        details: serde_json::Value,
    ) -> Self {
        Self::new(code, message, details, EXIT_USAGE)
    }

    pub(crate) fn failure(
        code: &'static str,
        message: &'static str,
        details: serde_json::Value,
    ) -> Self {
        Self::new(code, message, details, EXIT_FAILURE)
    }

    fn new(
        code: &'static str,
        message: &'static str,
        details: serde_json::Value,
        exit_code: u8,
    ) -> Self {
        Self {
            kind: "restackError",
            schema_version: 1,
            code,
            message,
            details,
            exit_code,
        }
    }

    pub(crate) const fn exit_code(&self) -> u8 {
        self.exit_code
    }
}

impl std::fmt::Display for MachineError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match serde_json::to_string(self) {
            Ok(json) => formatter.write_str(&json),
            Err(_) => formatter.write_str(
                r#"{"kind":"restackError","schemaVersion":1,"code":"serialization_failed","message":"could not serialize the structured error","details":{}}"#,
            ),
        }
    }
}

impl std::error::Error for MachineError {}

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
    #[error(transparent)]
    Machine(#[from] MachineError),
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
            Self::Machine(error) => error.exit_code(),
        }
    }

    pub(crate) const fn is_authentication(&self) -> bool {
        matches!(self, Self::Authentication)
    }

    pub(crate) const fn is_machine(&self) -> bool {
        matches!(self, Self::Machine(_))
    }
}
