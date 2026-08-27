//! Tests.

use std::path::Path;

use super::graph::*;
use super::refs::*;
use super::*;

#[test]
fn ref_components_reject_encoded_octets() {
    for value in ["%2e%2e/qa", "%2E%2E/qa", "qa%2fchild", "%252e%252e/qa"] {
        assert!(validate_ref_component("branch", value).is_err());
    }
    assert!(validate_ref_component("branch", "feature/100%-complete").is_ok());
}

#[test]
fn git_inspection_builds_a_first_merge_ordered_restack_snapshot(
) -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    run_git(directory.path(), &["init", "-q", "-b", "main"])?;
    run_git(directory.path(), &["config", "user.name", "Test Author"])?;
    run_git(
        directory.path(),
        &["config", "user.email", "test@example.com"],
    )?;
    std::fs::write(directory.path().join("base"), "base\n")?;
    run_git(directory.path(), &["add", "base"])?;
    run_git(directory.path(), &["commit", "-q", "-m", "base"])?;
    run_git(
        directory.path(),
        &["checkout", "-q", "-b", "feature/zeta", "main"],
    )?;
    std::fs::write(directory.path().join("zeta-one"), "one\n")?;
    run_git(directory.path(), &["add", "zeta-one"])?;
    run_git(directory.path(), &["commit", "-q", "-m", "zeta one"])?;
    run_git(directory.path(), &["checkout", "-q", "-b", "qa", "main"])?;
    run_git(
        directory.path(),
        &["merge", "-q", "--no-ff", "feature/zeta", "-m", "zeta one"],
    )?;
    run_git(directory.path(), &["checkout", "-q", "feature/zeta"])?;
    std::fs::write(directory.path().join("zeta-two"), "two\n")?;
    run_git(directory.path(), &["add", "zeta-two"])?;
    run_git(directory.path(), &["commit", "-q", "-m", "zeta two"])?;
    run_git(directory.path(), &["checkout", "-q", "qa"])?;
    run_git(
        directory.path(),
        &["merge", "-q", "--no-ff", "feature/zeta", "-m", "zeta two"],
    )?;
    run_git(
        directory.path(),
        &["commit", "-q", "--allow-empty", "-m", "### Match 'qa'"],
    )?;
    run_git(
        directory.path(),
        &["checkout", "-q", "-b", "feature/alpha", "main"],
    )?;
    std::fs::write(directory.path().join("alpha"), "alpha\n")?;
    run_git(directory.path(), &["add", "alpha"])?;
    run_git(directory.path(), &["commit", "-q", "-m", "alpha"])?;
    run_git(directory.path(), &["checkout", "-q", "qa"])?;
    run_git(
        directory.path(),
        &["merge", "-q", "--no-ff", "feature/alpha", "-m", "alpha"],
    )?;
    for branch in ["main", "qa", "feature/zeta", "feature/alpha"] {
        run_git(
            directory.path(),
            &[
                "update-ref",
                &format!("refs/remotes/origin/{branch}"),
                &format!("refs/heads/{branch}"),
            ],
        )?;
    }
    run_git(
        directory.path(),
        &[
            "symbolic-ref",
            "refs/remotes/origin/HEAD",
            "refs/remotes/origin/main",
        ],
    )?;
    let repository = gix::discover(directory.path())?;
    let inspection = inspect_environment(&repository, "origin", "qa", None)?;

    let snapshot = restack_snapshot(&repository, &inspection)?;

    assert_eq!(
        snapshot
            .features
            .iter()
            .map(|feature| feature.name.as_str())
            .collect::<Vec<_>>(),
        ["feature/zeta", "feature/alpha"]
    );
    assert_eq!(snapshot.features[0].historical_merges.len(), 2);
    assert_eq!(snapshot.dropped_markers.len(), 1);
    assert_eq!(snapshot.attributed_commits.len(), 3);
    Ok(())
}

