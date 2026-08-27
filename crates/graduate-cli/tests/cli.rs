use std::error::Error;
use std::path::{Path, PathBuf};
use std::process::Command as ProcessCommand;

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
        "GIT_DIR",
        "GIT_WORK_TREE",
        "GIT_INDEX_FILE",
        "GIT_OBJECT_DIRECTORY",
        "GIT_ALTERNATE_OBJECT_DIRECTORIES",
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
    let output = gd_command()?
        .args([
            "restack",
            "qa",
            "--params",
            r#"{"removeBranches":["feature/%2e%2e"]}"#,
        ])
        .env("XDG_CACHE_HOME", cache.path())
        .output()?;
    assert_eq!(output.status.code(), Some(2));
    let error: serde_json::Value = serde_json::from_slice(&output.stderr)?;
    assert_eq!(error["code"], "invalid_params");
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

#[test]
fn restack_preview_is_isolated_and_emits_canonical_machine_json() -> Result<(), Box<dyn Error>> {
    let fixture = RestackFixture::new()?;
    let source_head = fixture.git_text(&fixture.source, &["rev-parse", "HEAD"])?;
    let local_environment = fixture.git_text(&fixture.source, &["rev-parse", "refs/heads/qa"])?;
    let remote_environment = fixture.git_text(&fixture.remote, &["rev-parse", "refs/heads/qa"])?;
    let remote_environment_tree =
        fixture.git_text(&fixture.remote, &["rev-parse", "refs/heads/qa^{tree}"])?;
    let main_tip = fixture.git_text(&fixture.source, &["rev-parse", "refs/remotes/origin/main"])?;
    let fetch_head = fixture.source.join(".git/FETCH_HEAD");
    std::fs::write(&fetch_head, "preserve user fetch state\n")?;
    std::fs::write(fixture.source.join("base"), "dirty base\n")?;
    std::fs::write(fixture.source.join("untracked"), "leave me alone\n")?;
    let rerere = fixture.source.join(".git/rr-cache/personal");
    std::fs::create_dir_all(rerere.parent().ok_or("rerere parent")?)?;
    std::fs::write(&rerere, "personal resolution\n")?;
    let hook_marker = fixture.directory.path().join("hook-ran");
    let hooks = fixture.directory.path().join("hostile-hooks");
    std::fs::create_dir(&hooks)?;
    let hook = hooks.join("post-checkout");
    std::fs::write(
        &hook,
        format!("#!/bin/sh\nprintf ran > '{}'\n", hook_marker.display()),
    )?;
    make_executable(&hook)?;
    fixture.git(&fixture.source, &["config", "--unset-all", "user.name"])?;
    fixture.git(&fixture.source, &["config", "--unset-all", "user.email"])?;
    fixture.git(
        &fixture.source,
        &[
            "config",
            "--file",
            path_text(&fixture.global)?,
            "user.name",
            "Global Fixture Author",
        ],
    )?;
    fixture.git(
        &fixture.source,
        &[
            "config",
            "--file",
            path_text(&fixture.global)?,
            "user.email",
            "global-fixture@example.com",
        ],
    )?;
    fixture.git(
        &fixture.source,
        &[
            "config",
            "--file",
            path_text(&fixture.global)?,
            "core.hooksPath",
            path_text(&hooks)?,
        ],
    )?;
    fixture.git(
        &fixture.source,
        &[
            "config",
            "--file",
            path_text(&fixture.global)?,
            "commit.gpgSign",
            "true",
        ],
    )?;
    for (key, value) in [
        ("filter.hostile.clean", "false"),
        ("filter.hostile.smudge", "false"),
        ("filter.hostile.required", "true"),
    ] {
        fixture.git(
            &fixture.source,
            &["config", "--file", path_text(&fixture.global)?, key, value],
        )?;
    }

    let git_parameters = format!("'core.hooksPath={}'", hooks.display());
    let output = fixture.preview_with_git_config_parameters(&[], &git_parameters)?;

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let plan: serde_json::Value = serde_json::from_slice(&output.stdout)?;
    assert_eq!(plan["kind"], "restackPlan");
    assert_eq!(plan["schemaVersion"], 1);
    assert_eq!(plan["author"]["name"], "Global Fixture Author");
    assert_eq!(plan["author"]["email"], "global-fixture@example.com");
    assert_eq!(plan["retainedBranches"][0]["name"], "feature/a");
    assert_eq!(
        plan["merges"][0]["message"],
        "Merge branch 'feature/a' into qa"
    );
    assert_eq!(plan["merges"][0]["outcome"], "clean");
    assert_eq!(plan["merges"][0]["firstParent"], main_tip);
    assert_eq!(
        plan["merges"][0]["featureParent"],
        plan["retainedBranches"][0]["tip"]
    );
    assert_eq!(plan["finalTree"], remote_environment_tree);
    assert_eq!(
        plan["droppedMarkers"]
            .as_array()
            .ok_or("dropped markers")?
            .len(),
        1
    );
    assert_eq!(plan["effects"]["commitSigning"], "unsigned");
    assert_eq!(plan["effects"]["pushed"], false);
    assert_eq!(plan["planDigest"].as_str().ok_or("plan digest")?.len(), 64);
    assert_eq!(
        fixture.git_text(&fixture.source, &["rev-parse", "HEAD"])?,
        source_head
    );
    assert_eq!(
        fixture.git_text(&fixture.source, &["rev-parse", "refs/heads/qa"])?,
        local_environment
    );
    assert_eq!(
        fixture.git_text(&fixture.remote, &["rev-parse", "refs/heads/qa"])?,
        remote_environment
    );
    assert_eq!(std::fs::read_to_string(&rerere)?, "personal resolution\n");
    assert_eq!(
        std::fs::read_to_string(fetch_head)?,
        "preserve user fetch state\n"
    );
    assert!(!hook_marker.exists());
    assert_eq!(
        std::fs::read_to_string(fixture.source.join("base"))?,
        "dirty base\n"
    );
    assert_eq!(
        std::fs::read_to_string(fixture.source.join("untracked"))?,
        "leave me alone\n"
    );
    Ok(())
}

#[test]
fn restack_dry_run_defaults_to_retaining_every_feature() -> Result<(), Box<dyn Error>> {
    let fixture = RestackFixture::new()?;
    let output = gd_command()?
        .current_dir(&fixture.source)
        .args(["restack", "qa", "--main", "main", "--dry-run"])
        .env("GIT_CONFIG_GLOBAL", &fixture.global)
        .env("XDG_CACHE_HOME", &fixture.cache)
        .output()?;

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let plan: serde_json::Value = serde_json::from_slice(&output.stdout)?;
    assert_eq!(plan["kind"], "restackPlan");
    assert_eq!(plan["retainedBranches"][0]["name"], "feature/a");
    assert_eq!(plan["removedBranches"], serde_json::json!([]));
    assert_eq!(plan["effects"]["pushed"], false);
    Ok(())
}

