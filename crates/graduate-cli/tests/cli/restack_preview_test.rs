//! Machine restack preview tests.

use std::error::Error;

use crate::fixture::RestackFixture;
use crate::git::{make_executable, path_text};

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
    assert_eq!(plan["schemaVersion"], 3);
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
    let output = fixture
        .command()?
        .current_dir(&fixture.source)
        .args(["restack", "qa", "--main", "main", "--dry-run"])
        .env("GIT_CONFIG_GLOBAL", &fixture.global)
        .output()?;

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let plan: serde_json::Value = serde_json::from_slice(&output.stdout)?;
    assert_eq!(plan["kind"], "restackPlan");
    assert_eq!(plan["schemaVersion"], 3);
    assert_eq!(plan["retainedBranches"][0]["name"], "feature/a");
    assert_eq!(plan["removedBranches"], serde_json::json!([]));
    assert_eq!(
        plan["inventory"],
        serde_json::json!({"mode": "history", "reason": null})
    );
    assert_eq!(plan["carriedBranches"], serde_json::json!([]));
    assert_eq!(plan["orphanedCommits"], serde_json::json!([]));
    assert_eq!(plan["effects"]["reusedResolutions"], true);
    assert_eq!(plan["effects"]["pushed"], false);
    Ok(())
}

#[test]
fn restack_reconstructs_features_whose_content_has_whitespace_errors() -> Result<(), Box<dyn Error>>
{
    let fixture = RestackFixture::new()?;
    fixture.git(
        &fixture.source,
        &["checkout", "-q", "-b", "feature/ws", "main"],
    )?;
    std::fs::write(
        fixture.source.join("sloppy.cs"),
        "class Sloppy {   \n\tint x;  \n}\n\n\n",
    )?;
    fixture.git(&fixture.source, &["add", "sloppy.cs"])?;
    fixture.git(
        &fixture.source,
        &["commit", "-q", "-m", "trailing whitespace"],
    )?;
    fixture.git(
        &fixture.source,
        &["push", "-q", "-u", "origin", "feature/ws"],
    )?;
    fixture.git(&fixture.source, &["checkout", "-q", "qa"])?;
    fixture.git(
        &fixture.source,
        &[
            "merge",
            "-q",
            "--no-ff",
            "feature/ws",
            "-m",
            "accepted feature ws",
        ],
    )?;
    fixture.git(&fixture.source, &["push", "-q", "origin", "qa"])?;
    fixture.git(&fixture.source, &["checkout", "-q", "main"])?;

    let output = fixture.preview(&[])?;

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let plan: serde_json::Value = serde_json::from_slice(&output.stdout)?;
    assert_eq!(plan["merges"][1]["branch"], "feature/ws");
    assert_eq!(plan["merges"][1]["outcome"], "clean");
    Ok(())
}
