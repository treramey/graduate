use graduate::restack::TaintedFeature;

use super::super::render::selection_error_message;
use super::*;

fn tainted_snapshot() -> RestackSnapshot {
    let mut snapshot = snapshot();
    snapshot.attributed_commits = vec![
        AttributedCommit {
            commit: "one".to_owned(),
            branches: vec!["feature/PROJ-12-one".to_owned()],
        },
        AttributedCommit {
            commit: "two".to_owned(),
            branches: vec!["feature/two".to_owned()],
        },
    ];
    snapshot.tainted_features = vec![TaintedFeature {
        name: "feature/two".to_owned(),
        tip: "b".repeat(40),
        absorbed_merges: vec!["merge".to_owned()],
    }];
    snapshot
}

#[test]
fn tainted_features_render_removed_with_a_sub_row() -> Result<(), Box<dyn std::error::Error>> {
    let interaction = RestackInteraction::new(tainted_snapshot());
    let rendered = rendered(&interaction, None, None)?;

    assert!(rendered.contains("1 retained · 1 removed"));
    assert!(rendered.contains("↳ tainted  1 environment merge absorbed"));
    assert!(rendered.contains("↳ tainted"));
    Ok(())
}

#[test]
fn retaining_a_tainted_feature_explains_the_remediation() -> Result<(), Box<dyn std::error::Error>>
{
    let mut interaction = RestackInteraction::new(tainted_snapshot());
    interaction.update(RestackInteractionAction::MoveDown);
    let rejection = match interaction.update(RestackInteractionAction::Toggle) {
        RestackInteractionEffect::Rejected(error) => selection_error_message(&error, "main"),
        _ => String::new(),
    };
    let rendered = rendered(&interaction, None, Some(&rejection))?;

    assert!(rendered.contains("Recreate feature/two from main and cherry-pick your commits"));
    assert!(!interaction.is_retained(1));
    Ok(())
}

#[test]
fn review_lists_tainted_branches() -> Result<(), Box<dyn std::error::Error>> {
    let snapshot = tainted_snapshot();
    let selection = RestackSelection {
        retained: vec![BranchIdentity {
            name: "feature/PROJ-12-one".to_owned(),
            tip: "a".repeat(40),
        }],
        removed: vec![BranchIdentity {
            name: "feature/two".to_owned(),
            tip: "b".repeat(40),
        }],
    };
    let plan = build_plan(
        snapshot.clone(),
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
                branch: "feature/PROJ-12-one".to_owned(),
                tip: "a".repeat(40),
                commit: "preview".to_owned(),
                tree: "tree".to_owned(),
                resolution: MergeResolution::Clean,
            }],
            final_tree: "tree".to_owned(),
            preview_commit: "preview".to_owned(),
        },
        Vec::new(),
    )?;
    let mut interaction = RestackInteraction::new(snapshot);
    interaction.review_ready();
    let rendered = rendered(&interaction, Some(&plan), None)?;

    assert!(rendered.contains("Tainted branches (1)"));
    assert!(rendered.contains("1 environment merge absorbed"));
    Ok(())
}