#[test]
fn restack_preview_fetches_and_binds_a_changed_feature_tip() -> Result<(), Box<dyn Error>> {
    let fixture = RestackFixture::new()?;
    let first_output = fixture.preview(&[])?;
    assert!(first_output.status.success());
    let first: serde_json::Value = serde_json::from_slice(&first_output.stdout)?;
    let stale_tip = first["retainedBranches"][0]["tip"]
        .as_str()
        .ok_or("first feature tip")?
        .to_owned();
    let repeated_output = fixture.preview(&[])?;
    assert!(repeated_output.status.success());
    let repeated: serde_json::Value = serde_json::from_slice(&repeated_output.stdout)?;
    assert_eq!(repeated["planDigest"], first["planDigest"]);
    assert_eq!(repeated["finalTree"], first["finalTree"]);
    fixture.git(
        &fixture.source,
        &["config", "--unset-all", "remote.origin.fetch"],
    )?;
    fixture.git(
        &fixture.source,
        &[
            "config",
            "--add",
            "remote.origin.fetch",
            "+refs/heads/main:refs/remotes/origin/main",
        ],
    )?;
    fixture.git(
        &fixture.source,
        &[
            "config",
            "--add",
            "remote.origin.fetch",
            "+refs/heads/qa:refs/remotes/origin/qa",
        ],
    )?;
    let advanced_tip = fixture.advance_feature_from_separate_clone()?;
    assert_eq!(
        fixture.git_text(
            &fixture.source,
            &["rev-parse", "refs/remotes/origin/feature/a"]
        )?,
        stale_tip
    );

    let second_output = fixture.preview(&[])?;

    assert!(
        second_output.status.success(),
        "{}",
        String::from_utf8_lossy(&second_output.stderr)
    );
    let second: serde_json::Value = serde_json::from_slice(&second_output.stdout)?;
    assert_eq!(second["retainedBranches"][0]["tip"], advanced_tip);
    assert_ne!(second["planDigest"], first["planDigest"]);
    assert_ne!(second["finalTree"], first["finalTree"]);
    Ok(())
}

#[test]
fn restack_preview_ignores_fetch_refspecs_that_target_local_branches() -> Result<(), Box<dyn Error>>
{
    let fixture = RestackFixture::new()?;
    let local_environment = fixture.git_text(&fixture.source, &["rev-parse", "refs/heads/qa"])?;
    fixture.git(
        &fixture.source,
        &[
            "config",
            "--add",
            "remote.origin.fetch",
            "+refs/heads/qa:refs/heads/qa",
        ],
    )?;
    let advanced_environment = fixture.advance_environment_from_separate_clone()?;

    let output = fixture.preview(&[])?;

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let plan: serde_json::Value = serde_json::from_slice(&output.stdout)?;
    assert_eq!(plan["environment"]["oid"], advanced_environment);
    assert_eq!(
        fixture.git_text(&fixture.source, &["rev-parse", "refs/heads/qa"])?,
        local_environment
    );
    Ok(())
}

#[test]
fn restack_preview_can_remove_every_explicit_feature_without_pushing() -> Result<(), Box<dyn Error>>
{
    let fixture = RestackFixture::new()?;
    let remote_environment = fixture.git_text(&fixture.remote, &["rev-parse", "refs/heads/qa"])?;
    let main_tree = fixture.git_text(&fixture.remote, &["rev-parse", "refs/heads/main^{tree}"])?;

    let output = fixture.preview(&["feature/a"])?;

    assert!(output.status.success());
    let plan: serde_json::Value = serde_json::from_slice(&output.stdout)?;
    assert_eq!(plan["removedBranches"][0]["name"], "feature/a");
    assert_eq!(plan["retainedBranches"], serde_json::json!([]));
    assert_eq!(plan["merges"], serde_json::json!([]));
    assert_eq!(plan["finalTree"], main_tree);
    assert_eq!(
        fixture.git_text(&fixture.remote, &["rev-parse", "refs/heads/qa"])?,
        remote_environment
    );
    Ok(())
}

#[test]
fn restack_apply_publishes_the_reviewed_tree_without_mutating_local_state(
) -> Result<(), Box<dyn Error>> {
    let fixture = RestackFixture::new()?;
    fixture.git(
        &fixture.source,
        &["remote", "set-url", "origin", "../remote.git"],
    )?;
    let preview = fixture.preview(&[])?;
    assert!(preview.status.success());
    let plan: serde_json::Value = serde_json::from_slice(&preview.stdout)?;
    let digest = plan["planDigest"].as_str().ok_or("plan digest")?;
    let local_head = fixture.git_text(&fixture.source, &["rev-parse", "HEAD"])?;
    let local_environment = fixture.git_text(&fixture.source, &["rev-parse", "refs/heads/qa"])?;
    let personal = fixture.source.join(".git/rr-cache/personal");
    std::fs::create_dir_all(personal.parent().ok_or("personal rerere parent")?)?;
    std::fs::write(&personal, "personal\n")?;
    let hook_marker = fixture.directory.path().join("pre-push-ran");
    let hooks = fixture.directory.path().join("push-hooks");
    std::fs::create_dir(&hooks)?;
    let hook = hooks.join("pre-push");
    std::fs::write(
        &hook,
        format!("#!/bin/sh\nprintf ran > '{}'\n", hook_marker.display()),
    )?;
    make_executable(&hook)?;
    fixture.git(
        &fixture.source,
        &[
            "config",
            "--file",
            path_text(&fixture.global)?,
            "core.hooksPath",
            path_text(&hooks)?,
        ],
    )?;

    let output = fixture.apply(&[], digest)?;

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let result: serde_json::Value = serde_json::from_slice(&output.stdout)?;
    assert_eq!(result["kind"], "restackResult");
    assert_eq!(result["planDigest"], plan["planDigest"]);
    assert_eq!(result["tree"], plan["finalTree"]);
    assert_eq!(result["pushed"], true);
    assert_eq!(result["resolutionCounts"]["clean"], 1);
    assert_eq!(
        fixture.git_text(&fixture.remote, &["rev-parse", "refs/heads/qa"])?,
        result["environment"]["newOid"]
    );
    assert_eq!(
        fixture.git_text(&fixture.remote, &["rev-parse", "refs/heads/qa^{tree}"])?,
        plan["finalTree"]
    );
    assert_eq!(
        fixture.git_text(&fixture.source, &["rev-parse", "HEAD"])?,
        local_head
    );
    assert_eq!(
        fixture.git_text(&fixture.source, &["rev-parse", "refs/heads/qa"])?,
        local_environment
    );
    assert_eq!(std::fs::read_to_string(personal)?, "personal\n");
    assert!(!hook_marker.exists());
    Ok(())
}

#[test]
fn restack_apply_rejects_missing_authority_and_fetched_ref_changes() -> Result<(), Box<dyn Error>> {
    let missing = RestackFixture::new()?;
    let unauthorized = missing.apply_without_digest()?;
    assert_eq!(unauthorized.status.code(), Some(2));
    assert_eq!(
        structured_restack_error(unauthorized)?["code"],
        "plan_digest_required"
    );

    for drift in ["main", "feature", "environment", "deletedFeature"] {
        let fixture = RestackFixture::new()?;
        let preview = fixture.preview(&[])?;
        let plan: serde_json::Value = serde_json::from_slice(&preview.stdout)?;
        let digest = plan["planDigest"].as_str().ok_or("plan digest")?;
        let before = fixture.git_text(&fixture.remote, &["rev-parse", "refs/heads/qa"])?;
        match drift {
            "main" => {
                fixture.advance_main_from_separate_clone()?;
            }
            "feature" => {
                fixture.advance_feature_from_separate_clone()?;
            }
            "environment" => {
                fixture.advance_environment_from_separate_clone()?;
            }
            "deletedFeature" => {
                fixture.git(
                    &fixture.remote,
                    &["update-ref", "-d", "refs/heads/feature/a"],
                )?;
            }
            _ => return Err("unknown drift scenario".into()),
        }

        let output = fixture.apply(&[], digest)?;

        assert_eq!(output.status.code(), Some(1), "{drift}");
        let error = structured_restack_error(output)?;
        let expected_code = if drift == "deletedFeature" {
            "unsupported_history"
        } else {
            "stale_plan"
        };
        assert_eq!(error["code"], expected_code, "{drift}");
        if drift != "environment" {
            assert_eq!(
                fixture.git_text(&fixture.remote, &["rev-parse", "refs/heads/qa"])?,
                before,
                "{drift}"
            );
        }
    }
    Ok(())
}

