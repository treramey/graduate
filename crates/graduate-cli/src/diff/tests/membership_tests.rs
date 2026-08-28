use super::*;

/// `feature/a` is merged into `qa` and then extended by one commit;
/// `feature/b` merges `qa` back into itself; `feature/c` is fully merged.
fn membership_fixture(path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    init_repository(path)?;
    run_git(path, &["checkout", "-q", "-b", "feature/a"])?;
    commit_file(path, "a", "a\n", "a one", "2024-02-01T00:00:00Z")?;
    run_git(path, &["checkout", "-q", "-b", "feature/c", "main"])?;
    commit_file(path, "c", "c\n", "c one", "2024-02-01T00:00:00Z")?;
    run_git(path, &["checkout", "-q", "-b", "qa", "main"])?;
    run_git(
        path,
        &["merge", "-q", "--no-ff", "feature/a", "-m", "promote a"],
    )?;
    run_git(
        path,
        &["merge", "-q", "--no-ff", "feature/c", "-m", "promote c"],
    )?;
    run_git(path, &["checkout", "-q", "feature/a"])?;
    commit_file(path, "a", "a\na2\n", "a two", "2024-02-02T00:00:00Z")?;
    run_git(path, &["checkout", "-q", "-b", "feature/b", "main"])?;
    commit_file(path, "b", "b\n", "b one", "2024-02-03T00:00:00Z")?;
    run_git(path, &["merge", "-q", "--no-ff", "qa", "-m", "sync qa"])?;
    run_git(path, &["checkout", "-q", "qa"])?;
    run_git(
        path,
        &["merge", "-q", "--no-ff", "feature/b", "-m", "promote b"],
    )?;
    publish(path, &["main", "qa", "feature/a", "feature/b", "feature/c"])
}

#[test]
fn extended_branches_report_tips_outside_the_environment() -> Result<(), Box<dyn std::error::Error>>
{
    let directory = tempfile::tempdir()?;
    membership_fixture(directory.path())?;

    let rows = measured_rows(&scan_options(directory.path(), "qa"))?;
    let row = |name: &str| rows.iter().find(|row| row.branch == name);

    let a = row("feature/a").ok_or("feature/a missing")?;
    assert!(!a.tip_in_environment);
    assert_eq!(a.unmerged_ahead, 1);
    assert_eq!(a.ahead, 2);
    assert_eq!(a.tip.len(), 40);
    let c = row("feature/c").ok_or("feature/c missing")?;
    assert!(c.tip_in_environment);
    assert_eq!(c.unmerged_ahead, 0);
    assert_eq!(c.absorbed_environment_merges, 0);
    Ok(())
}

#[test]
fn branches_that_merged_the_environment_count_absorbed_merges(
) -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    membership_fixture(directory.path())?;

    let rows = measured_rows(&scan_options(directory.path(), "qa"))?;
    let b = rows
        .iter()
        .find(|row| row.branch == "feature/b")
        .ok_or("feature/b missing")?;
    assert_eq!(b.merged_environments, vec!["qa".to_owned()]);
    // `sync qa` reaches both environment merges (`promote a`, `promote c`).
    assert_eq!(b.absorbed_environment_merges, 2);
    assert!(b.tip_in_environment);
    assert_eq!(b.unmerged_ahead, 0);
    let a = rows
        .iter()
        .find(|row| row.branch == "feature/a")
        .ok_or("feature/a missing")?;
    assert_eq!(a.absorbed_environment_merges, 0);
    Ok(())
}
