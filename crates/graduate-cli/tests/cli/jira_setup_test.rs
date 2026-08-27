//! Jira setup command tests.

use std::error::Error;

use predicates::prelude::*;

use crate::common::gd_command;

#[test]
fn jira_setup_help_describes_interactive_and_environment_paths() -> Result<(), Box<dyn Error>> {
    gd_command()?
        .args(["auth", "setup", "jira", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "Interactive setup must run in a terminal",
        ))
        .stdout(predicate::str::contains(
            "Use Tab and Shift-Tab to move and Enter to continue",
        ))
        .stdout(predicate::str::contains("Ctrl-C cancels from any stage"))
        .stdout(predicate::str::contains("--from-env"))
        .stdout(predicate::str::contains("--no-open"))
        .stdout(predicate::str::contains("--dry-run"))
        .stdout(predicate::str::contains("--version"));
    Ok(())
}

#[test]
fn login_is_not_retained_as_a_legacy_alias() -> Result<(), Box<dyn Error>> {
    gd_command()?
        .arg("login")
        .assert()
        .code(2)
        .stderr(predicate::str::contains("unrecognized subcommand 'login'"));
    Ok(())
}

#[test]
fn unattended_jira_setup_dry_run_validates_without_saving_or_exposing_token(
) -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("config.json");

    let assertion = gd_command()?
        .args([
            "--config",
            path.to_str().ok_or("non-Unicode test path")?,
            "auth",
            "setup",
            "jira",
            "--from-env",
            "--dry-run",
        ])
        .env("ATLASSIAN_HOST", "https://example.atlassian.net/jira")
        .env("ATLASSIAN_EMAIL", "person@example.com")
        .env("ATLASSIAN_TOKEN", "never-print-this-secret")
        .assert()
        .success()
        .stdout(predicate::str::contains("nothing was saved"));

    let output = assertion.get_output();
    assert!(!String::from_utf8_lossy(&output.stdout).contains("never-print-this-secret"));
    assert!(!String::from_utf8_lossy(&output.stderr).contains("never-print-this-secret"));
    assert!(!path.exists());
    Ok(())
}

#[test]
fn unattended_jira_setup_requires_every_atlassian_value() -> Result<(), Box<dyn Error>> {
    gd_command()?
        .args(["auth", "setup", "jira", "--from-env", "--dry-run"])
        .env("ATLASSIAN_HOST", "example.atlassian.net")
        .env("ATLASSIAN_EMAIL", "person@example.com")
        .assert()
        .code(2)
        .stderr(predicate::str::contains(
            "ATLASSIAN_TOKEN must be set and non-empty",
        ));
    Ok(())
}

#[test]
fn interactive_jira_setup_rejects_redirected_terminals() -> Result<(), Box<dyn Error>> {
    gd_command()?
        .args(["auth", "setup", "jira"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains(
            "interactive setup must run in a terminal",
        ));
    Ok(())
}