#[test]
fn restack_apply_binds_the_reviewed_remote_endpoint_and_reports_remote_rejection(
) -> Result<(), Box<dyn Error>> {
    let redirected = RestackFixture::new()?;
    let preview = redirected.preview(&[])?;
    let plan: serde_json::Value = serde_json::from_slice(&preview.stdout)?;
    let digest = plan["planDigest"].as_str().ok_or("plan digest")?;
    let other_remote = redirected.directory.path().join("other-remote.git");
    run_git(
        redirected.directory.path(),
        &redirected.global,
        &[
            "clone",
            "--bare",
            "-q",
            path_text(&redirected.remote)?,
            path_text(&other_remote)?,
        ],
    )?;
    redirected.git(
        &redirected.source,
        &[
            "remote",
            "set-url",
            "--push",
            "origin",
            path_text(&other_remote)?,
        ],
    )?;

    let changed_endpoint = redirected.apply(&[], digest)?;

    assert_eq!(changed_endpoint.status.code(), Some(1));
    assert_eq!(
        structured_restack_error(changed_endpoint)?["code"],
        "stale_plan"
    );

    let rejected = RestackFixture::new()?;
    let preview = rejected.preview(&[])?;
    let plan: serde_json::Value = serde_json::from_slice(&preview.stdout)?;
    let digest = plan["planDigest"].as_str().ok_or("plan digest")?;
    let before = rejected.git_text(&rejected.remote, &["rev-parse", "refs/heads/qa"])?;
    let hook = rejected.remote.join("hooks/pre-receive");
    std::fs::write(&hook, "#!/bin/sh\nexit 1\n")?;
    make_executable(&hook)?;

    let output = rejected.apply(&[], digest)?;

    assert_eq!(output.status.code(), Some(1));
    assert_eq!(structured_restack_error(output)?["code"], "push_rejected");
    assert_eq!(
        rejected.git_text(&rejected.remote, &["rev-parse", "refs/heads/qa"])?,
        before
    );
    Ok(())
}

#[cfg(unix)]
#[test]
fn restack_apply_rejects_a_retargeted_local_remote_symlink() -> Result<(), Box<dyn Error>> {
    use std::os::unix::fs::symlink;

    let fixture = RestackFixture::new()?;
    let endpoint = fixture.directory.path().join("remote-endpoint");
    symlink(&fixture.remote, &endpoint)?;
    fixture.git(
        &fixture.source,
        &["remote", "set-url", "origin", path_text(&endpoint)?],
    )?;
    let preview = fixture.preview(&[])?;
    let plan: serde_json::Value = serde_json::from_slice(&preview.stdout)?;
    let digest = plan["planDigest"].as_str().ok_or("plan digest")?;
    let other_remote = fixture.directory.path().join("retargeted.git");
    run_git(
        fixture.directory.path(),
        &fixture.global,
        &[
            "clone",
            "--bare",
            "-q",
            path_text(&fixture.remote)?,
            path_text(&other_remote)?,
        ],
    )?;
    std::fs::remove_file(&endpoint)?;
    symlink(&other_remote, &endpoint)?;

    let output = fixture.apply(&[], digest)?;

    assert_eq!(output.status.code(), Some(1));
    assert_eq!(structured_restack_error(output)?["code"], "stale_plan");
    assert_eq!(
        fixture.git_text(&fixture.remote, &["rev-parse", "refs/heads/qa"])?,
        fixture.git_text(&other_remote, &["rev-parse", "refs/heads/qa"])?
    );
    Ok(())
}

#[test]
fn restack_apply_revalidates_removed_refs_on_a_distinct_push_endpoint() -> Result<(), Box<dyn Error>>
{
    let fixture = RestackFixture::new()?;
    let push_remote = fixture.directory.path().join("push-remote.git");
    run_git(
        fixture.directory.path(),
        &fixture.global,
        &[
            "clone",
            "--bare",
            "-q",
            path_text(&fixture.remote)?,
            path_text(&push_remote)?,
        ],
    )?;
    fixture.git(
        &fixture.source,
        &[
            "remote",
            "set-url",
            "--push",
            "origin",
            path_text(&push_remote)?,
        ],
    )?;
    let preview = fixture.preview(&["feature/a"])?;
    let plan: serde_json::Value = serde_json::from_slice(&preview.stdout)?;
    let digest = plan["planDigest"].as_str().ok_or("plan digest")?;
    let writer = fixture.directory.path().join("push-writer");
    run_git(
        fixture.directory.path(),
        &fixture.global,
        &["clone", "-q", path_text(&push_remote)?, path_text(&writer)?],
    )?;
    fixture.git(&writer, &["config", "user.name", "Other Author"])?;
    fixture.git(&writer, &["config", "user.email", "other@example.com"])?;
    fixture.git(&writer, &["checkout", "-q", "feature/a"])?;
    std::fs::write(writer.join("push-only-change"), "changed\n")?;
    fixture.git(&writer, &["add", "push-only-change"])?;
    fixture.git(&writer, &["commit", "-q", "-m", "advance removed ref"])?;
    fixture.git(&writer, &["push", "-q", "origin", "feature/a"])?;
    let environment_before = fixture.git_text(&push_remote, &["rev-parse", "refs/heads/qa"])?;

    let output = fixture.apply(&["feature/a"], digest)?;

    assert_eq!(output.status.code(), Some(1));
    let error = structured_restack_error(output)?;
    assert_eq!(error["code"], "stale_plan");
    assert_eq!(error["details"]["reason"], "movedRef");
    assert_eq!(error["details"]["endpoint"], "push");
    assert_eq!(
        fixture.git_text(&push_remote, &["rev-parse", "refs/heads/qa"])?,
        environment_before
    );
    Ok(())
}

#[cfg(unix)]
#[test]
fn restack_apply_exact_lease_rejects_an_environment_race_before_publication(
) -> Result<(), Box<dyn Error>> {
    let fixture = RestackFixture::new()?;
    let preview = fixture.preview(&[])?;
    assert!(
        preview.status.success(),
        "{}",
        String::from_utf8_lossy(&preview.stderr)
    );
    let plan: serde_json::Value = serde_json::from_slice(&preview.stdout)?;
    let digest = plan["planDigest"].as_str().ok_or("plan digest")?;
    let old_oid = fixture.git_text(&fixture.remote, &["rev-parse", "refs/heads/qa"])?;
    let race_oid = fixture.git_text(&fixture.remote, &["rev-parse", "refs/heads/main"])?;
    let wrapper_directory = fixture.directory.path().join("git-wrapper");
    std::fs::create_dir(&wrapper_directory)?;
    let wrapper = wrapper_directory.join("git");
    let real_git = find_git_executable()?;
    std::fs::write(
        &wrapper,
        format!(
            "#!/bin/sh\nif [ \"$1\" = push ]; then\n  '{}' --git-dir='{}' update-ref refs/heads/qa '{}' '{}' || exit 91\nfi\nexec '{}' \"$@\"\n",
            real_git.display(),
            fixture.remote.display(),
            race_oid,
            old_oid,
            real_git.display(),
        ),
    )?;
    make_executable(&wrapper)?;
    let path = std::env::join_paths(std::iter::once(wrapper_directory).chain(
        std::env::split_paths(&std::env::var_os("PATH").ok_or("PATH")?),
    ))?;

    let output = fixture.apply_with_path(&[], digest, &path)?;

    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        fixture.git_text(&fixture.remote, &["rev-parse", "refs/heads/qa"])?,
        race_oid
    );
    Ok(())
}

#[test]
fn restack_preview_prunes_a_deleted_feature_before_inventory() -> Result<(), Box<dyn Error>> {
    let fixture = RestackFixture::new()?;
    fixture.git(
        &fixture.remote,
        &["update-ref", "-d", "refs/heads/feature/a"],
    )?;

    let output = fixture.preview(&[])?;

    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8(output.stderr)?;
    let error: serde_json::Value =
        serde_json::from_str(stderr.lines().last().ok_or("structured inventory error")?)?;
    assert_eq!(error["code"], "unsupported_history");
    assert_eq!(error["details"]["kind"], "deletedFeatureRef");
    Ok(())
}

