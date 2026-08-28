//! Restack preview input, fetch, and inventory tests.

use std::error::Error;

use crate::common::structured_restack_error;
use crate::fixture::RestackFixture;

#[test]
fn restack_machine_preview_never_falls_back_to_the_reachability_inventory(
) -> Result<(), Box<dyn Error>> {
    let fixture = RestackFixture::new()?;
    fixture.git(&fixture.source, &["checkout", "-q", "qa"])?;
    std::fs::write(fixture.source.join("hotfix"), "direct work\n")?;
    fixture.git(&fixture.source, &["add", "hotfix"])?;
    fixture.git(
        &fixture.source,
        &["commit", "-q", "-m", "direct hotfix on qa"],
    )?;
    fixture.git(&fixture.source, &["push", "-q", "origin", "qa"])?;
    fixture.git(&fixture.source, &["checkout", "-q", "main"])?;

    for arguments in [
        vec!["restack", "qa", "--main", "main", "--dry-run"],
        vec![
            "restack",
            "qa",
            "--main",
            "main",
            "--params",
            r#"{"removeBranches":[]}"#,
        ],
    ] {
        let output = fixture
            .command()?
            .current_dir(&fixture.source)
            .args(&arguments)
            .env("GIT_CONFIG_GLOBAL", &fixture.global)
            .output()?;
        assert!(output.stdout.is_empty());
        let error = structured_restack_error(output)?;
        assert_eq!(error["kind"], "restackError");
        assert_eq!(error["schemaVersion"], 3);
        assert_eq!(error["code"], "unsupported_history");
        assert_eq!(error["details"]["kind"], "directCommit");
        assert!(error["details"].get("fallback").is_none());
        assert!(error.get("inventory").is_none());
    }
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
