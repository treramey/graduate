use std::collections::BTreeMap;

use graduate::restack::{
    build_inventory_snapshot, build_plan, select_features, InventoryMode, MergeOutcome,
    MergeResolution, OrphanedCommit, PlanError, Reconstruction, RemoteEndpointIdentity,
    RestackAuthor,
};

use super::super::errors::plan_error;
use super::super::machine_output::plan_json;
use super::*;

#[test]
fn plan_json_describes_the_inventory_mode_carried_branches_and_orphans(
) -> Result<(), Box<dyn std::error::Error>> {
    use graduate::restack::{
        FeatureRef, GraphCommit, InventoryError, RestackGraph, UnsupportedHistory,
    };
    use std::collections::BTreeSet;

    let ids =
        |values: &[&str]| -> BTreeSet<String> { values.iter().map(ToString::to_string).collect() };
    let mut commits = BTreeMap::new();
    for (id, parents) in [
        ("base", vec![]),
        ("a", vec!["base"]),
        ("b", vec!["a"]),
        ("stray", vec!["b"]),
    ] {
        commits.insert(
            id.to_owned(),
            GraphCommit {
                id: id.to_owned(),
                tree: format!("tree-{id}"),
                parents: parents.into_iter().map(str::to_owned).collect(),
                message: id.to_owned(),
            },
        );
    }
    let graph = RestackGraph {
        remote: "origin".to_owned(),
        environment: "qa".to_owned(),
        environment_ref: "refs/remotes/origin/qa".to_owned(),
        environment_tip: "stray".to_owned(),
        main: "main".to_owned(),
        main_ref: "refs/remotes/origin/main".to_owned(),
        main_tip: "base".to_owned(),
        environment_ancestors: ids(&["base", "a", "b", "stray"]),
        main_ancestors: ids(&["base"]),
        feature_refs: vec![
            FeatureRef {
                name: "feature/a".to_owned(),
                tip: "a".to_owned(),
                ancestors: ids(&["a"]),
            },
            FeatureRef {
                name: "feature/b".to_owned(),
                tip: "b".to_owned(),
                ancestors: ids(&["a", "b"]),
            },
        ],
        commits,
    };
    let reason = UnsupportedHistory::from(InventoryError::DirectCommit {
        commit: "stray".to_owned(),
    });
    let snapshot = build_inventory_snapshot(&graph, reason, &BTreeMap::new());
    let selection = select_features(&snapshot, &[])?;
    let author = RestackAuthor {
        name: "Pat".to_owned(),
        email: "pat@example.com".to_owned(),
    };
    let endpoints = RemoteEndpointIdentity {
        fetch_sha256: "f".repeat(64),
        push_sha256: "p".repeat(64),
    };
    let reconstruction = || Reconstruction {
        merges: vec![MergeOutcome {
            branch: "feature/b".to_owned(),
            tip: "b".to_owned(),
            commit: "preview".to_owned(),
            tree: "tree".to_owned(),
            resolution: MergeResolution::Clean,
        }],
        final_tree: "tree".to_owned(),
        preview_commit: "preview".to_owned(),
    };
    let plan = build_plan(
        snapshot.clone(),
        endpoints.clone(),
        author.clone(),
        selection.clone(),
        reconstruction(),
        vec![OrphanedCommit {
            commit: "stray".to_owned(),
            subject: "stray".to_owned(),
            author: "Pat".to_owned(),
            date: "2026-01-02".to_owned(),
        }],
    )?;
    let value = plan_json(&plan);
    assert_eq!(value["schemaVersion"], 2);
    assert_eq!(value["inventory"]["mode"], "reachability");
    assert_eq!(value["inventory"]["reason"]["kind"], "directCommit");
    assert_eq!(value["inventory"]["reason"]["commit"], "stray");
    assert_eq!(
        value["carriedBranches"],
        json!([{"name": "feature/a", "tip": "a", "carriers": ["feature/b"]}])
    );
    assert_eq!(
        value["orphanedCommits"],
        json!([{"commit": "stray", "subject": "stray", "author": "Pat", "date": "2026-01-02"}])
    );
    assert_eq!(value["effects"]["reusedResolutions"], false);

    let mut history = snapshot;
    history.inventory_mode = InventoryMode::History;
    history.unsupported_history = None;
    history.carried_features.clear();
    history.unattributed_commits.clear();
    let plan = build_plan(
        history,
        endpoints,
        author,
        selection,
        reconstruction(),
        Vec::new(),
    )?;
    let value = plan_json(&plan);
    assert_eq!(
        value["inventory"],
        json!({"mode": "history", "reason": null})
    );
    assert_eq!(value["carriedBranches"], json!([]));
    assert_eq!(value["orphanedCommits"], json!([]));
    assert_eq!(value["effects"]["reusedResolutions"], true);
    Ok(())
}

#[test]
fn plan_error_reports_orphan_mismatches_as_machine_json() -> Result<(), Box<dyn std::error::Error>>
{
    let error = plan_error(PlanError::OrphanedCommits {
        expected: 2,
        actual: 1,
        mismatch: "abc".to_owned(),
    });
    let CliError::Machine(error) = error else {
        return Err("plan_error must build a machine failure".into());
    };
    assert_eq!(error.code, "validation_failed");
    let text = error.to_string();
    assert!(text.contains(r#""stage":"orphanedCommits""#), "{text}");
    assert!(text.contains(r#""mismatch":"abc""#), "{text}");
    Ok(())
}
