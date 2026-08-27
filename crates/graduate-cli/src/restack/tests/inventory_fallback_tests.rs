use std::collections::BTreeMap;
use std::fs;

use graduate::restack::{
    build_plan, select_features, InventoryMode, MergeOutcome, MergeResolution, Reconstruction,
    RemoteEndpointIdentity, RestackAuthor, RESTACK_SCHEMA_VERSION,
};

use super::super::errors::session_error;
use super::super::interactive_steps::{inventory_fallback, orphan_rows};
use super::super::resume::sealed_session_plan;
use super::*;
use crate::restack::session::{SessionConflict, SessionError, SessionMetadata, SessionStatus};
use crate::shared::environment_git::{
    inspect_environment, restack_snapshot, RestackInspectionError,
};

#[test]
fn inventory_fallback_on_a_real_repository_matches_git_rev_list(
) -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let git = |arguments: &[&str]| -> Result<(), Box<dyn std::error::Error>> {
        let status = crate::shared::environment_git::isolated_git_command()
            .args(["-c", "core.fsmonitor=false"])
            .args(arguments)
            .current_dir(root)
            .status()?;
        if status.success() {
            Ok(())
        } else {
            Err(format!("git {} failed", arguments.join(" ")).into())
        }
    };
    let git_lines = |arguments: &[&str]| -> Result<BTreeSet<String>, Box<dyn std::error::Error>> {
        let output = crate::shared::environment_git::isolated_git_command()
            .args(arguments)
            .current_dir(root)
            .output()?;
        if !output.status.success() {
            return Err(format!("git {} failed", arguments.join(" ")).into());
        }
        Ok(String::from_utf8(output.stdout)?
            .lines()
            .map(str::to_owned)
            .collect())
    };
    git(&["init", "-q", "-b", "main"])?;
    git(&["config", "user.name", "Test Author"])?;
    git(&["config", "user.email", "test@example.com"])?;
    fs::write(root.join("base"), "base\n")?;
    git(&["add", "base"])?;
    git(&["commit", "-q", "-m", "base"])?;
    git(&["checkout", "-q", "-b", "feature/a", "main"])?;
    fs::write(root.join("a-one"), "one\n")?;
    git(&["add", "a-one"])?;
    git(&["commit", "-q", "-m", "a one"])?;
    git(&["checkout", "-q", "-b", "a-side", "main"])?;
    fs::write(root.join("a-side"), "side\n")?;
    git(&["add", "a-side"])?;
    git(&["commit", "-q", "-m", "a side"])?;
    git(&["checkout", "-q", "feature/a"])?;
    git(&["merge", "-q", "--no-ff", "a-side", "-m", "pull merge"])?;
    git(&["branch", "-q", "-D", "a-side"])?;
    git(&["checkout", "-q", "-b", "feature/b", "feature/a"])?;
    fs::write(root.join("b-one"), "b\n")?;
    git(&["add", "b-one"])?;
    git(&["commit", "-q", "-m", "b one"])?;
    git(&["checkout", "-q", "-b", "qa", "feature/a"])?;
    git(&["merge", "-q", "--no-ff", "feature/b", "-m", "merge b"])?;
    fs::write(root.join("stray"), "stray\n")?;
    git(&["add", "stray"])?;
    git(&["commit", "-q", "-m", "direct work"])?;
    fs::write(root.join("stray-two"), "two\n")?;
    git(&["add", "stray-two"])?;
    git(&["commit", "-q", "-m", "more direct work"])?;
    for branch in ["main", "qa", "feature/a", "feature/b"] {
        git(&[
            "update-ref",
            &format!("refs/remotes/origin/{branch}"),
            &format!("refs/heads/{branch}"),
        ])?;
    }
    let repository = gix::discover(root)?;
    let inspection = inspect_environment(&repository, "origin", "qa", Some("main"))?;
    let Err(RestackInspectionError::Unsupported { error, graph }) =
        restack_snapshot(&repository, &inspection)
    else {
        return Err("expected the history proof to fail".into());
    };

    let (snapshot, rows) = inventory_fallback(&repository, error, &graph)?;

    assert_eq!(snapshot.inventory_mode, InventoryMode::Reachability);
    assert_eq!(
        snapshot
            .features
            .iter()
            .map(|feature| feature.name.as_str())
            .collect::<Vec<_>>(),
        ["feature/b"]
    );
    assert_eq!(snapshot.carried_features[0].name, "feature/a");
    let retained = select_features(&snapshot, &[])?.retained;
    let orphans = orphan_rows(&snapshot, &rows, &retained)?;
    let mut expected = git_lines(&["rev-list", "--no-merges", "main..qa"])?;
    for kept in &retained {
        for reached in git_lines(&["rev-list", "--no-merges", &format!("main..{}", kept.tip)])? {
            expected.remove(&reached);
        }
    }
    assert_eq!(
        orphans
            .iter()
            .map(|orphan| orphan.commit.clone())
            .collect::<BTreeSet<_>>(),
        expected
    );
    let mut subjects = orphans
        .iter()
        .map(|orphan| orphan.subject.as_str())
        .collect::<Vec<_>>();
    subjects.sort_unstable();
    assert_eq!(subjects, ["direct work", "more direct work"]);
    assert!(orphans.iter().all(|orphan| orphan.author == "Test Author"));

    let missing = orphan_rows(&snapshot, &BTreeMap::new(), &retained)
        .err()
        .ok_or("expected a missing row to fail closed")?;
    let CliError::Machine(missing) = missing else {
        return Err("expected a machine failure".into());
    };
    assert_eq!(missing.code, "inspection_failed");
    assert!(missing.to_string().contains(r#""stage":"orphans""#));
    Ok(())
}

#[test]
fn conflicted_inventory_session_round_trips_orphaned_commits_into_the_sealed_plan(
) -> Result<(), Box<dyn std::error::Error>> {
    let (snapshot, orphans) = reachability_plan_inputs();
    let selection = select_features(&snapshot, &[])?;
    let author = RestackAuthor {
        name: "Pat".to_owned(),
        email: "pat@example.com".to_owned(),
    };
    let endpoints = RemoteEndpointIdentity {
        fetch_sha256: "f".repeat(64),
        push_sha256: "p".repeat(64),
    };
    let merges = vec![MergeOutcome {
        branch: "feature/b".to_owned(),
        tip: "b".to_owned(),
        commit: "preview".to_owned(),
        tree: "tree".to_owned(),
        resolution: MergeResolution::Manual,
    }];
    let expected = build_plan(
        snapshot.clone(),
        endpoints.clone(),
        author.clone(),
        selection.clone(),
        Reconstruction {
            merges: merges.clone(),
            final_tree: "tree".to_owned(),
            preview_commit: "preview".to_owned(),
        },
        orphans.clone(),
    )?;

    let directory = tempfile::tempdir()?;
    let store =
        SessionStore::open_root(directory.path().join("sessions")).map_err(session_error)?;
    let mut draft = store.begin().map_err(session_error)?;
    let metadata = SessionMetadata::conflicted(
        "repository".to_owned(),
        snapshot,
        endpoints,
        author,
        selection,
        orphans.clone(),
        SessionConflict {
            merges: Vec::new(),
            next_feature: 0,
            expected_head: "base".to_owned(),
            expected_head_reflog: "reflog".to_owned(),
            expected_feature_tip: "b".to_owned(),
        },
    )
    .map_err(session_error)?;
    draft.save(&metadata).map_err(session_error)?;
    let token = draft.token();
    drop(draft);

    let mut handle = store.resume(&token).map_err(session_error)?;
    assert_eq!(handle.metadata.orphaned_commits, orphans);
    handle.metadata.merges = merges;
    handle.metadata.next_feature = 1;
    handle.metadata.expected_feature_tip = None;
    handle.metadata.expected_head = "preview".to_owned();
    handle.metadata.final_tree = Some("tree".to_owned());
    handle.metadata.preview_commit = Some("preview".to_owned());
    handle.metadata.plan_digest = Some(expected.digest.clone());
    handle.metadata.status = SessionStatus::Sealed;
    handle.save().map_err(session_error)?;
    let plan = sealed_session_plan(&handle.metadata)?;
    assert_eq!(plan.orphaned_commits, orphans);
    assert_eq!(plan.digest, expected.digest);

    let mut without_rows = handle.metadata.clone();
    without_rows.orphaned_commits.clear();
    let Err(CliError::Machine(error)) = sealed_session_plan(&without_rows) else {
        return Err("a sealed plan without its orphan rows must not rebuild".into());
    };
    assert_eq!(error.code, "validation_failed");

    handle.metadata.schema_version = 1;
    handle.save().map_err(session_error)?;
    drop(handle);
    assert_eq!(
        store.resume(&token).err(),
        Some(SessionError::SchemaMismatch {
            found: 1,
            expected: RESTACK_SCHEMA_VERSION,
        })
    );
    let mapped = session_error(SessionError::SchemaMismatch {
        found: 1,
        expected: RESTACK_SCHEMA_VERSION,
    });
    let CliError::Machine(mapped) = mapped else {
        return Err("expected a machine failure".into());
    };
    assert_eq!(mapped.code, "session_schema_mismatch");
    Ok(())
}