#[test]
fn restack_machine_failures_are_structured_and_redact_fetch_secrets() -> Result<(), Box<dyn Error>>
{
    let directory = tempfile::tempdir()?;
    let source = directory.path().join("source");
    let global = directory.path().join("global.gitconfig");
    std::fs::write(&global, [])?;
    std::fs::create_dir(&source)?;
    run_git(&source, &global, &["init", "-q", "-b", "main"])?;
    run_git(&source, &global, &["config", "user.name", "Fixture Author"])?;
    run_git(
        &source,
        &global,
        &["config", "user.email", "fixture@example.com"],
    )?;
    let secret_path = directory.path().join("remote-url-secret-do-not-print");
    std::fs::create_dir(&secret_path)?;
    run_git(
        &source,
        &global,
        &["remote", "add", "origin", path_text(&secret_path)?],
    )?;

    let invalid = gd_command()?
        .current_dir(&source)
        .args([
            "restack",
            "qa",
            "--params",
            r#"{"removeBranches":"not-an-array"}"#,
        ])
        .env("XDG_CACHE_HOME", directory.path().join("cache"))
        .output()?;
    assert_eq!(invalid.status.code(), Some(2));
    let invalid_error: serde_json::Value = serde_json::from_slice(&invalid.stderr)?;
    assert_eq!(invalid_error["kind"], "restackError");
    assert_eq!(invalid_error["schemaVersion"], 1);
    assert_eq!(invalid_error["code"], "invalid_params");

    let non_terminal = gd_command()?
        .current_dir(&source)
        .args(["restack", "qa"])
        .output()?;
    assert_eq!(non_terminal.status.code(), Some(2));
    let non_terminal_error: serde_json::Value = serde_json::from_slice(&non_terminal.stderr)?;
    assert_eq!(non_terminal_error["code"], "params_required");

    let clap_invalid = gd_command()?
        .args(["restack", "--params", r#"{"removeBranches":[]}"#])
        .output()?;
    assert_eq!(clap_invalid.status.code(), Some(2));
    let clap_error: serde_json::Value = serde_json::from_slice(&clap_invalid.stderr)?;
    assert_eq!(clap_error["code"], "invalid_usage");

    for arguments in [
        vec!["restack", "qa", "--abort"],
        vec!["restack", "qa", "--resume", "opaque", "--apply", "--abort"],
        vec![
            "restack",
            "qa",
            "--resume",
            "opaque",
            "--params",
            r#"{"removeBranches":[]}"#,
        ],
    ] {
        let invalid_combination = gd_command()?.args(arguments).output()?;
        assert_eq!(invalid_combination.status.code(), Some(2));
        let error: serde_json::Value = serde_json::from_slice(&invalid_combination.stderr)?;
        assert_eq!(error["code"], "invalid_usage");
    }

    let fetch = gd_command()?
        .current_dir(&source)
        .args([
            "restack",
            "qa",
            "--params",
            r#"{"removeBranches":[],"planDigest":"0000000000000000000000000000000000000000000000000000000000000000"}"#,
            "--apply",
        ])
        .env("GIT_PAT", "pat-secret-do-not-print")
        .env("XDG_CACHE_HOME", directory.path().join("cache"))
        .output()?;
    assert_eq!(fetch.status.code(), Some(1));
    let stdout = String::from_utf8_lossy(&fetch.stdout);
    let stderr = String::from_utf8_lossy(&fetch.stderr);
    assert!(!stdout.contains("pat-secret-do-not-print"));
    assert!(!stderr.contains("pat-secret-do-not-print"));
    assert!(!stdout.contains("remote-url-secret-do-not-print"));
    assert!(!stderr.contains("remote-url-secret-do-not-print"));
    let error_line = stderr.lines().last().ok_or("structured fetch error")?;
    let fetch_error: serde_json::Value = serde_json::from_str(error_line)?;
    assert_eq!(fetch_error["code"], "fetch_failed");
    Ok(())
}

#[test]
fn restack_reuses_an_accepted_resolution_and_resumes_a_changed_conflict(
) -> Result<(), Box<dyn Error>> {
    let trained = ConflictRestackFixture::new()?;
    let personal = trained.source.join(".git/rr-cache/personal");
    std::fs::create_dir_all(personal.parent().ok_or("personal rerere parent")?)?;
    std::fs::write(&personal, "personal\n")?;

    let replay = trained.preview()?;

    assert!(
        replay.status.success(),
        "{}",
        String::from_utf8_lossy(&replay.stderr)
    );
    let replay_plan: serde_json::Value = serde_json::from_slice(&replay.stdout)?;
    assert_eq!(replay_plan["merges"][0]["outcome"], "rerere");
    assert_eq!(std::fs::read_to_string(&personal)?, "personal\n");

    let resumed = ConflictRestackFixture::new()?;
    let remote_before = resumed.git_text(&resumed.remote, &["rev-parse", "refs/heads/qa"])?;
    resumed.advance_main()?;
    let conflict = resumed.preview()?;
    assert_eq!(conflict.status.code(), Some(1));
    let conflict_stderr = String::from_utf8(conflict.stderr)?;
    let error: serde_json::Value = serde_json::from_str(
        conflict_stderr
            .lines()
            .last()
            .ok_or("structured conflict error")?,
    )?;
    assert_eq!(error["code"], "reconstruction_conflict");
    assert_eq!(error["details"]["branch"], "feature/conflict");
    assert_eq!(
        error["details"]["unresolvedPaths"],
        serde_json::json!(["conflict"])
    );
    let token = error["details"]["resumeToken"]
        .as_str()
        .ok_or("resume token")?;
    let work_area = PathBuf::from(error["details"]["workArea"].as_str().ok_or("work area")?);
    assert!(error["details"]["expiresAt"].as_u64().is_some());
    assert_eq!(
        resumed.git_text(&resumed.remote, &["rev-parse", "refs/heads/qa"])?,
        remote_before
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        let session = work_area.parent().ok_or("session directory")?;
        assert_eq!(
            std::fs::metadata(session)?.permissions().mode() & 0o777,
            0o700
        );
        assert_eq!(
            std::fs::metadata(session.join("session.json"))?
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        assert_eq!(
            std::fs::metadata(
                session
                    .parent()
                    .ok_or("session store")?
                    .join("integrity.key")
            )?
            .permissions()
            .mode()
                & 0o777,
            0o600
        );
    }

    std::fs::write(work_area.join("conflict"), "manual resolution\n")?;
    resumed.git(&work_area, &["add", "conflict"])?;
    let output = resumed.resume(token, "qa", &resumed.source)?;

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let plan: serde_json::Value = serde_json::from_slice(&output.stdout)?;
    assert_eq!(plan["merges"][0]["outcome"], "manual");
    assert_eq!(plan["effects"]["pushed"], false);
    assert!(!String::from_utf8(output.stdout)?.contains(token));
    assert_eq!(
        resumed.git_text(&resumed.remote, &["rev-parse", "refs/heads/qa"])?,
        remote_before
    );
    Ok(())
}

#[test]
fn restack_resume_apply_publishes_the_sealed_manual_resolution_once() -> Result<(), Box<dyn Error>>
{
    let fixture = ConflictRestackFixture::new()?;
    let local_environment = fixture.git_text(&fixture.source, &["rev-parse", "refs/heads/qa"])?;
    let (token, work_area, plan) = fixture.seal_manual_resolution()?;

    let output = fixture.resume_apply(&token)?;

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let result: serde_json::Value = serde_json::from_slice(&output.stdout)?;
    assert_eq!(result["kind"], "restackResult");
    assert_eq!(result["planDigest"], plan["planDigest"]);
    assert_eq!(result["tree"], plan["finalTree"]);
    assert_eq!(result["resolutionCounts"]["manual"], 1);
    assert_eq!(result["pushed"], true);
    assert_eq!(
        fixture.git_text(&fixture.remote, &["rev-parse", "refs/heads/qa^{tree}"])?,
        plan["finalTree"].as_str().ok_or("final tree")?
    );
    assert_eq!(
        fixture.git_text(&fixture.source, &["rev-parse", "refs/heads/qa"])?,
        local_environment
    );
    assert!(!work_area.parent().ok_or("session directory")?.exists());
    let output_text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!output_text.contains(&token));

    let replay = structured_restack_error(fixture.resume_apply(&token)?)?;
    assert_eq!(replay["code"], "invalid_session");
    assert_eq!(replay["details"]["reason"], "missing");
    Ok(())
}

#[test]
fn restack_resume_apply_revalidates_remote_inputs_without_consuming_a_stale_session(
) -> Result<(), Box<dyn Error>> {
    let fixture = ConflictRestackFixture::new()?;
    let (token, _work_area, _plan) = fixture.seal_manual_resolution()?;
    let expected_main = fixture.git_text(&fixture.remote, &["rev-parse", "refs/heads/main"])?;
    let stale_main = fixture.git_text(
        &fixture.remote,
        &["rev-parse", "refs/heads/feature/conflict"],
    )?;
    fixture.git(
        &fixture.remote,
        &["update-ref", "refs/heads/main", &stale_main, &expected_main],
    )?;

    let stale = structured_restack_error(fixture.resume_apply(&token)?)?;

    assert_eq!(stale["code"], "stale_plan");
    assert_eq!(stale["details"]["reason"], "movedRef");
    fixture.git(
        &fixture.remote,
        &["update-ref", "refs/heads/main", &expected_main, &stale_main],
    )?;
    let retry = fixture.resume_apply(&token)?;
    assert!(
        retry.status.success(),
        "{}",
        String::from_utf8_lossy(&retry.stderr)
    );
    Ok(())
}

#[test]
fn restack_resume_apply_rejects_endpoint_retargeting_without_consuming_the_session(
) -> Result<(), Box<dyn Error>> {
    let fixture = ConflictRestackFixture::new()?;
    let (token, _work_area, _plan) = fixture.seal_manual_resolution()?;
    let other_remote = fixture._directory.path().join("other-remote.git");
    run_git(
        fixture._directory.path(),
        &fixture.global,
        &[
            "clone",
            "--bare",
            "-q",
            path_text(&fixture.remote)?,
            path_text(&other_remote)?,
        ],
    )?;
    fixture.git(
        &fixture.source,
        &[
            "remote",
            "set-url",
            "--push",
            "origin",
            path_text(&other_remote)?,
        ],
    )?;

    let stale = structured_restack_error(fixture.resume_apply(&token)?)?;

    assert_eq!(stale["code"], "stale_plan");
    assert_eq!(stale["details"]["reason"], "remoteEndpoint");
    fixture.git(
        &fixture.source,
        &[
            "remote",
            "set-url",
            "--push",
            "origin",
            path_text(&fixture.remote)?,
        ],
    )?;
    let retry = fixture.resume_apply(&token)?;
    assert!(
        retry.status.success(),
        "{}",
        String::from_utf8_lossy(&retry.stderr)
    );
    Ok(())
}

#[test]
fn restack_resume_apply_preserves_the_session_after_a_rejected_push() -> Result<(), Box<dyn Error>>
{
    let fixture = ConflictRestackFixture::new()?;
    let (token, _work_area, _plan) = fixture.seal_manual_resolution()?;
    let hook = fixture.remote.join("hooks/pre-receive");
    std::fs::write(&hook, "#!/bin/sh\nexit 1\n")?;
    make_executable(&hook)?;

    let rejected = structured_restack_error(fixture.resume_apply(&token)?)?;

    assert_eq!(rejected["code"], "push_rejected");
    std::fs::remove_file(hook)?;
    let retry = fixture.resume_apply(&token)?;
    assert!(
        retry.status.success(),
        "{}",
        String::from_utf8_lossy(&retry.stderr)
    );
    Ok(())
}

#[cfg(unix)]
#[test]
fn restack_resume_apply_consumes_the_session_when_push_success_is_reported_as_failure(
) -> Result<(), Box<dyn Error>> {
    let fixture = ConflictRestackFixture::new()?;
    let (token, _work_area, plan) = fixture.seal_manual_resolution()?;
    let wrapper_directory = fixture._directory.path().join("git-wrapper");
    std::fs::create_dir(&wrapper_directory)?;
    let wrapper = wrapper_directory.join("git");
    let real_git = find_git_executable()?;
    std::fs::write(
        &wrapper,
        format!(
            "#!/bin/sh\nif [ \"$1\" = push ]; then\n  '{}' \"$@\"\n  result=$?\n  if [ \"$result\" -eq 0 ]; then exit 1; fi\n  exit \"$result\"\nfi\nexec '{}' \"$@\"\n",
            real_git.display(),
            real_git.display(),
        ),
    )?;
    make_executable(&wrapper)?;
    let path = std::env::join_paths(std::iter::once(wrapper_directory).chain(
        std::env::split_paths(&std::env::var_os("PATH").ok_or("PATH")?),
    ))?;

    let output = fixture.resume_apply_with_path(&token, &path)?;

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let result: serde_json::Value = serde_json::from_slice(&output.stdout)?;
    assert_eq!(result["planDigest"], plan["planDigest"]);
    let replay = structured_restack_error(fixture.resume_apply(&token)?)?;
    assert_eq!(replay["code"], "invalid_session");
    assert_eq!(replay["details"]["reason"], "missing");
    Ok(())
}

#[test]
fn restack_abort_consumes_a_conflicted_session_without_changing_refs() -> Result<(), Box<dyn Error>>
{
    let fixture = ConflictRestackFixture::new()?;
    fixture.advance_main()?;
    let before = fixture.git_text(&fixture.remote, &["rev-parse", "refs/heads/qa"])?;
    let conflict = structured_restack_error(fixture.preview()?)?;
    let token = conflict["details"]["resumeToken"]
        .as_str()
        .ok_or("resume token")?;
    let work_area = PathBuf::from(
        conflict["details"]["workArea"]
            .as_str()
            .ok_or("work area")?,
    );

    let output = fixture.abort(token)?;

    assert!(output.status.success());
    let result: serde_json::Value = serde_json::from_slice(&output.stdout)?;
    assert_eq!(result["kind"], "restackAbortResult");
    assert_eq!(result["aborted"], true);
    assert_eq!(result["effects"]["remoteRefsChanged"], false);
    assert!(!String::from_utf8(output.stdout)?.contains(token));
    assert!(!work_area.parent().ok_or("session directory")?.exists());
    assert_eq!(
        fixture.git_text(&fixture.remote, &["rev-parse", "refs/heads/qa"])?,
        before
    );
    let replay = structured_restack_error(fixture.abort(token)?)?;
    assert_eq!(replay["code"], "invalid_session");
    assert_eq!(replay["details"]["reason"], "missing");
    Ok(())
}

#[test]
fn restack_resume_rejects_expired_locked_tampered_and_mismatched_sessions(
) -> Result<(), Box<dyn Error>> {
    let fixture = ConflictRestackFixture::new()?;
    fixture.advance_main()?;
    let conflict = structured_restack_error(fixture.preview()?)?;
    let token = conflict["details"]["resumeToken"]
        .as_str()
        .ok_or("resume token")?;
    let work_area = PathBuf::from(
        conflict["details"]["workArea"]
            .as_str()
            .ok_or("work area")?,
    );

    let wrong_environment =
        structured_restack_error(fixture.resume(token, "other", &fixture.source)?)?;
    assert_eq!(wrong_environment["code"], "stale_session");
    assert_eq!(wrong_environment["details"]["reason"], "environment");

    let other = fixture._directory.path().join("other");
    std::fs::create_dir(&other)?;
    fixture.git(&other, &["init", "-q", "-b", "main"])?;
    let wrong_repository = structured_restack_error(fixture.resume(token, "qa", &other)?)?;
    assert_eq!(wrong_repository["code"], "stale_session");
    assert_eq!(wrong_repository["details"]["reason"], "repository");

    let lock_path = work_area
        .parent()
        .ok_or("session directory")?
        .join("session.lock");
    let lock = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(lock_path)?;
    fs2::FileExt::lock_exclusive(&lock)?;
    let locked = structured_restack_error(fixture.resume(token, "qa", &fixture.source)?)?;
    assert_eq!(locked["code"], "session_locked");
    fs2::FileExt::unlock(&lock)?;
    drop(lock);

    let metadata_path = work_area
        .parent()
        .ok_or("session directory")?
        .join("session.json");
    let mut metadata: serde_json::Value = serde_json::from_slice(&std::fs::read(&metadata_path)?)?;
    metadata["metadata"]["author"]["name"] = serde_json::json!("tampered");
    let token_key = session_token_secret(token)?;
    sign_session_envelope(&mut metadata, &token_key)?;
    std::fs::write(&metadata_path, serde_json::to_vec_pretty(&metadata)?)?;
    let tampered = structured_restack_error(fixture.resume(token, "qa", &fixture.source)?)?;
    assert_eq!(tampered["code"], "invalid_session");
    assert_eq!(tampered["details"]["reason"], "tampered");

    let committed = ConflictRestackFixture::new()?;
    committed.advance_main()?;
    let conflict = structured_restack_error(committed.preview()?)?;
    let committed_token = conflict["details"]["resumeToken"]
        .as_str()
        .ok_or("committed token")?;
    let committed_area = PathBuf::from(
        conflict["details"]["workArea"]
            .as_str()
            .ok_or("committed work area")?,
    );
    std::fs::write(committed_area.join("conflict"), "agent resolution\n")?;
    committed.git(&committed_area, &["add", "conflict"])?;
    let expected_head = committed.git_text(&committed_area, &["rev-parse", "HEAD"])?;
    let expected_feature = committed.git_text(&committed_area, &["rev-parse", "MERGE_HEAD"])?;
    committed.git(
        &committed_area,
        &[
            "-c",
            "user.name=Agent",
            "-c",
            "user.email=agent@example.com",
            "-c",
            "rerere.enabled=false",
            "commit",
            "-q",
            "-m",
            "agent commit",
        ],
    )?;
    committed.git(&committed_area, &["reset", "--soft", &expected_head])?;
    std::fs::write(
        committed_area.join(".git/MERGE_HEAD"),
        format!("{expected_feature}\n"),
    )?;
    assert_eq!(
        committed.git_text(&committed_area, &["rev-parse", "HEAD"])?,
        expected_head
    );
    assert_eq!(
        committed.git_text(&committed_area, &["rev-parse", "MERGE_HEAD"])?,
        expected_feature
    );
    let agent_commit =
        structured_restack_error(committed.resume(committed_token, "qa", &committed.source)?)?;
    assert_eq!(agent_commit["code"], "invalid_session_state");
    assert_eq!(agent_commit["details"]["reason"], "agentCommit");

    let expired = ConflictRestackFixture::new()?;
    expired.advance_main()?;
    let conflict = structured_restack_error(expired.preview()?)?;
    let expired_token = conflict["details"]["resumeToken"]
        .as_str()
        .ok_or("expired token")?;
    let expired_area = PathBuf::from(
        conflict["details"]["workArea"]
            .as_str()
            .ok_or("expired work area")?,
    );
    expire_session(&expired_area)?;
    let expired_error =
        structured_restack_error(expired.resume(expired_token, "qa", &expired.source)?)?;
    assert_eq!(expired_error["code"], "expired_session");
    assert!(!expired_area.exists());
    Ok(())
}

fn structured_restack_error(
    output: std::process::Output,
) -> Result<serde_json::Value, Box<dyn Error>> {
    if output.status.success() {
        return Err("restack invocation unexpectedly succeeded".into());
    }
    let stderr = String::from_utf8(output.stderr)?;
    Ok(serde_json::from_str(
        stderr.lines().last().ok_or("structured restack error")?,
    )?)
}

fn expire_session(work_area: &Path) -> Result<(), Box<dyn Error>> {
    let session_directory = work_area.parent().ok_or("session directory")?;
    let metadata_path = session_directory.join("session.json");
    let mut envelope: serde_json::Value = serde_json::from_slice(&std::fs::read(&metadata_path)?)?;
    envelope["metadata"]["expiresAt"] = serde_json::json!(0);
    let sessions_root = session_directory.parent().ok_or("session store")?;
    let key = std::fs::read(sessions_root.join("integrity.key"))?;
    sign_session_envelope(&mut envelope, &key)?;
    let mut contents = serde_json::to_vec_pretty(&envelope)?;
    contents.push(b'\n');
    std::fs::write(metadata_path, contents)?;
    Ok(())
}

fn session_token_secret(token: &str) -> Result<Vec<u8>, Box<dyn Error>> {
    let secret = token.split('.').nth(2).ok_or("session token secret")?;
    let mut key = Vec::with_capacity(secret.len() / 2);
    for index in (0..secret.len()).step_by(2) {
        key.push(u8::from_str_radix(&secret[index..index + 2], 16)?);
    }
    Ok(key)
}

fn sign_session_envelope(
    envelope: &mut serde_json::Value,
    key: &[u8],
) -> Result<(), Box<dyn Error>> {
    use hmac::{Hmac, Mac};
    use sha2::Sha256;

    let canonical = serde_json::to_vec(&envelope["metadata"])?;
    let mut mac = Hmac::<Sha256>::new_from_slice(key)?;
    mac.update(&canonical);
    mac.update(&[0]);
    mac.update(
        envelope["capability"]
            .as_str()
            .ok_or("session capability")?
            .as_bytes(),
    );
    let integrity = mac
        .finalize()
        .into_bytes()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    envelope["integrity"] = serde_json::json!(integrity);
    Ok(())
}

struct RestackFixture {
    directory: tempfile::TempDir,
    source: PathBuf,
    remote: PathBuf,
    global: PathBuf,
    cache: PathBuf,
}

impl RestackFixture {
    fn new() -> Result<Self, Box<dyn Error>> {
        let directory = tempfile::tempdir()?;
        let source = directory.path().join("source");
        let remote = directory.path().join("remote.git");
        let global = directory.path().join("global.gitconfig");
        std::fs::write(&global, [])?;
        std::fs::create_dir(&source)?;
        std::fs::create_dir(&remote)?;
        run_git(&remote, &global, &["init", "--bare", "-q"])?;
        run_git(
            &remote,
            &global,
            &["symbolic-ref", "HEAD", "refs/heads/main"],
        )?;
        run_git(&source, &global, &["init", "-q", "-b", "main"])?;
        run_git(&source, &global, &["config", "user.name", "Fixture Author"])?;
        run_git(
            &source,
            &global,
            &["config", "user.email", "fixture@example.com"],
        )?;
        run_git(
            &source,
            &global,
            &["remote", "add", "origin", path_text(&remote)?],
        )?;
        std::fs::write(source.join("base"), "base\n")?;
        std::fs::write(source.join(".gitattributes"), "base filter=hostile\n")?;
        run_git(&source, &global, &["add", "base", ".gitattributes"])?;
        run_git(&source, &global, &["commit", "-q", "-m", "base"])?;
        run_git(&source, &global, &["push", "-q", "-u", "origin", "main"])?;
        run_git(
            &source,
            &global,
            &["checkout", "-q", "-b", "feature/a", "main"],
        )?;
        std::fs::write(source.join("feature-a"), "feature a\n")?;
        run_git(&source, &global, &["add", "feature-a"])?;
        run_git(&source, &global, &["commit", "-q", "-m", "feature a"])?;
        run_git(
            &source,
            &global,
            &["push", "-q", "-u", "origin", "feature/a"],
        )?;
        run_git(&source, &global, &["checkout", "-q", "-b", "qa", "main"])?;
        run_git(
            &source,
            &global,
            &[
                "merge",
                "-q",
                "--no-ff",
                "feature/a",
                "-m",
                "accepted feature a",
            ],
        )?;
        run_git(
            &source,
            &global,
            &["commit", "-q", "--allow-empty", "-m", "### Match 'qa'"],
        )?;
        run_git(&source, &global, &["push", "-q", "-u", "origin", "qa"])?;
        run_git(&source, &global, &["checkout", "-q", "main"])?;
        Ok(Self {
            cache: directory.path().join("cache"),
            directory,
            source,
            remote,
            global,
        })
    }

    fn preview(&self, removals: &[&str]) -> Result<std::process::Output, Box<dyn Error>> {
        self.preview_with_environment(removals, None)
    }

    fn preview_with_git_config_parameters(
        &self,
        removals: &[&str],
        parameters: &str,
    ) -> Result<std::process::Output, Box<dyn Error>> {
        self.preview_with_environment(removals, Some(parameters))
    }

    fn preview_with_environment(
        &self,
        removals: &[&str],
        git_config_parameters: Option<&str>,
    ) -> Result<std::process::Output, Box<dyn Error>> {
        let params = serde_json::json!({"removeBranches": removals}).to_string();
        let mut command = gd_command()?;
        command
            .current_dir(&self.source)
            .args(["restack", "qa", "--main", "main", "--params", &params])
            .env("GIT_CONFIG_GLOBAL", &self.global)
            .env("XDG_CACHE_HOME", &self.cache);
        if let Some(parameters) = git_config_parameters {
            command.env("GIT_CONFIG_PARAMETERS", parameters);
        }
        Ok(command.output()?)
    }

    fn apply(
        &self,
        removals: &[&str],
        digest: &str,
    ) -> Result<std::process::Output, Box<dyn Error>> {
        let params = serde_json::json!({
            "removeBranches": removals,
            "planDigest": digest,
        })
        .to_string();
        Ok(gd_command()?
            .current_dir(&self.source)
            .args([
                "restack", "qa", "--main", "main", "--params", &params, "--apply",
            ])
            .env("GIT_CONFIG_GLOBAL", &self.global)
            .env("XDG_CACHE_HOME", &self.cache)
            .output()?)
    }

    #[cfg(unix)]
    fn apply_with_path(
        &self,
        removals: &[&str],
        digest: &str,
        path: &std::ffi::OsStr,
    ) -> Result<std::process::Output, Box<dyn Error>> {
        let params = serde_json::json!({
            "removeBranches": removals,
            "planDigest": digest,
        })
        .to_string();
        Ok(gd_command()?
            .current_dir(&self.source)
            .args([
                "restack", "qa", "--main", "main", "--params", &params, "--apply",
            ])
            .env("GIT_CONFIG_GLOBAL", &self.global)
            .env("XDG_CACHE_HOME", &self.cache)
            .env("PATH", path)
            .output()?)
    }

    fn apply_without_digest(&self) -> Result<std::process::Output, Box<dyn Error>> {
        Ok(gd_command()?
            .current_dir(&self.source)
            .args([
                "restack",
                "qa",
                "--main",
                "main",
                "--params",
                r#"{"removeBranches":[]}"#,
                "--apply",
            ])
            .env("GIT_CONFIG_GLOBAL", &self.global)
            .env("XDG_CACHE_HOME", &self.cache)
            .output()?)
    }

    fn advance_feature_from_separate_clone(&self) -> Result<String, Box<dyn Error>> {
        let writer = self.directory.path().join("writer");
        run_git(
            self.directory.path(),
            &self.global,
            &["clone", "-q", path_text(&self.remote)?, path_text(&writer)?],
        )?;
        run_git(
            &writer,
            &self.global,
            &["config", "user.name", "Other Author"],
        )?;
        run_git(
            &writer,
            &self.global,
            &["config", "user.email", "other@example.com"],
        )?;
        run_git(&writer, &self.global, &["checkout", "-q", "feature/a"])?;
        std::fs::write(writer.join("feature-a-two"), "feature a two\n")?;
        run_git(&writer, &self.global, &["add", "feature-a-two"])?;
        run_git(&writer, &self.global, &["commit", "-q", "-m", "advance a"])?;
        run_git(
            &writer,
            &self.global,
            &["push", "-q", "origin", "feature/a"],
        )?;
        self.git_text(&writer, &["rev-parse", "HEAD"])
    }

    fn advance_main_from_separate_clone(&self) -> Result<String, Box<dyn Error>> {
        let writer = self.directory.path().join("main-writer");
        run_git(
            self.directory.path(),
            &self.global,
            &["clone", "-q", path_text(&self.remote)?, path_text(&writer)?],
        )?;
        run_git(
            &writer,
            &self.global,
            &["config", "user.name", "Other Author"],
        )?;
        run_git(
            &writer,
            &self.global,
            &["config", "user.email", "other@example.com"],
        )?;
        std::fs::write(writer.join("main-two"), "main two\n")?;
        run_git(&writer, &self.global, &["add", "main-two"])?;
        run_git(
            &writer,
            &self.global,
            &["commit", "-q", "-m", "advance main"],
        )?;
        run_git(&writer, &self.global, &["push", "-q", "origin", "main"])?;
        self.git_text(&writer, &["rev-parse", "HEAD"])
    }

    fn advance_environment_from_separate_clone(&self) -> Result<String, Box<dyn Error>> {
        let writer = self.directory.path().join("environment-writer");
        run_git(
            self.directory.path(),
            &self.global,
            &["clone", "-q", path_text(&self.remote)?, path_text(&writer)?],
        )?;
        run_git(
            &writer,
            &self.global,
            &["config", "user.name", "Other Author"],
        )?;
        run_git(
            &writer,
            &self.global,
            &["config", "user.email", "other@example.com"],
        )?;
        run_git(&writer, &self.global, &["checkout", "-q", "qa"])?;
        run_git(
            &writer,
            &self.global,
            &["commit", "-q", "--allow-empty", "-m", "### Match 'qa'"],
        )?;
        run_git(&writer, &self.global, &["push", "-q", "origin", "qa"])?;
        self.git_text(&writer, &["rev-parse", "HEAD"])
    }

    fn git(&self, path: &Path, arguments: &[&str]) -> Result<(), Box<dyn Error>> {
        run_git(path, &self.global, arguments)
    }

    fn git_text(&self, path: &Path, arguments: &[&str]) -> Result<String, Box<dyn Error>> {
        git_text(path, &self.global, arguments)
    }
}

struct ConflictRestackFixture {
    _directory: tempfile::TempDir,
    source: PathBuf,
    remote: PathBuf,
    global: PathBuf,
    cache: PathBuf,
}

impl ConflictRestackFixture {
    fn new() -> Result<Self, Box<dyn Error>> {
        let directory = tempfile::tempdir()?;
        let source = directory.path().join("source");
        let remote = directory.path().join("remote.git");
        let global = directory.path().join("global.gitconfig");
        let cache = directory.path().join("cache");
        std::fs::write(&global, [])?;
        std::fs::create_dir(&source)?;
        std::fs::create_dir(&remote)?;
        run_git(&remote, &global, &["init", "--bare", "-q"])?;
        run_git(
            &remote,
            &global,
            &["symbolic-ref", "HEAD", "refs/heads/main"],
        )?;
        run_git(&source, &global, &["init", "-q", "-b", "main"])?;
        run_git(&source, &global, &["config", "user.name", "Fixture Author"])?;
        run_git(
            &source,
            &global,
            &["config", "user.email", "fixture@example.com"],
        )?;
        run_git(
            &source,
            &global,
            &["remote", "add", "origin", path_text(&remote)?],
        )?;
        std::fs::write(source.join("conflict"), "base\n")?;
        run_git(&source, &global, &["add", "conflict"])?;
        run_git(&source, &global, &["commit", "-q", "-m", "base"])?;
        run_git(&source, &global, &["push", "-q", "-u", "origin", "main"])?;
        run_git(
            &source,
            &global,
            &["checkout", "-q", "-b", "feature/conflict", "main"],
        )?;
        std::fs::write(source.join("conflict"), "feature\n")?;
        run_git(&source, &global, &["commit", "-q", "-am", "feature"])?;
        run_git(
            &source,
            &global,
            &["push", "-q", "-u", "origin", "feature/conflict"],
        )?;
        run_git(&source, &global, &["checkout", "-q", "main"])?;
        std::fs::write(source.join("conflict"), "main old\n")?;
        run_git(&source, &global, &["commit", "-q", "-am", "main old"])?;
        run_git(&source, &global, &["push", "-q", "origin", "main"])?;
        run_git(&source, &global, &["checkout", "-q", "-b", "qa", "main"])?;
        let merge = isolated_git(&global)
            .current_dir(&source)
            .args(["merge", "--no-ff", "--no-commit", "feature/conflict"])
            .output()?;
        if merge.status.success() {
            return Err("fixture merge unexpectedly succeeded".into());
        }
        std::fs::write(source.join("conflict"), "accepted\n")?;
        run_git(&source, &global, &["add", "conflict"])?;
        run_git(
            &source,
            &global,
            &["commit", "-q", "-m", "accepted conflict"],
        )?;
        run_git(&source, &global, &["push", "-q", "-u", "origin", "qa"])?;
        run_git(&source, &global, &["checkout", "-q", "main"])?;
        Ok(Self {
            _directory: directory,
            source,
            remote,
            global,
            cache,
        })
    }

    fn preview(&self) -> Result<std::process::Output, Box<dyn Error>> {
        Ok(gd_command()?
            .current_dir(&self.source)
            .args([
                "restack",
                "qa",
                "--main",
                "main",
                "--params",
                r#"{"removeBranches":[]}"#,
            ])
            .env("GIT_CONFIG_GLOBAL", &self.global)
            .env("XDG_CACHE_HOME", &self.cache)
            .output()?)
    }

    fn resume(
        &self,
        token: &str,
        environment: &str,
        source: &Path,
    ) -> Result<std::process::Output, Box<dyn Error>> {
        Ok(gd_command()?
            .current_dir(source)
            .args(["restack", environment, "--resume", token])
            .env("GIT_CONFIG_GLOBAL", &self.global)
            .env("XDG_CACHE_HOME", &self.cache)
            .output()?)
    }

    fn seal_manual_resolution(
        &self,
    ) -> Result<(String, PathBuf, serde_json::Value), Box<dyn Error>> {
        self.advance_main()?;
        let conflict = structured_restack_error(self.preview()?)?;
        let token = conflict["details"]["resumeToken"]
            .as_str()
            .ok_or("resume token")?
            .to_owned();
        let work_area = PathBuf::from(
            conflict["details"]["workArea"]
                .as_str()
                .ok_or("work area")?,
        );
        std::fs::write(work_area.join("conflict"), "manual resolution\n")?;
        self.git(&work_area, &["add", "conflict"])?;
        let output = self.resume(&token, "qa", &self.source)?;
        if !output.status.success() {
            return Err(format!(
                "restack resume failed: {}",
                String::from_utf8_lossy(&output.stderr)
            )
            .into());
        }
        let plan = serde_json::from_slice(&output.stdout)?;
        Ok((token, work_area, plan))
    }

    fn resume_apply(&self, token: &str) -> Result<std::process::Output, Box<dyn Error>> {
        Ok(gd_command()?
            .current_dir(&self.source)
            .args(["restack", "qa", "--resume", token, "--apply"])
            .env("GIT_CONFIG_GLOBAL", &self.global)
            .env("XDG_CACHE_HOME", &self.cache)
            .output()?)
    }

    #[cfg(unix)]
    fn resume_apply_with_path(
        &self,
        token: &str,
        path: &std::ffi::OsStr,
    ) -> Result<std::process::Output, Box<dyn Error>> {
        Ok(gd_command()?
            .current_dir(&self.source)
            .args(["restack", "qa", "--resume", token, "--apply"])
            .env("GIT_CONFIG_GLOBAL", &self.global)
            .env("XDG_CACHE_HOME", &self.cache)
            .env("PATH", path)
            .output()?)
    }

    fn abort(&self, token: &str) -> Result<std::process::Output, Box<dyn Error>> {
        Ok(gd_command()?
            .current_dir(&self.source)
            .args(["restack", "qa", "--resume", token, "--abort"])
            .env("GIT_CONFIG_GLOBAL", &self.global)
            .env("XDG_CACHE_HOME", &self.cache)
            .output()?)
    }

    fn advance_main(&self) -> Result<(), Box<dyn Error>> {
        std::fs::write(self.source.join("conflict"), "main new\n")?;
        self.git(&self.source, &["commit", "-q", "-am", "main new"])?;
        self.git(&self.source, &["push", "-q", "origin", "main"])
    }

    fn git(&self, path: &Path, arguments: &[&str]) -> Result<(), Box<dyn Error>> {
        run_git(path, &self.global, arguments)
    }

    fn git_text(&self, path: &Path, arguments: &[&str]) -> Result<String, Box<dyn Error>> {
        git_text(path, &self.global, arguments)
    }
}

fn run_git(path: &Path, global: &Path, arguments: &[&str]) -> Result<(), Box<dyn Error>> {
    let status = isolated_git(global)
        .args(arguments)
        .current_dir(path)
        .status()?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("git {} failed with {status}", arguments.join(" ")).into())
    }
}

