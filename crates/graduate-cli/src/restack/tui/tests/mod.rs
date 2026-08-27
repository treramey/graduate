//! Shared fixtures and helpers.

use graduate::restack::{
    build_inventory_snapshot, build_plan, AttributedCommit, BranchIdentity, ExplicitFeature,
    FeatureRef, GraphCommit, HistoricalMerge, InventoryError, InventoryMode, MergeOutcome,
    MergeResolution, OrphanedCommit, Reconstruction, RemoteEndpointIdentity, RestackAuthor,
    RestackGraph, RestackSelection, RestackSnapshot, UnsupportedHistory,
};
use ratatui::backend::TestBackend;
use ratatui::Terminal;

use super::render::render;
use super::*;

mod checklist_tests;
mod confirmation_tests;
mod inventory_tests;
mod review_tests;
mod unsupported_history_tests;

fn rendered(
    interaction: &RestackInteraction,
    plan: Option<&RestackPlan>,
    rejection: Option<&str>,
) -> Result<String, Box<dyn std::error::Error>> {
    rendered_at(interaction, plan, rejection, 115, 38)
}

fn rendered_at(
    interaction: &RestackInteraction,
    plan: Option<&RestackPlan>,
    rejection: Option<&str>,
    width: u16,
    height: u16,
) -> Result<String, Box<dyn std::error::Error>> {
    let mut terminal = Terminal::new(TestBackend::new(width, height))?;
    let mut view = RestackViewState::default();
    terminal.draw(|frame| render(frame, interaction, plan, rejection, &mut view))?;
    Ok(terminal.backend().to_string())
}

fn inventory_plan(
    orphan_count: usize,
) -> Result<(RestackInteraction, RestackPlan), Box<dyn std::error::Error>> {
    let mut snapshot = inventory_snapshot(UnsupportedHistory::from(InventoryError::DirectCommit {
        commit: "stray".to_owned(),
    }));
    snapshot.unattributed_commits = (0..orphan_count).map(|n| format!("orphan-{n}")).collect();
    let orphans = (0..orphan_count)
        .map(|n| OrphanedCommit {
            commit: format!("orphan-{n}"),
            subject: format!("lost work {n}"),
            author: "Pat".to_owned(),
            date: "2026-01-02".to_owned(),
        })
        .collect();
    let mut interaction = RestackInteraction::from_inventory(snapshot.clone());
    let _ = interaction.update(RestackInteractionAction::AcceptInventoryFallback);
    let selection = RestackSelection {
        retained: vec![BranchIdentity {
            name: "feature/two".to_owned(),
            tip: "b".to_owned(),
        }],
        removed: Vec::new(),
    };
    let plan = build_plan(
        snapshot,
        RemoteEndpointIdentity {
            fetch_sha256: "f".repeat(64),
            push_sha256: "p".repeat(64),
        },
        RestackAuthor {
            name: "Pat".to_owned(),
            email: "pat@example.com".to_owned(),
        },
        selection,
        Reconstruction {
            merges: vec![MergeOutcome {
                branch: "feature/two".to_owned(),
                tip: "b".to_owned(),
                commit: "preview".to_owned(),
                tree: "tree-tip".to_owned(),
                resolution: MergeResolution::Clean,
            }],
            final_tree: "tree-tip".to_owned(),
            preview_commit: "preview".to_owned(),
        },
        orphans,
    )?;
    Ok((interaction, plan))
}

/// Reachability snapshot: feature/b carries feature/a; `stray` is dropped.
fn inventory_snapshot(reason: UnsupportedHistory) -> RestackSnapshot {
    let ids = |values: &[&str]| -> std::collections::BTreeSet<String> {
        values.iter().map(ToString::to_string).collect()
    };
    let mut commits = std::collections::BTreeMap::new();
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
                name: "feature/PROJ-12-one".to_owned(),
                tip: "a".to_owned(),
                ancestors: ids(&["a"]),
            },
            FeatureRef {
                name: "feature/two".to_owned(),
                tip: "b".to_owned(),
                ancestors: ids(&["a", "b"]),
            },
        ],
        commits,
    };
    build_inventory_snapshot(&graph, reason, &std::collections::BTreeMap::new())
}

fn plan() -> Result<RestackPlan, Box<dyn std::error::Error>> {
    Ok(build_plan(
        snapshot(),
        RemoteEndpointIdentity {
            fetch_sha256: "f".repeat(64),
            push_sha256: "p".repeat(64),
        },
        RestackAuthor {
            name: "Pat".to_owned(),
            email: "pat@example.com".to_owned(),
        },
        RestackSelection {
            retained: vec![BranchIdentity {
                name: "feature/PROJ-12-one".to_owned(),
                tip: "a".repeat(40),
            }],
            removed: vec![BranchIdentity {
                name: "feature/two".to_owned(),
                tip: "b".repeat(40),
            }],
        },
        Reconstruction {
            merges: vec![MergeOutcome {
                branch: "feature/PROJ-12-one".to_owned(),
                tip: "a".repeat(40),
                commit: "preview".to_owned(),
                tree: "tree-tip".to_owned(),
                resolution: MergeResolution::Clean,
            }],
            final_tree: "tree-tip".to_owned(),
            preview_commit: "preview".to_owned(),
        },
        Vec::new(),
    )?)
}

fn snapshot() -> RestackSnapshot {
    RestackSnapshot {
        remote: "origin".to_owned(),
        environment: "qa".to_owned(),
        environment_ref: "refs/remotes/origin/qa".to_owned(),
        environment_tip: "environment-tip".to_owned(),
        main: "main".to_owned(),
        main_ref: "refs/remotes/origin/main".to_owned(),
        main_tip: "main-tip".to_owned(),
        features: vec![
            ExplicitFeature {
                name: "feature/PROJ-12-one".to_owned(),
                tip: "a".repeat(40),
                historical_merges: vec![HistoricalMerge {
                    commit: "merge".to_owned(),
                    first_parent: "parent".to_owned(),
                    feature_parent: "feature".to_owned(),
                    tree: "tree".to_owned(),
                }],
            },
            ExplicitFeature {
                name: "feature/two".to_owned(),
                tip: "b".repeat(40),
                historical_merges: Vec::new(),
            },
        ],
        graduated_features: Vec::new(),
        indirect_features: Vec::new(),
        dropped_markers: Vec::new(),
        attributed_commits: vec![AttributedCommit {
            commit: "shared".to_owned(),
            branches: vec!["feature/PROJ-12-one".to_owned(), "feature/two".to_owned()],
        }],
        inventory_mode: InventoryMode::History,
        unsupported_history: None,
        carried_features: Vec::new(),
        unattributed_commits: Vec::new(),
    }
}
