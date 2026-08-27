//! Report file output and destination validation.

use std::io::{self, Write};
use std::path::{Path, PathBuf};

use graduate::promotion::{PromotionAgeReport, ReportDate};

use super::age_csv::format_age_csv;
use super::report_csv::format_csv;
use super::report_json::{age_report_value, report_value};
use super::report_table::{format_age_table, format_table};
use super::PromotionReport;
use crate::cli::{DiffReport, ReportFormat};
use crate::environment_git::unix_date;
use crate::error::CliError;

pub(super) fn write_report(
    report: &PromotionReport,
    report_kind: DiffReport,
    format: ReportFormat,
    output: Option<&Path>,
) -> Result<(), CliError> {
    let content = match report_kind {
        DiffReport::Branches => match format {
            ReportFormat::Json => {
                format!("{}\n", serde_json::to_string_pretty(&report_value(report))?)
            }
            ReportFormat::Yaml => serde_yaml::to_string(&report_value(report))?,
            ReportFormat::Table => format_table(report),
            ReportFormat::Csv => format_csv(report)?,
        },
        DiffReport::Age => {
            let as_of = current_report_date()?;
            let age = PromotionAgeReport::new(&report.inventory.ahead, &report.branches, as_of)
                .map_err(|error| CliError::Git(format!("could not build age report: {error}")))?;
            match format {
                ReportFormat::Json => format!(
                    "{}\n",
                    serde_json::to_string_pretty(&age_report_value(report, &age))?
                ),
                ReportFormat::Yaml => serde_yaml::to_string(&age_report_value(report, &age))?,
                ReportFormat::Table => format_age_table(report, &age),
                ReportFormat::Csv => format_age_csv(report, &age)?,
            }
        }
    };
    if let Some(output) = output {
        let output = validate_output_path(output)?;
        let mut temporary = tempfile::Builder::new()
            .prefix(".graduate-report-")
            .suffix(".tmp")
            .tempfile_in(&output.parent)?;
        temporary.write_all(content.as_bytes())?;
        temporary.as_file().sync_all()?;
        revalidate_output_parent(&output)?;
        temporary
            .persist(&output.destination)
            .map_err(|error| CliError::Io(error.error))?;
        eprintln!("Wrote {}", output.destination.display());
    } else {
        let stdout = io::stdout();
        let mut stdout = stdout.lock();
        stdout.write_all(content.as_bytes())?;
    }
    Ok(())
}

pub(crate) fn current_report_date() -> Result<ReportDate, CliError> {
    let elapsed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|error| CliError::Git(format!("system clock predates the Unix epoch: {error}")))?;
    let seconds = i64::try_from(elapsed.as_secs())
        .map_err(|_| CliError::Git("system clock is outside the supported range".to_owned()))?;
    ReportDate::parse(&unix_date(seconds))
        .map_err(|error| CliError::Git(format!("could not determine report date: {error}")))
}

pub(super) struct ValidatedOutput {
    pub(super) destination: PathBuf,
    parent: PathBuf,
    repository: PathBuf,
}

pub(super) fn validate_output_path(path: &Path) -> Result<ValidatedOutput, CliError> {
    if path.as_os_str().is_empty()
        || path.is_absolute()
        || path.components().any(|component| {
            matches!(
                component,
                std::path::Component::ParentDir
                    | std::path::Component::RootDir
                    | std::path::Component::Prefix(_)
            )
        })
        || path.to_string_lossy().chars().any(char::is_control)
    {
        return Err(CliError::InvalidInput(
            "--output must be a relative file path within the current directory".to_owned(),
        ));
    }
    let current = std::env::current_dir()?.canonicalize()?;
    let lexical_destination = current.join(path);
    let parent = lexical_destination
        .parent()
        .ok_or_else(|| CliError::InvalidInput("--output must name a file".to_owned()))?;
    let parent = parent.canonicalize().map_err(|error| {
        CliError::InvalidInput(format!(
            "--output parent directory must already exist: {error}"
        ))
    })?;
    if !parent.starts_with(&current) {
        return Err(CliError::InvalidInput(
            "--output must stay within the current directory".to_owned(),
        ));
    }
    let leaf = lexical_destination
        .file_name()
        .ok_or_else(|| CliError::InvalidInput("--output must name a file".to_owned()))?;
    let destination = parent.join(leaf);
    if std::fs::symlink_metadata(&destination)
        .is_ok_and(|metadata| metadata.file_type().is_symlink())
    {
        return Err(CliError::InvalidInput(
            "--output must not replace a symbolic link".to_owned(),
        ));
    }
    Ok(ValidatedOutput {
        destination,
        parent,
        repository: current,
    })
}

fn revalidate_output_parent(output: &ValidatedOutput) -> Result<(), CliError> {
    let parent = output.parent.canonicalize()?;
    if parent != output.parent || !parent.starts_with(&output.repository) {
        return Err(CliError::InvalidInput(
            "--output parent directory changed while the report was being written".to_owned(),
        ));
    }
    if std::fs::symlink_metadata(&output.destination)
        .is_ok_and(|metadata| metadata.file_type().is_symlink())
    {
        return Err(CliError::InvalidInput(
            "--output must not replace a symbolic link".to_owned(),
        ));
    }
    Ok(())
}
