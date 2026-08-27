//! Commit graph traversal and restack graph construction.

use std::collections::{BTreeMap, BTreeSet, HashSet, VecDeque};

use gix::bstr::ByteSlice;
use graduate::promotion::PromotionCommit;
use graduate::restack::{GraphCommit, OrphanedCommit, RestackGraph};

use super::refs::{excluded_branch, gitoxide_error, unix_date};
use super::{EnvironmentInspection, RestackInspectionError};
use crate::shared::error::CliError;

/// Author timestamps for every remote tip the environment holds but main does not.
pub(crate) fn tip_timestamps(
    repository: &gix::Repository,
    graph: &RestackGraph,
) -> Result<BTreeMap<String, i64>, CliError> {
    let mut timestamps = BTreeMap::new();
    for feature in &graph.feature_refs {
        if !graph.environment_ancestors.contains(&feature.tip)
            || graph.main_ancestors.contains(&feature.tip)
            || timestamps.contains_key(&feature.tip)
        {
            continue;
        }
        let commit = find_commit_by_hex(repository, &feature.tip)?;
        let seconds = commit
            .author()
            .map_err(gitoxide_error)?
            .time()
            .map_err(gitoxide_error)?
            .seconds;
        timestamps.insert(feature.tip.clone(), seconds);
    }
    Ok(timestamps)
}

/// Subject, author, and date rows for commits a rebuild may drop.
pub(crate) fn commit_rows<'a>(
    repository: &gix::Repository,
    ids: impl IntoIterator<Item = &'a String>,
) -> Result<BTreeMap<String, OrphanedCommit>, CliError> {
    let mut rows = BTreeMap::new();
    for id in ids {
        let commit = find_commit_by_hex(repository, id)?;
        let author = commit.author().map_err(gitoxide_error)?;
        let seconds = author.time().map_err(gitoxide_error)?.seconds;
        let subject = commit
            .message_raw()
            .map_err(gitoxide_error)?
            .to_str_lossy()
            .lines()
            .next()
            .unwrap_or_default()
            .trim()
            .to_owned();
        rows.insert(
            id.clone(),
            OrphanedCommit {
                commit: id.clone(),
                subject,
                author: author.name.to_str_lossy().into_owned(),
                date: unix_date(seconds),
            },
        );
    }
    Ok(rows)
}

fn find_commit_by_hex<'repo>(
    repository: &'repo gix::Repository,
    id: &str,
) -> Result<gix::Commit<'repo>, CliError> {
    let id = gix::ObjectId::from_hex(id.as_bytes()).map_err(gitoxide_error)?;
    repository.find_commit(id).map_err(gitoxide_error)
}

/// Commits reachable from `start` that the environment holds but main does not.
///
/// Every commit reachable from main is pruned together with its whole history,
/// so the walk only visits the commits a feature adds on top of main.
pub(super) fn unique_ancestors(
    repository: &gix::Repository,
    start: gix::ObjectId,
    main_ancestors: &HashSet<gix::ObjectId>,
    unique: &HashSet<gix::ObjectId>,
) -> Result<HashSet<gix::ObjectId>, CliError> {
    let mut visited = HashSet::new();
    let mut found = HashSet::new();
    let mut pending = VecDeque::from([start]);
    while let Some(id) = pending.pop_front() {
        if main_ancestors.contains(&id) || !visited.insert(id) {
            continue;
        }
        if unique.contains(&id) {
            found.insert(id);
        }
        let commit = repository.find_commit(id).map_err(gitoxide_error)?;
        pending.extend(commit.parent_ids().map(|parent| parent.detach()));
    }
    Ok(found)
}

/// Load the commits `build_snapshot` reads: every environment-only commit plus
/// the parents it compares trees against.
pub(super) fn classified_commits(
    repository: &gix::Repository,
    unique: &HashSet<gix::ObjectId>,
) -> Result<BTreeMap<String, GraphCommit>, RestackInspectionError> {
    let mut commits = BTreeMap::new();
    let mut parents = Vec::new();
    for id in unique {
        let (key, commit) = graph_commit(repository, *id)?;
        parents.extend(commit.parents.iter().cloned());
        commits.insert(key, commit);
    }
    for parent in parents {
        if commits.contains_key(&parent) {
            continue;
        }
        let id = gix::ObjectId::from_hex(parent.as_bytes())
            .map_err(|error| RestackInspectionError::Git(error.to_string()))?;
        let (key, commit) = graph_commit(repository, id)?;
        commits.insert(key, commit);
    }
    Ok(commits)
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

pub(super) fn object_ids(ids: &HashSet<gix::ObjectId>) -> BTreeSet<String> {
    ids.iter().map(ToString::to_string).collect()
}

pub(super) fn remote_feature_refs(
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

pub(super) fn non_merge_commits_excluding(
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
