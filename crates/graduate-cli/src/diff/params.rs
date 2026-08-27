//! Unattended promotion report parameter parsing.

use serde::Deserialize;

use crate::shared::environment_git::validate_ref_component;
use crate::shared::error::CliError;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DiffParams {
    branches: Vec<String>,
}

pub(super) fn parse_selected_branches(
    params: Option<&str>,
) -> Result<Option<Vec<String>>, CliError> {
    let Some(params) = params else {
        return Ok(None);
    };
    let parsed: DiffParams = serde_json::from_str(params).map_err(|error| {
        CliError::InvalidInput(format!(
            "--params must be a JSON object like {{\"branches\":[\"feature/A\",\"feature/B\"]}}: {error}"
        ))
    })?;
    if parsed.branches.is_empty() {
        return Err(CliError::InvalidInput(
            "--params branches must contain at least one feature branch".to_owned(),
        ));
    }
    for branch in &parsed.branches {
        validate_ref_component("--params branch", branch)?;
    }
    let mut branches = parsed.branches;
    branches.sort();
    branches.dedup();
    Ok(Some(branches))
}
