//! Resumed session safety and abort tests.

use std::error::Error;
use std::path::PathBuf;

use crate::common::{
    expire_session, session_token_secret, sign_session_envelope, structured_restack_error,
};
use crate::conflict_fixture::ConflictRestackFixture;
#[cfg(unix)]
use crate::git::find_git_executable;
use crate::git::{make_executable, path_text, run_git};

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

#[test]
fn restack_resume_reviews_a_sealed_session_again_without_changing_it() -> Result<(), Box<dyn Error>>
{
    let fixture = ConflictRestackFixture::new()?;
    let (token, _work_area, plan) = fixture.seal_manual_resolution()?;

    let again = fixture.resume(&token, "qa", &fixture.source)?;
    assert!(
        again.status.success(),
        "{}",
        String::from_utf8_lossy(&again.stderr)
    );
    let replayed: serde_json::Value = serde_json::from_slice(&again.stdout)?;
    assert_eq!(replayed, plan);

    let applied = fixture.resume_apply(&token)?;
    assert!(
        applied.status.success(),
        "{}",
        String::from_utf8_lossy(&applied.stderr)
    );
    Ok(())
}
