//! Resumable restack session tests.

use std::error::Error;
use std::path::PathBuf;

use crate::common::structured_restack_error;
use crate::conflict_fixture::ConflictRestackFixture;

#[test]
fn restack_resume_rejects_a_resolution_that_leaves_conflict_markers() -> Result<(), Box<dyn Error>>
{
    let fixture = ConflictRestackFixture::new()?;
    fixture.advance_main()?;
    let conflict = fixture.preview()?;
    let error = structured_restack_error(conflict)?;
    assert_eq!(error["code"], "reconstruction_conflict");
    let token = error["details"]["resumeToken"]
        .as_str()
        .ok_or("resume token")?;
    let work_area = PathBuf::from(error["details"]["workArea"].as_str().ok_or("work area")?);

    std::fs::write(
        work_area.join("conflict"),
        "<<<<<<< HEAD\nours\n=======\ntheirs\n>>>>>>> feature/conflict\n",
    )?;
    fixture.git(&work_area, &["add", "conflict"])?;
    let output = fixture.resume(token, "qa", &fixture.source)?;

    assert!(output.stdout.is_empty());
    let error = structured_restack_error(output)?;
    assert_eq!(error["code"], "reconstruction_failed");
    assert_eq!(error["details"]["stage"], "stagedDiffCheck");
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
