use std::error::Error;

use assert_cmd::Command;
use predicates::prelude::*;

fn gd_command() -> Result<Command, Box<dyn Error>> {
    let mut command = Command::cargo_bin("gd")?;
    for variable in [
        "GRADUATE_CONFIG",
        "ATLASSIAN_HOST",
        "ATLASSIAN_EMAIL",
        "ATLASSIAN_TOKEN",
        "GIT_PAT",
    ] {
        command.env_remove(variable);
    }
    Ok(command)
}

#[test]
fn help_uses_the_product_focused_command_style() -> Result<(), Box<dyn Error>> {
    let usage = format!(
        "Usage: gd{} [OPTIONS] <COMMAND>",
        std::env::consts::EXE_SUFFIX
    );
    gd_command()?
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::starts_with(format!(
            "Inspect Jira Cloud from the terminal\n\n{usage}"
        )))
        .stdout(predicate::str::contains(usage))
        .stdout(predicate::str::contains("tui").not())
        .stdout(predicate::str::contains(
            "auth             Configure authentication for a ticket system",
        ))
        .stdout(predicate::str::contains("login").not());
    Ok(())
}

#[test]
fn version_uses_the_workspace_version() -> Result<(), Box<dyn Error>> {
    gd_command()?
        .arg("--version")
        .assert()
        .success()
        .stdout(predicate::str::contains(env!("CARGO_PKG_VERSION")));
    Ok(())
}

#[test]
fn bare_invocation_requires_a_command() -> Result<(), Box<dyn Error>> {
    let usage = format!(
        "Usage: gd{} [OPTIONS] <COMMAND>",
        std::env::consts::EXE_SUFFIX
    );
    gd_command()?
        .assert()
        .code(2)
        .stderr(predicate::str::contains(usage));
    Ok(())
}

#[test]
fn help_describes_skill_generation() -> Result<(), Box<dyn Error>> {
    gd_command()?
        .args(["generate-skills", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("--output-dir"))
        .stdout(predicate::str::contains("--force"));
    Ok(())
}

#[test]
fn diff_help_describes_promotion_and_automation_options() -> Result<(), Box<dyn Error>> {
    gd_command()?
        .args(["diff", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("<ENVIRONMENT>"))
        .stdout(predicate::str::contains("--main <BRANCH>"))
        .stdout(predicate::str::contains("--report <REPORT>"))
        .stdout(predicate::str::contains("--params <JSON>"))
        .stdout(predicate::str::contains("--format <FORMAT>"))
        .stdout(predicate::str::contains("-o, --output <PATH>"))
        .stdout(predicate::str::contains("--csv").not())
        .stdout(predicate::str::contains("--pat").not())
        .stdout(predicate::str::contains("--unattended").not())
        .stdout(predicate::str::contains("--no-fetch"));
    Ok(())
}

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
