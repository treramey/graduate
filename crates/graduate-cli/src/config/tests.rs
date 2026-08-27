//! Tests.

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
fn save_does_not_touch_the_legacy_fixed_temporary_path() -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("config.json");
    let unrelated = path.with_extension("tmp");
    fs::write(&unrelated, "unrelated data")?;

    Config::default().save(&path)?;

    assert_eq!(fs::read_to_string(unrelated)?, "unrelated data");
    Ok(())
}

#[test]
fn destination_with_tmp_extension_is_atomically_replaced() -> Result<(), Box<dyn std::error::Error>>
{
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
fn legacy_flat_config_is_loaded_as_the_jira_connection() -> Result<(), Box<dyn std::error::Error>> {
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
fn saved_config_is_readable_only_by_the_current_user() -> Result<(), Box<dyn std::error::Error>> {
    use std::os::unix::fs::PermissionsExt;

    let directory = tempfile::tempdir()?;
    let path = directory.path().join("config.json");

    Config::default().save(&path)?;

    assert_eq!(fs::metadata(path)?.permissions().mode() & 0o777, 0o600);
    Ok(())
}
