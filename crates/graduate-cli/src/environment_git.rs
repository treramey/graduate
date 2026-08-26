//! Shared Gitoxide inspection for environment-based workflows.

use std::collections::{BTreeMap, BTreeSet, HashSet, VecDeque};

use gix::bstr::ByteSlice;
use graduate::promotion::{EnvironmentInventory, PromotionCommit};
use graduate::restack::{
    build_snapshot, FeatureRef, GraphCommit, InventoryError, RestackGraph, RestackSnapshot,
};
use thiserror::Error;

use crate::error::CliError;

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
    #[error(transparent)]
    Unsupported(#[from] InventoryError),
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
    let feature_refs = remote_feature_refs(repository, inspection)
        .map_err(|error| RestackInspectionError::Git(error.to_string()))?
        .into_iter()
        .map(|(name, tip)| {
            let ancestors = ancestors(repository, tip)
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
    let commits = inspection
        .environment_ancestors
        .iter()
        .map(|id| graph_commit(repository, *id))
        .collect::<Result<BTreeMap<_, _>, _>>()?;
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
    build_snapshot(&graph).map_err(RestackInspectionError::from)
}

fn graph_commit(
    repository: &gix::Repository,
    id: gix::ObjectId,
) -> Result<(String, GraphCommit), RestackInspectionError> {
    let commit = repository
        .find_commit(id)
        .map_err(|error| RestackInspectionError::Git(error.to_string()))?;
    let tree = commit
        .tree_id()
        .map_err(|error| RestackInspectionError::Git(error.to_string()))?
        .detach()
        .to_string();
    let parents = commit
        .parent_ids()
        .map(|parent| parent.detach().to_string())
        .collect();
    let message = commit
        .message_raw()
        .map_err(|error| RestackInspectionError::Git(error.to_string()))?
        .to_str_lossy()
        .into_owned();
    let id = id.to_string();
    Ok((
        id.clone(),
        GraphCommit {
            id,
            tree,
            parents,
            message,
        },
    ))
}

fn object_ids(ids: &HashSet<gix::ObjectId>) -> BTreeSet<String> {
    ids.iter().map(ToString::to_string).collect()
}

fn remote_feature_refs(
    repository: &gix::Repository,
    inspection: &EnvironmentInspection,
) -> Result<Vec<(String, gix::ObjectId)>, CliError> {
    let references = repository.references().map_err(gitoxide_error)?;
    let references = references
        .prefixed(inspection.prefix.as_str())
        .map_err(gitoxide_error)?;
    let mut feature_refs = Vec::new();
    for reference in references {
        let mut reference = reference.map_err(gitoxide_error)?;
        let full_name = reference.name().as_bstr().to_str_lossy();
        let Some(branch) = full_name
            .strip_prefix(&inspection.prefix)
            .map(str::to_owned)
        else {
            continue;
        };
        if excluded_branch(&branch, &inspection.environment, &inspection.main) {
            continue;
        }
        let id = reference.peel_to_id().map_err(gitoxide_error)?.detach();
        feature_refs.push((branch, id));
    }
    feature_refs.sort_by(|left, right| left.0.cmp(&right.0));
    Ok(feature_refs)
}

fn non_merge_commits_excluding(
    repository: &gix::Repository,
    tip: gix::ObjectId,
    excluded_ancestors: &HashSet<gix::ObjectId>,
) -> Result<Vec<PromotionCommit>, CliError> {
    let mut visited = HashSet::new();
    let mut pending = VecDeque::from([tip]);
    let mut commits = Vec::new();
    while let Some(id) = pending.pop_front() {
        if excluded_ancestors.contains(&id) || !visited.insert(id) {
            continue;
        }
        let commit = repository.find_commit(id).map_err(gitoxide_error)?;
        let parents = commit
            .parent_ids()
            .map(|parent| parent.detach())
            .collect::<Vec<_>>();
        pending.extend(parents.iter().copied());
        if parents.len() > 1 {
            continue;
        }
        let author = commit.author().map_err(gitoxide_error)?;
        let seconds = author.time().map_err(gitoxide_error)?.seconds;
        let message = commit.message_raw().map_err(gitoxide_error)?;
        let subject = message
            .to_str_lossy()
            .lines()
            .next()
            .unwrap_or_default()
            .trim()
            .to_owned();
        let id = id.to_string();
        commits.push((
            seconds,
            PromotionCommit {
                short_id: id.chars().take(7).collect(),
                id,
                subject,
                author: author.name.to_str_lossy().into_owned(),
                date: unix_date(seconds),
            },
        ));
    }
    commits.sort_by(|left, right| {
        right
            .0
            .cmp(&left.0)
            .then_with(|| left.1.id.cmp(&right.1.id))
    });
    Ok(commits.into_iter().map(|(_, commit)| commit).collect())
}

pub(crate) fn resolve_main_branch(
    repository: &gix::Repository,
    prefix: &str,
    explicit: Option<&str>,
) -> Result<String, CliError> {
    if let Some(explicit) = explicit {
        reference_id(repository, &format!("{prefix}{explicit}"))?;
        return Ok(explicit.to_owned());
    }
    if let Ok(reference) = repository.find_reference(&format!("{prefix}HEAD")) {
        if let Some(target) = reference.target().try_name() {
            let target = target.as_bstr().to_str_lossy();
            if let Some(branch) = target.strip_prefix(prefix) {
                if reference_id(repository, &target).is_ok() {
                    return Ok(branch.to_owned());
                }
            }
        }
    }
    for candidate in ["main", "master", "trunk", "develop"] {
        if reference_id(repository, &format!("{prefix}{candidate}")).is_ok() {
            return Ok(candidate.to_owned());
        }
    }
    Err(CliError::Git(
        "could not determine the main branch from the remote default or common names; pass --main <BRANCH>"
            .to_owned(),
    ))
}

pub(crate) fn reference_id(
    repository: &gix::Repository,
    name: &str,
) -> Result<gix::ObjectId, CliError> {
    let mut reference = repository.find_reference(name).map_err(gitoxide_error)?;
    reference
        .peel_to_id()
        .map(|id| id.detach())
        .map_err(gitoxide_error)
}

pub(crate) fn ancestors(
    repository: &gix::Repository,
    start: gix::ObjectId,
) -> Result<HashSet<gix::ObjectId>, CliError> {
    let mut found = HashSet::new();
    let mut pending = VecDeque::from([start]);
    while let Some(id) = pending.pop_front() {
        if !found.insert(id) {
            continue;
        }
        let commit = repository.find_commit(id).map_err(gitoxide_error)?;
        pending.extend(commit.parent_ids().map(|parent| parent.detach()));
    }
    Ok(found)
}

pub(crate) fn excluded_branch(branch: &str, environment: &str, main: &str) -> bool {
    branch == "HEAD"
        || branch == environment
        || branch == main
        || KNOWN_ENVIRONMENTS.contains(&branch)
        || branch.starts_with("backup/")
}

pub(crate) fn validate_ref_component(label: &str, value: &str) -> Result<(), CliError> {
    if value.trim().is_empty()
        || value.starts_with('-')
        || value.chars().any(char::is_control)
        || gix::validate::reference::name_partial(value.as_bytes().as_bstr()).is_err()
    {
        return Err(CliError::InvalidInput(format!(
            "{label} must be a non-empty Git branch or remote name"
        )));
    }
    Ok(())
}

pub(crate) fn unix_date(seconds: i64) -> String {
    let days = seconds.div_euclid(86_400);
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let day_of_era = z - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let mut year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = month_prime + if month_prime < 10 { 3 } else { -9 };
    year += i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}")
}

pub(crate) fn gitoxide_error(error: impl std::fmt::Display) -> CliError {
    CliError::Git(error.to_string())
}

#[cfg(test)]
pub(crate) fn isolated_git_command() -> std::process::Command {
    let mut command = std::process::Command::new("git");
    for variable in [
        "GIT_ALTERNATE_OBJECT_DIRECTORIES",
        "GIT_COMMON_DIR",
        "GIT_DIR",
        "GIT_GRAFT_FILE",
        "GIT_INDEX_FILE",
        "GIT_INTERNAL_SUPER_PREFIX",
        "GIT_OBJECT_DIRECTORY",
        "GIT_PREFIX",
        "GIT_QUARANTINE_PATH",
        "GIT_SHALLOW_FILE",
        "GIT_WORK_TREE",
    ] {
        command.env_remove(variable);
    }
    command
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

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
}