fn git_text(path: &Path, global: &Path, arguments: &[&str]) -> Result<String, Box<dyn Error>> {
    let output = isolated_git(global)
        .args(arguments)
        .current_dir(path)
        .output()?;
    if !output.status.success() {
        return Err(format!("git {} failed with {}", arguments.join(" "), output.status).into());
    }
    Ok(String::from_utf8(output.stdout)?
        .trim_end_matches(['\r', '\n'])
        .to_owned())
}

fn isolated_git(global: &Path) -> ProcessCommand {
    let mut command = ProcessCommand::new("git");
    for variable in [
        "GIT_ALTERNATE_OBJECT_DIRECTORIES",
        "GIT_COMMON_DIR",
        "GIT_CONFIG",
        "GIT_CONFIG_COUNT",
        "GIT_DIR",
        "GIT_GRAFT_FILE",
        "GIT_INDEX_FILE",
        "GIT_OBJECT_DIRECTORY",
        "GIT_PREFIX",
        "GIT_QUARANTINE_PATH",
        "GIT_SHALLOW_FILE",
        "GIT_WORK_TREE",
    ] {
        command.env_remove(variable);
    }
    command
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", global)
        .arg("-c")
        .arg("core.fsmonitor=false");
    command
}

fn path_text(path: &Path) -> Result<&str, Box<dyn Error>> {
    path.to_str()
        .ok_or_else(|| "test path is not valid UTF-8".into())
}

#[cfg(unix)]
fn find_git_executable() -> Result<PathBuf, Box<dyn Error>> {
    let path = std::env::var_os("PATH").ok_or("PATH")?;
    for directory in std::env::split_paths(&path) {
        let candidate = directory.join(format!("git{}", std::env::consts::EXE_SUFFIX));
        if candidate.is_file() {
            return Ok(candidate);
        }
    }
    Err("git executable was not found on PATH".into())
}

#[cfg(unix)]
fn make_executable(path: &Path) -> Result<(), Box<dyn Error>> {
    use std::os::unix::fs::PermissionsExt;

    let mut permissions = std::fs::metadata(path)?.permissions();
    permissions.set_mode(0o700);
    std::fs::set_permissions(path, permissions)?;
    Ok(())
}

#[cfg(not(unix))]
fn make_executable(_path: &Path) -> Result<(), Box<dyn Error>> {
    Ok(())
}