#[test]
fn git_inspection_keeps_the_graph_when_history_cannot_be_read(
) -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    run_git(directory.path(), &["init", "-q", "-b", "main"])?;
    run_git(directory.path(), &["config", "user.name", "Test Author"])?;
    run_git(
        directory.path(),
        &["config", "user.email", "test@example.com"],
    )?;
    std::fs::write(directory.path().join("base"), "base\n")?;
    run_git(directory.path(), &["add", "base"])?;
    run_git(directory.path(), &["commit", "-q", "-m", "base"])?;
    // feature/a with an internal pull-merge, then the environment is the
    // feature's own history plus direct work: a pull-merge on the spine.
    run_git(
        directory.path(),
        &["checkout", "-q", "-b", "feature/a", "main"],
    )?;
    std::fs::write(directory.path().join("a-one"), "one\n")?;
    run_git(directory.path(), &["add", "a-one"])?;
    run_git(directory.path(), &["commit", "-q", "-m", "a one"])?;
    run_git(
        directory.path(),
        &["checkout", "-q", "-b", "a-side", "main"],
    )?;
    std::fs::write(directory.path().join("a-side"), "side\n")?;
    run_git(directory.path(), &["add", "a-side"])?;
    run_git(directory.path(), &["commit", "-q", "-m", "a side"])?;
    run_git(directory.path(), &["checkout", "-q", "feature/a"])?;
    run_git(
        directory.path(),
        &[
            "merge",
            "-q",
            "--no-ff",
            "a-side",
            "-m",
            "Merge branch 'feature/a' into feature/a",
        ],
    )?;
    run_git(directory.path(), &["branch", "-q", "-D", "a-side"])?;
    run_git(
        directory.path(),
        &["checkout", "-q", "-b", "feature/b", "feature/a"],
    )?;
    std::fs::write(directory.path().join("b-one"), "b\n")?;
    run_git(directory.path(), &["add", "b-one"])?;
    run_git(directory.path(), &["commit", "-q", "-m", "b one"])?;
    run_git(
        directory.path(),
        &["checkout", "-q", "-b", "qa", "feature/a"],
    )?;
    run_git(
        directory.path(),
        &["merge", "-q", "--no-ff", "feature/b", "-m", "merge b"],
    )?;
    std::fs::write(directory.path().join("stray"), "stray\n")?;
    run_git(directory.path(), &["add", "stray"])?;
    run_git(directory.path(), &["commit", "-q", "-m", "direct work"])?;
    for branch in ["main", "qa", "feature/a", "feature/b"] {
        run_git(
            directory.path(),
            &[
                "update-ref",
                &format!("refs/remotes/origin/{branch}"),
                &format!("refs/heads/{branch}"),
            ],
        )?;
    }
    run_git(
        directory.path(),
        &[
            "symbolic-ref",
            "refs/remotes/origin/HEAD",
            "refs/remotes/origin/main",
        ],
    )?;
    let repository = gix::discover(directory.path())?;
    let inspection = inspect_environment(&repository, "origin", "qa", None)?;

    let Err(RestackInspectionError::Unsupported { error, graph }) =
        restack_snapshot(&repository, &inspection)
    else {
        return Err("expected the history proof to fail".into());
    };
    let InventoryError::AmbiguousFeatureRefs { branches, .. } = error else {
        return Err(format!("expected an ambiguous merge, got {error}").into());
    };
    assert_eq!(branches, ["feature/a", "feature/b"]);
    let tips = tip_timestamps(&repository, &graph)?;
    let candidates = graph
        .feature_refs
        .iter()
        .filter(|feature| tips.contains_key(&feature.tip))
        .map(|feature| feature.name.as_str())
        .collect::<Vec<_>>();
    assert_eq!(candidates, ["feature/a", "feature/b"]);
    let stray = graph
        .environment_ancestors
        .difference(&graph.main_ancestors)
        .find(|id| {
            graph
                .commits
                .get(*id)
                .is_some_and(|commit| commit.message.trim() == "direct work")
        })
        .ok_or("stray commit missing from graph")?;
    let rows = commit_rows(&repository, [stray])?;
    let row = rows.get(stray).ok_or("row missing")?;
    assert_eq!(row.subject, "direct work");
    assert_eq!(row.author, "Test Author");
    assert_eq!(row.date.len(), 10);
    Ok(())
}

fn run_git(path: &Path, arguments: &[&str]) -> Result<(), Box<dyn std::error::Error>> {
    let status = isolated_git_command()
        .args(["-c", "core.fsmonitor=false"])
        .args(arguments)
        .current_dir(path)
        .status()?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("git {} failed with {status}", arguments.join(" ")).into())
    }
}
