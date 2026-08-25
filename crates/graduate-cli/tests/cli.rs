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
            "Inspect Jira Cloud from the terminal\n\n{usage}"
        )))
        .stdout(predicate::str::contains(usage))
        .stdout(predicate::str::contains("tui").not())
        .stdout(predicate::str::contains(
            "auth             Configure authentication for a ticket system",
        ))
        .stdout(predicate::str::contains("restack").not())
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

#[test]
fn restack_preview_is_isolated_and_emits_canonical_machine_json() -> Result<(), Box<dyn Error>> {
    let fixture = RestackFixture::new()?;
    let source_head = fixture.git_text(&fixture.source, &["rev-parse", "HEAD"])?;
    let local_environment = fixture.git_text(&fixture.source, &["rev-parse", "refs/heads/qa"])?;
    let remote_environment = fixture.git_text(&fixture.remote, &["rev-parse", "refs/heads/qa"])?;
    let remote_environment_tree =
        fixture.git_text(&fixture.remote, &["rev-parse", "refs/heads/qa^{tree}"])?;
    let main_tip = fixture.git_text(&fixture.source, &["rev-parse", "refs/remotes/origin/main"])?;
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

    let output = fixture.preview(&[])?;

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
        .output()?;
    assert_eq!(invalid.status.code(), Some(2));
    let invalid_error: serde_json::Value = serde_json::from_slice(&invalid.stderr)?;
    assert_eq!(invalid_error["kind"], "restackError");
    assert_eq!(invalid_error["schemaVersion"], 1);
    assert_eq!(invalid_error["code"], "invalid_params");

    let clap_invalid = gd_command()?
        .args(["restack", "--params", r#"{"removeBranches":[]}"#])
        .output()?;
    assert_eq!(clap_invalid.status.code(), Some(2));
    let clap_error: serde_json::Value = serde_json::from_slice(&clap_invalid.stderr)?;
    assert_eq!(clap_error["code"], "invalid_usage");

    let fetch = gd_command()?
        .current_dir(&source)
        .args(["restack", "qa", "--params", r#"{"removeBranches":[]}"#])
        .env("GIT_PAT", "pat-secret-do-not-print")
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

struct RestackFixture {
    directory: tempfile::TempDir,
    source: PathBuf,
    remote: PathBuf,
    global: PathBuf,
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
        run_git(&source, &global, &["add", "base"])?;
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
            directory,
            source,
            remote,
            global,
        })
    }

    fn preview(&self, removals: &[&str]) -> Result<std::process::Output, Box<dyn Error>> {
        let params = serde_json::json!({"removeBranches": removals}).to_string();
        Ok(gd_command()?
            .current_dir(&self.source)
            .args(["restack", "qa", "--main", "main", "--params", &params])
            .env("GIT_CONFIG_GLOBAL", &self.global)
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
