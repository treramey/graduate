//! Reference resolution, validation, and Git helpers.

use std::collections::{HashSet, VecDeque};

use gix::bstr::ByteSlice;

use super::KNOWN_ENVIRONMENTS;
use crate::shared::error::CliError;

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
        || contains_percent_encoded_octet(value)
        || gix::validate::reference::name_partial(value.as_bytes().as_bstr()).is_err()
    {
        return Err(CliError::InvalidInput(format!(
            "{label} must be a non-empty Git branch or remote name"
        )));
    }
    Ok(())
}

fn contains_percent_encoded_octet(value: &str) -> bool {
    value.as_bytes().windows(3).any(|octet| {
        octet[0] == b'%' && octet[1].is_ascii_hexdigit() && octet[2].is_ascii_hexdigit()
    })
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
