//! Versioned ticket-system configuration and atomic persistence.

use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use graduate::jira::JiraCredentials;
use graduate::jira_auth::{CompletedLogin, LoginDefaults};
use serde::{Deserialize, Serialize};

use crate::shared::error::CliError;

#[cfg(test)]
mod tests;

pub(crate) const GRADUATE_CONFIG_ENV: &str = "GRADUATE_CONFIG";

pub(crate) const ATLASSIAN_EMAIL_ENV: &str = "ATLASSIAN_EMAIL";

pub(crate) const ATLASSIAN_TOKEN_ENV: &str = "ATLASSIAN_TOKEN";

pub(crate) const ATLASSIAN_HOST_ENV: &str = "ATLASSIAN_HOST";

const CONFIG_VERSION: u32 = 1;

const JIRA_CONNECTION_NAME: &str = "jira";

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Config {
    version: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    default_connection: Option<String>,
    #[serde(default)]
    connections: BTreeMap<String, ConnectionConfig>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            version: CONFIG_VERSION,
            default_connection: None,
            connections: BTreeMap::new(),
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(
    tag = "provider",
    rename_all = "lowercase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
enum ConnectionConfig {
    Jira {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        site: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        email: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        token: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        account_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        display_name: Option<String>,
    },
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct LegacyConfig {
    #[serde(default)]
    account_id: Option<String>,
    #[serde(default)]
    display_name: Option<String>,
    #[serde(default)]
    atlassian_user_email: Option<String>,
    #[serde(default)]
    atlassian_token: Option<String>,
    #[serde(default)]
    hostname: Option<String>,
}

impl Config {
    pub(crate) fn load(path: &Path) -> Result<Self, CliError> {
        match fs::read_to_string(path) {
            Ok(contents) => Self::parse(&contents, path),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(error) => Err(CliError::Config(format!(
                "could not read {}: {error}",
                path.display()
            ))),
        }
    }

    fn parse(contents: &str, path: &Path) -> Result<Self, CliError> {
        let value: serde_json::Value = serde_json::from_str(contents).map_err(|error| {
            CliError::Config(format!("could not parse {}: {error}", path.display()))
        })?;
        if value.get("version").is_some() {
            let config: Self = serde_json::from_value(value).map_err(|error| {
                CliError::Config(format!("could not parse {}: {error}", path.display()))
            })?;
            if config.version != CONFIG_VERSION {
                return Err(CliError::Config(format!(
                    "unsupported configuration version {} in {}; expected {CONFIG_VERSION}",
                    config.version,
                    path.display()
                )));
            }
            return Ok(config);
        }
        let legacy: LegacyConfig = serde_json::from_value(value).map_err(|error| {
            CliError::Config(format!("could not parse {}: {error}", path.display()))
        })?;
        Ok(Self::from_legacy(legacy))
    }

    fn from_legacy(legacy: LegacyConfig) -> Self {
        let has_jira = legacy.hostname.is_some()
            || legacy.atlassian_user_email.is_some()
            || legacy.atlassian_token.is_some()
            || legacy.account_id.is_some()
            || legacy.display_name.is_some();
        let mut config = Self::default();
        if has_jira {
            config.default_connection = Some(JIRA_CONNECTION_NAME.to_owned());
            config.connections.insert(
                JIRA_CONNECTION_NAME.to_owned(),
                ConnectionConfig::Jira {
                    site: legacy.hostname,
                    email: legacy.atlassian_user_email,
                    token: legacy.atlassian_token,
                    account_id: legacy.account_id,
                    display_name: legacy.display_name,
                },
            );
        }
        config
    }

    pub(crate) fn jira_login_defaults(&self) -> LoginDefaults {
        match self.connections.get(JIRA_CONNECTION_NAME) {
            Some(ConnectionConfig::Jira {
                site, email, token, ..
            }) => LoginDefaults {
                hostname: site.clone(),
                atlassian_user_email: email.clone(),
                atlassian_token: token.clone(),
            },
            None => LoginDefaults::default(),
        }
    }

    pub(crate) fn jira_credentials(&self) -> Result<Option<JiraCredentials>, CliError> {
        let Some(ConnectionConfig::Jira {
            site: Some(site),
            email: Some(email),
            token: Some(token),
            ..
        }) = self.connections.get(JIRA_CONNECTION_NAME)
        else {
            return Ok(None);
        };
        JiraCredentials::parse(site, email, token)
            .map(Some)
            .map_err(|error| {
                CliError::Config(format!("stored Jira connection is invalid: {error}"))
            })
    }

    pub(crate) fn set_jira_connection(&mut self, completed: &CompletedLogin) {
        self.default_connection = Some(JIRA_CONNECTION_NAME.to_owned());
        self.connections.insert(
            JIRA_CONNECTION_NAME.to_owned(),
            ConnectionConfig::Jira {
                site: Some(completed.credentials().site().as_str().to_owned()),
                email: Some(completed.credentials().email().as_str().to_owned()),
                token: Some(completed.credentials().token().expose_secret().to_owned()),
                account_id: Some(completed.identity().account_id().to_owned()),
                display_name: Some(completed.identity().display_name().to_owned()),
            },
        );
    }

    pub(crate) fn save(&self, path: &Path) -> Result<(), CliError> {
        let parent = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        fs::create_dir_all(parent)?;
        let mut contents = serde_json::to_vec_pretty(self)?;
        contents.push(b'\n');

        #[cfg(windows)]
        {
            let file = atomicwrites::AtomicFile::new(path, atomicwrites::AllowOverwrite);
            file.write(|temporary| temporary.write_all(&contents))
                .map_err(std::io::Error::from)?;
        }

        #[cfg(not(windows))]
        {
            let mut temporary = tempfile::Builder::new()
                .prefix(".graduate-config-")
                .suffix(".tmp")
                .tempfile_in(parent)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;

                temporary
                    .as_file()
                    .set_permissions(fs::Permissions::from_mode(0o600))?;
                let mode = temporary.as_file().metadata()?.permissions().mode() & 0o777;
                if mode != 0o600 {
                    return Err(CliError::Config(
                        "could not restrict temporary configuration permissions".to_owned(),
                    ));
                }
            }
            temporary.write_all(&contents)?;
            temporary.as_file().sync_all()?;
            temporary
                .persist(path)
                .map_err(|error| CliError::Io(error.error))?;
        }
        Ok(())
    }
}

pub(crate) fn config_path() -> Result<PathBuf, CliError> {
    if let Some(path) = std::env::var_os(GRADUATE_CONFIG_ENV) {
        return Ok(PathBuf::from(path));
    }
    dirs::home_dir()
        .map(|home| home.join(".graduate").join("config.json"))
        .ok_or_else(|| CliError::Config("could not determine the home directory".to_owned()))
}
