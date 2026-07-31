//! Jira configuration loading, environment overrides, and atomic persistence.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::CliError;

pub(crate) const GRAD_CONFIG_ENV: &str = "GRAD_CONFIG";
pub(crate) const ATLASSIAN_EMAIL_ENV: &str = "ATLASSIAN_EMAIL";
pub(crate) const ATLASSIAN_TOKEN_ENV: &str = "ATLASSIAN_TOKEN";
pub(crate) const ATLASSIAN_HOST_ENV: &str = "ATLASSIAN_HOST";

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Config {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) account_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) display_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) atlassian_user_email: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) atlassian_token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) hostname: Option<String>,
}

impl Config {
    pub(crate) fn load(path: &Path) -> Result<Self, CliError> {
        match fs::read_to_string(path) {
            Ok(contents) => serde_json::from_str(&contents).map_err(|error| {
                CliError::Config(format!("could not parse {}: {error}", path.display()))
            }),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(error) => Err(CliError::Config(format!(
                "could not read {}: {error}",
                path.display()
            ))),
        }
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
                .prefix(".grad-config-")
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
    if let Some(path) = std::env::var_os(GRAD_CONFIG_ENV) {
        return Ok(PathBuf::from(path));
    }
    dirs::home_dir()
        .map(|home| home.join(".grad").join("config.json"))
        .ok_or_else(|| CliError::Config("could not determine the home directory".to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let config = Config {
            hostname: Some("example.atlassian.net".to_owned()),
            ..Config::default()
        };

        config.save(&path)?;

        assert_eq!(
            Config::load(&path)?.hostname.as_deref(),
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
        let config = Config {
            hostname: Some("example.atlassian.net".to_owned()),
            ..Config::default()
        };

        config.save(&path)?;

        assert_eq!(
            Config::load(&path)?.hostname.as_deref(),
            Some("example.atlassian.net")
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
