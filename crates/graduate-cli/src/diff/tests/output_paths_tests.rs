use super::super::output::validate_output_path;
use super::*;

#[test]
fn output_rejects_paths_outside_the_current_directory() {
    assert!(validate_output_path(Path::new("../report.json")).is_err());
    assert!(validate_output_path(Path::new("/tmp/report.json")).is_err());
}

#[cfg(unix)]
#[test]
fn output_uses_the_canonical_parent_for_internal_symlinks() -> Result<(), Box<dyn std::error::Error>>
{
    use std::os::unix::fs::symlink;

    let current = std::env::current_dir()?;
    let directory = tempfile::tempdir_in(&current)?;
    let reports = directory.path().join("reports");
    std::fs::create_dir(&reports)?;
    symlink(&reports, directory.path().join("report-link"))?;
    let relative = directory
        .path()
        .strip_prefix(&current)?
        .join("report-link/qa.json");

    let output = validate_output_path(&relative)?;

    assert_eq!(output.destination, reports.canonicalize()?.join("qa.json"));
    Ok(())
}

#[cfg(unix)]
#[test]
fn output_rejects_a_parent_symlink_that_escapes_the_repository(
) -> Result<(), Box<dyn std::error::Error>> {
    use std::os::unix::fs::symlink;

    let current = std::env::current_dir()?;
    let directory = tempfile::tempdir_in(&current)?;
    let outside = tempfile::tempdir()?;
    symlink(outside.path(), directory.path().join("outside"))?;
    let relative = directory
        .path()
        .strip_prefix(&current)?
        .join("outside/report.json");

    assert!(validate_output_path(&relative).is_err());
    Ok(())
}
