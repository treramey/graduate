//! Plan digest authorization and remote ref revalidation.

use std::collections::BTreeMap;
use std::path::Path;

use graduate::restack::RestackPlan;
use serde_json::json;

use super::machine_output::machine_failure;
use super::source::configured_author;
use crate::shared::error::CliError;
use crate::shared::git_process;

pub(super) fn authorize_plan(
    plan: &RestackPlan,
    requested_digest: Option<&str>,
) -> Result<(), CliError> {
    if requested_digest == Some(plan.digest.as_str()) {
        Ok(())
    } else {
        Err(machine_failure(
            "stale_plan",
            "the freshly reconstructed plan does not match the reviewed digest",
            json!({"reason": "planDigest"}),
        ))
    }
}

pub(super) fn revalidate_plan(
    source: &Path,
    remote: &git_process::RestackRemote,
    plan: &RestackPlan,
) -> Result<(), CliError> {
    if configured_author(source)? != plan.author {
        return Err(machine_failure(
            "stale_plan",
            "the configured Git identity changed before publication",
            json!({"reason": "identity"}),
        ));
    }
    let expected = expected_remote_refs(plan);
    let refs = expected.keys().cloned().collect::<Vec<_>>();
    validate_remote_refs(
        git_process::read_restack_remote_refs(remote, source, &refs, false),
        &expected,
        "fetch",
    )?;
    if remote.has_distinct_push_endpoint() {
        validate_remote_refs(
            git_process::read_restack_remote_refs(remote, source, &refs, true),
            &expected,
            "push",
        )?;
    }
    Ok(())
}

fn expected_remote_refs(plan: &RestackPlan) -> BTreeMap<String, String> {
    let mut refs = BTreeMap::new();
    refs.insert(
        remote_environment_ref(&plan.snapshot.environment),
        plan.snapshot.environment_tip.clone(),
    );
    refs.insert(
        remote_environment_ref(&plan.snapshot.main),
        plan.snapshot.main_tip.clone(),
    );
    for feature in plan
        .selection
        .retained
        .iter()
        .chain(&plan.selection.removed)
    {
        refs.insert(remote_environment_ref(&feature.name), feature.tip.clone());
    }
    refs
}

fn validate_remote_refs(
    actual: Result<BTreeMap<String, String>, CliError>,
    expected: &BTreeMap<String, String>,
    endpoint: &'static str,
) -> Result<(), CliError> {
    let actual = actual.map_err(|_| {
        machine_failure(
            "remote_revalidation_failed",
            "could not re-read the remote refs before publication",
            json!({"endpoint": endpoint}),
        )
    })?;
    for (reference, expected_oid) in expected {
        match actual.get(reference) {
            Some(actual_oid) if actual_oid == expected_oid => {}
            Some(_) => {
                return Err(machine_failure(
                    "stale_plan",
                    "a reviewed remote input moved before publication",
                    json!({"reason": "movedRef", "ref": reference, "endpoint": endpoint}),
                ));
            }
            None => {
                return Err(machine_failure(
                    "stale_plan",
                    "a reviewed remote input was deleted before publication",
                    json!({"reason": "deletedRef", "ref": reference, "endpoint": endpoint}),
                ));
            }
        }
    }
    Ok(())
}

pub(super) fn remote_environment_ref(branch: &str) -> String {
    format!("refs/heads/{branch}")
}
