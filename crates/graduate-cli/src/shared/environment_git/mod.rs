//! Shared Gitoxide inspection for environment-based workflows.

use std::collections::HashSet;

use graduate::promotion::EnvironmentInventory;
use graduate::restack::{
    build_snapshot, FeatureRef, InventoryError, RestackGraph, RestackSnapshot,
};
use thiserror::Error;

use crate::shared::error::CliError;
use graph::{
    classified_commits, non_merge_commits_excluding, object_ids, remote_feature_refs,
    unique_ancestors,
};

mod graph;
mod refs;
#[cfg(test)]
mod tests;

pub(crate) use graph::{commit_rows, tip_timestamps};
#[cfg(test)]
pub(crate) use refs::isolated_git_command;
pub(crate) use refs::{
    ancestors, excluded_branch, gitoxide_error, reference_id, resolve_main_branch, unix_date,
    validate_ref_component,
};

pub(crate) const KNOWN_ENVIRONMENTS: [&str; 3] = ["qa", "staging", "cycle"];

/// Captured refs and reachability shared by promotion and restack inspection.
pub(crate) struct EnvironmentInspection {
    pub(crate) remote: String,
    pub(crate) prefix: String,
    pub(crate) environment: String,
    pub(crate) environment_ref: String,
    pub(crate) environment_id: gix::ObjectId,
    pub(crate) main: String,
    pub(crate) main_ref: String,
    pub(crate) main_id: gix::ObjectId,
    pub(crate) environment_ancestors: HashSet<gix::ObjectId>,
    pub(crate) main_ancestors: HashSet<gix::ObjectId>,
}

#[derive(Debug, Error)]
pub(crate) enum RestackInspectionError {
    #[error("{0}")]
    Git(String),
    /// The history proof failed; the captured graph survives for the
    /// reachability fallback.
    #[error("{error}")]
    Unsupported {
        error: InventoryError,
        graph: Box<RestackGraph>,
    },
}

pub(crate) fn inspect_environment(
    repository: &gix::Repository,
    remote: &str,
    environment: &str,
    explicit_main: Option<&str>,
) -> Result<EnvironmentInspection, CliError> {
    let prefix = format!("refs/remotes/{remote}/");
    let environment_ref = format!("{prefix}{environment}");
    let environment_id = reference_id(repository, &environment_ref)
        .map_err(|_| CliError::Git(format!("{environment_ref} does not exist after fetching")))?;
    let main = resolve_main_branch(repository, &prefix, explicit_main)?;
    let main_ref = format!("{prefix}{main}");
    let main_id = reference_id(repository, &main_ref)?;
    let environment_ancestors = ancestors(repository, environment_id)?;
    let main_ancestors = ancestors(repository, main_id)?;
    Ok(EnvironmentInspection {
        remote: remote.to_owned(),
        prefix,
        environment: environment.to_owned(),
        environment_ref,
        environment_id,
        main,
        main_ref,
        main_id,
        environment_ancestors,
        main_ancestors,
    })
}

pub(crate) fn promotion_candidates(
    repository: &gix::Repository,
    inspection: &EnvironmentInspection,
    selected_branches: Option<&[String]>,
) -> Result<Vec<(String, gix::ObjectId)>, CliError> {
    if let Some(selected_branches) = selected_branches {
        let mut candidates = Vec::with_capacity(selected_branches.len());
        for branch in selected_branches {
            if excluded_branch(branch, &inspection.environment, &inspection.main) {
                return Err(CliError::InvalidInput(format!(
                    "--params branch `{branch}` is not a selectable feature branch"
                )));
            }
            let reference = format!("{}{branch}", inspection.prefix);
            let id = reference_id(repository, &reference).map_err(|_| {
                CliError::InvalidInput(format!(
                    "--params branch `{branch}` does not exist on the selected remote"
                ))
            })?;
            if inspection.main_ancestors.contains(&id) {
                return Err(CliError::InvalidInput(format!(
                    "--params branch `{branch}` has already reached `{}`",
                    inspection.main
                )));
            }
            if !inspection.environment_ancestors.contains(&id) {
                return Err(CliError::InvalidInput(format!(
                    "--params branch `{branch}` has not reached `{}`",
                    inspection.environment
                )));
            }
            candidates.push((branch.clone(), id));
        }
        return Ok(candidates);
    }

    let mut candidates = Vec::new();
    for (branch, id) in remote_feature_refs(repository, inspection)? {
        if inspection.environment_ancestors.contains(&id)
            && !inspection.main_ancestors.contains(&id)
        {
            candidates.push((branch, id));
        }
    }
    Ok(candidates)
}

pub(crate) fn promotion_inventory(
    repository: &gix::Repository,
    inspection: &EnvironmentInspection,
) -> Result<EnvironmentInventory, CliError> {
    Ok(EnvironmentInventory {
        ahead: non_merge_commits_excluding(
            repository,
            inspection.environment_id,
            &inspection.main_ancestors,
        )?,
        behind_main: non_merge_commits_excluding(
            repository,
            inspection.main_id,
            &inspection.environment_ancestors,
        )?,
    })
}

pub(crate) fn restack_snapshot(
    repository: &gix::Repository,
    inspection: &EnvironmentInspection,
) -> Result<RestackSnapshot, RestackInspectionError> {
    let unique = inspection
        .environment_ancestors
        .difference(&inspection.main_ancestors)
        .copied()
        .collect::<HashSet<_>>();
    let feature_refs = remote_feature_refs(repository, inspection)
        .map_err(|error| RestackInspectionError::Git(error.to_string()))?
        .into_iter()
        .map(|(name, tip)| {
            let ancestors = unique_ancestors(repository, tip, &inspection.main_ancestors, &unique)
                .map_err(|error| RestackInspectionError::Git(error.to_string()))?
                .into_iter()
                .map(|id| id.to_string())
                .collect();
            Ok(FeatureRef {
                name,
                tip: tip.to_string(),
                ancestors,
            })
        })
        .collect::<Result<Vec<_>, RestackInspectionError>>()?;
    let commits = classified_commits(repository, &unique)?;
    let graph = RestackGraph {
        remote: inspection.remote.clone(),
        environment: inspection.environment.clone(),
        environment_ref: inspection.environment_ref.clone(),
        environment_tip: inspection.environment_id.to_string(),
        main: inspection.main.clone(),
        main_ref: inspection.main_ref.clone(),
        main_tip: inspection.main_id.to_string(),
        environment_ancestors: object_ids(&inspection.environment_ancestors),
        main_ancestors: object_ids(&inspection.main_ancestors),
        feature_refs,
        commits,
    };
    match build_snapshot(&graph) {
        Ok(snapshot) => Ok(snapshot),
        Err(error) => Err(RestackInspectionError::Unsupported {
            error,
            graph: Box::new(graph),
        }),
    }
}
