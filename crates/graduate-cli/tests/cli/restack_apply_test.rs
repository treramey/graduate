//! Machine restack publication tests.

use std::error::Error;

use crate::common::structured_restack_error;
use crate::fixture::RestackFixture;
use crate::git::{make_executable, path_text, run_git};

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
