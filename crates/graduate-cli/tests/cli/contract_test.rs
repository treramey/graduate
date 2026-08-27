//! Help, version, describe, and argument contract tests.

use std::error::Error;

use predicates::prelude::*;

use crate::common::{gd_command, isolate_gd_storage};

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
            "Inspect and rebuild Git workflow environments from the terminal\n\n{usage}"
        )))
        .stdout(predicate::str::contains(usage))
        .stdout(predicate::str::contains("tui").not())
        .stdout(predicate::str::contains(
            "auth             Configure authentication for a ticket system",
        ))
        .stdout(predicate::str::contains(
            "restack          Review and safely publish an isolated environment reconstruction",
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
fn describe_restack_emits_the_runtime_json_contract() -> Result<(), Box<dyn Error>> {
    let output = gd_command()?
        .args(["describe", "restack", "--json"])
        .output()?;

    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let description: serde_json::Value = serde_json::from_slice(&output.stdout)?;
    assert_eq!(description["kind"], "commandDescription");
    assert_eq!(description["schemaVersion"], 1);
    assert_eq!(description["command"], "gd restack");
    assert_eq!(
        description["payloadSchemas"]["restackPreviewParams"]["required"],
        serde_json::json!(["removeBranches"])
    );
    assert_eq!(
        description["payloadSchemas"]["restackApplyParams"]["required"],
        serde_json::json!(["removeBranches", "planDigest"])
    );
    assert_eq!(description["security"]["operatorTrusted"], false);
    assert_eq!(description["security"]["repositoryContentTrusted"], false);
    assert_eq!(description["results"]["stderr"]["kind"], "restackError");
    Ok(())
}

#[test]
fn schema_restack_emits_the_runtime_json_contract_without_a_format_flag(
) -> Result<(), Box<dyn Error>> {
    let output = gd_command()?.args(["schema", "restack"]).output()?;

    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let description: serde_json::Value = serde_json::from_slice(&output.stdout)?;
    assert_eq!(description["kind"], "commandDescription");
    assert_eq!(description["command"], "gd restack");
    let dry_run = description["arguments"]
        .as_array()
        .ok_or("arguments")?
        .iter()
        .find(|argument| argument["name"] == "dryRun")
        .ok_or("dry-run argument")?;
    assert_eq!(
        dry_run["defaultSelection"]["removeBranches"],
        serde_json::json!([])
    );
    assert_eq!(
        dry_run["conflictsWith"],
        serde_json::json!(["apply", "resume", "abort"])
    );
    Ok(())
}

#[test]
fn describe_restack_requires_an_explicit_machine_format() -> Result<(), Box<dyn Error>> {
    gd_command()?
        .args(["describe", "restack"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("--json"));
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
fn restack_help_exposes_only_the_guarded_contract() -> Result<(), Box<dyn Error>> {
    gd_command()?
        .args(["restack", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("<ENVIRONMENT>"))
        .stdout(predicate::str::contains("--main <BRANCH>"))
        .stdout(predicate::str::contains("--remote <REMOTE>"))
        .stdout(predicate::str::contains("--params <JSON>"))
        .stdout(predicate::str::contains("--dry-run"))
        .stdout(predicate::str::contains("--apply"))
        .stdout(predicate::str::contains("--resume <TOKEN>"))
        .stdout(predicate::str::contains("--abort"))
        .stdout(predicate::str::contains("--no-fetch").not())
        .stdout(predicate::str::contains("--format").not())
        .stdout(predicate::str::contains("--output").not());
    Ok(())
}

#[test]
fn restack_dry_run_conflicts_with_apply() -> Result<(), Box<dyn Error>> {
    let output = gd_command()?
        .args([
            "restack",
            "qa",
            "--dry-run",
            "--apply",
            "--params",
            r#"{"removeBranches":[],"planDigest":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}"#,
        ])
        .output()?;

    assert_eq!(output.status.code(), Some(2));
    let error: serde_json::Value = serde_json::from_slice(&output.stderr)?;
    assert_eq!(error["code"], "invalid_usage");
    Ok(())
}

#[test]
fn restack_rejects_percent_encoded_ref_components_before_git_io() -> Result<(), Box<dyn Error>> {
    for arguments in [
        vec![
            "restack",
            "%2e%2e/qa",
            "--params",
            r#"{"removeBranches":[]}"#,
        ],
        vec![
            "restack",
            "qa",
            "--main",
            "%252e%252e/main",
            "--params",
            r#"{"removeBranches":[]}"#,
        ],
        vec![
            "restack",
            "qa",
            "--remote",
            "origin%2fother",
            "--params",
            r#"{"removeBranches":[]}"#,
        ],
    ] {
        let output = gd_command()?.args(arguments).output()?;
        assert_eq!(output.status.code(), Some(2));
        let error: serde_json::Value = serde_json::from_slice(&output.stderr)?;
        assert_eq!(error["code"], "invalid_ref");
    }

    let cache = tempfile::tempdir()?;
    let mut command = gd_command()?;
    isolate_gd_storage(&mut command, cache.path());
    let output = command
        .args([
            "restack",
            "qa",
            "--params",
            r#"{"removeBranches":["feature/%2e%2e"]}"#,
        ])
        .output()?;
    assert_eq!(output.status.code(), Some(2));
    let error: serde_json::Value = serde_json::from_slice(&output.stderr)?;
    assert_eq!(error["code"], "invalid_params");
    Ok(())
}
