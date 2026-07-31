//! Versioned ticket-system configuration and atomic persistence.

use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use graduate::jira::JiraCredentials;
use graduate::jira_auth::{CompletedLogin, LoginDefaults};
use serde::{Deserialize, Serialize};

use crate::error::CliError;

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

#[cfg(test)]
mod tests {
    use graduate::jira::{JiraCredentials, JiraIdentity};

    use super::*;

    fn config_with_jira_site(site: &str) -> Config {
        Config::from_legacy(LegacyConfig {
            hostname: Some(site.to_owned()),
            ..LegacyConfig::default()
        })
    }

    #[test]
    fn malformed_config_is_not_silently_discarded() -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("config.json");
        fs::write(&path, "not json")?;

        assert!(matches!(Config::load(&path), Err(CliError::Config(_))));
        Ok(())
    }

    #[test]
    fn save_replaces_existing_config_without_leaving_temporary_files(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("config.json");
        fs::write(&path, "old")?;
        let config = config_with_jira_site("example.atlassian.net");

        config.save(&path)?;

        assert_eq!(
            Config::load(&path)?
                .jira_login_defaults()
                .hostname
                .as_deref(),
            Some("example.atlassian.net")
        );
        assert_eq!(fs::read_dir(directory.path())?.count(), 1);
        Ok(())
    }

    #[test]
    fn save_does_not_touch_the_legacy_fixed_temporary_path(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("config.json");
        let unrelated = path.with_extension("tmp");
        fs::write(&unrelated, "unrelated data")?;

        Config::default().save(&path)?;

        assert_eq!(fs::read_to_string(unrelated)?, "unrelated data");
        Ok(())
    }

    #[test]
    fn destination_with_tmp_extension_is_atomically_replaced(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("config.tmp");
        fs::write(&path, "old")?;
        let config = config_with_jira_site("example.atlassian.net");

        config.save(&path)?;

        assert_eq!(
            Config::load(&path)?
                .jira_login_defaults()
                .hostname
                .as_deref(),
            Some("example.atlassian.net")
        );
        Ok(())
    }

    #[test]
    fn legacy_flat_config_is_loaded_as_the_jira_connection(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("config.json");
        fs::write(
            &path,
            r#"{
  "hostname": "example.atlassian.net",
  "atlassianUserEmail": "person@example.com",
  "atlassianToken": "secret",
  "accountId": "account-1",
  "displayName": "Person"
}"#,
        )?;

        let defaults = Config::load(&path)?.jira_login_defaults();

        assert_eq!(defaults.hostname.as_deref(), Some("example.atlassian.net"));
        assert_eq!(
            defaults.atlassian_user_email.as_deref(),
            Some("person@example.com")
        );
        assert_eq!(defaults.atlassian_token.as_deref(), Some("secret"));
        Ok(())
    }

    #[test]
    fn verified_jira_connection_uses_the_versioned_provider_schema(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("config.json");
        let credentials =
            JiraCredentials::parse("example.atlassian.net", "person@example.com", "secret")?;
        let identity = JiraIdentity::new("account-1", "Person")?;
        let mut config = Config::default();
        config.set_jira_connection(&CompletedLogin::verified(credentials, identity));

        config.save(&path)?;

        let value: serde_json::Value = serde_json::from_str(&fs::read_to_string(&path)?)?;
        assert_eq!(value["version"], 1);
        assert_eq!(value["defaultConnection"], "jira");
        assert_eq!(value["connections"]["jira"]["provider"], "jira");
        assert_eq!(
            value["connections"]["jira"]["site"],
            "example.atlassian.net"
        );
        assert_eq!(value["connections"]["jira"]["accountId"], "account-1");
        let defaults = Config::load(&path)?.jira_login_defaults();
        assert_eq!(defaults.hostname.as_deref(), Some("example.atlassian.net"));
        assert_eq!(
            defaults.atlassian_user_email.as_deref(),
            Some("person@example.com")
        );
        assert_eq!(defaults.atlassian_token.as_deref(), Some("secret"));
        Ok(())
    }

    #[test]
    fn unsupported_config_version_is_rejected() -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("config.json");
        fs::write(&path, r#"{"version":2,"connections":{}}"#)?;

        let result = Config::load(&path);

        assert!(
            matches!(result, Err(CliError::Config(message)) if message.contains("unsupported configuration version 2"))
        );
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn stale_fixed_temporary_symlink_is_not_followed() -> Result<(), Box<dyn std::error::Error>> {
        use std::os::unix::fs::symlink;

        let directory = tempfile::tempdir()?;
        let path = directory.path().join("config.json");
        let victim = directory.path().join("victim");
        fs::write(&victim, "private unrelated data")?;
        symlink(&victim, path.with_extension("tmp"))?;

        Config::default().save(&path)?;

        assert_eq!(fs::read_to_string(victim)?, "private unrelated data");
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn saved_config_is_readable_only_by_the_current_user() -> Result<(), Box<dyn std::error::Error>>
    {
        use std::os::unix::fs::PermissionsExt;

        let directory = tempfile::tempdir()?;
        let path = directory.path().join("config.json");

        Config::default().save(&path)?;

        assert_eq!(fs::metadata(path)?.permissions().mode() & 0o777, 0o600);
        Ok(())
    }
}
