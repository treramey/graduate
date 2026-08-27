//! Restack publication revalidation and failure tests.

use std::error::Error;

use crate::common::{gd_command, isolate_gd_storage, structured_restack_error};
use crate::fixture::RestackFixture;
use crate::git::{find_git_executable, make_executable, path_text, run_git};

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
    assert_eq!(structured_restack_error(output)?["code"], "push_rejected");
    assert_eq!(
        fixture.git_text(&fixture.remote, &["rev-parse", "refs/heads/qa"])?,
        race_oid
    );
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

    let cache = directory.path().join("cache");
    let mut invalid_command = gd_command()?;
    isolate_gd_storage(&mut invalid_command, &cache);
    let invalid = invalid_command
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
    assert_eq!(invalid_error["schemaVersion"], 2);
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

    let mut fetch_command = gd_command()?;
    isolate_gd_storage(&mut fetch_command, &cache);
    let fetch = fetch_command
        .current_dir(&source)
        .args([
            "restack",
            "qa",
            "--params",
            r#"{"removeBranches":[],"planDigest":"0000000000000000000000000000000000000000000000000000000000000000"}"#,
            "--apply",
        ])
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
