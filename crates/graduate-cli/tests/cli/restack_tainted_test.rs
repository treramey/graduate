use std::error::Error;

use crate::fixture::RestackFixture;

#[test]
fn dry_run_removes_features_that_merged_the_environment() -> Result<(), Box<dyn Error>> {
    let fixture = RestackFixture::new()?;
    fixture.add_environment_merging_feature()?;

    let output = fixture.dry_run()?;

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let plan: serde_json::Value = serde_json::from_slice(&output.stdout)?;
    assert_eq!(plan["schemaVersion"], 3);
    assert_eq!(plan["taintedBranches"][0]["name"], "feature/b");
    assert_eq!(
        plan["taintedBranches"][0]["absorbedMerges"]
            .as_array()
            .map(Vec::len),
        Some(1)
    );
    assert_eq!(plan["removedBranches"][0]["name"], "feature/b");
    assert_eq!(plan["retainedBranches"][0]["name"], "feature/a");
    assert_eq!(plan["retainedBranches"].as_array().map(Vec::len), Some(1));
    Ok(())
}

#[test]
fn explicit_removals_are_unioned_with_tainted_features() -> Result<(), Box<dyn Error>> {
    let fixture = RestackFixture::new()?;
    fixture.add_environment_merging_feature()?;

    let output = fixture.preview(&["feature/a"])?;

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let plan: serde_json::Value = serde_json::from_slice(&output.stdout)?;
    let removed = plan["removedBranches"]
        .as_array()
        .ok_or("removedBranches")?
        .iter()
        .map(|branch| branch["name"].as_str().unwrap_or_default().to_owned())
        .collect::<Vec<_>>();
    assert_eq!(removed, ["feature/a", "feature/b"]);
    assert_eq!(plan["retainedBranches"].as_array().map(Vec::len), Some(0));
    Ok(())
}
