//! Deterministic repository-controlled Agent Skill generation.

use std::fs;
use std::fs::OpenOptions;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicU64;

use clap::CommandFactory;
use fs2::FileExt;

use crate::cli::{Cli, GenerateSkillsArgs};
use crate::shared::error::CliError;
use content::{INDEX, SKILL};
use paths::{create_parents_without_symlinks, validate_destinations, validate_output_dir};
use publication::replace_artifacts;
use recovery::recover_pending_publications;
use staging::{StagedArtifact, StagingDirectory};

mod content;
mod paths;
mod publication;
mod recovery;
mod staging;
#[cfg(test)]
mod tests;

static STAGING_SEQUENCE: AtomicU64 = AtomicU64::new(0);

const STAGING_PREFIX: &str = ".graduate-generate-skills-";

const MANIFEST_NAME: &str = "transaction.json";

const COMMITTED_NAME: &str = "committed";

pub(crate) fn run(args: &GenerateSkillsArgs) -> Result<(), CliError> {
    let current = std::env::current_dir()?.canonicalize()?;
    let output_dir = validate_output_dir(&current, &args.output_dir)?;
    let skill_path = output_dir.join("graduate").join("SKILL.md");
    let skill = format!(
        "{SKILL}\n## Current command contract\n\n```text\n{}```\n",
        Cli::command().render_long_help()
    );
    let files = [
        GeneratedFile {
            path: skill_path.clone(),
            content: &skill,
        },
        GeneratedFile {
            path: current.join("docs/skills.md"),
            content: INDEX,
        },
    ];
    write_generated(&files, args.force)?;
    println!("Generated {}", skill_path.display());
    Ok(())
}

struct GeneratedFile<'a> {
    path: PathBuf,
    content: &'a str,
}

struct GenerationLock {
    _file: fs::File,
}

impl GenerationLock {
    fn acquire(current: &Path) -> Result<Self, CliError> {
        let mut hasher = DefaultHasher::new();
        current.hash(&mut hasher);
        let lock_path = std::env::temp_dir().join(format!(
            "graduate-generate-skills-{:016x}.lock",
            hasher.finish()
        ));
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(lock_path)?;
        file.lock_exclusive()?;
        Ok(Self { _file: file })
    }
}

fn write_generated(files: &[GeneratedFile<'_>], force: bool) -> Result<(), CliError> {
    let current = std::env::current_dir()?.canonicalize()?;
    let _lock = GenerationLock::acquire(&current)?;
    recover_pending_publications(&current, files)?;
    validate_destinations(files, force)?;
    let mut staging = StagingDirectory::create(&current)?;
    let mut artifacts = Vec::with_capacity(files.len());
    for (index, file) in files.iter().enumerate() {
        let staged = staging.path.join(index.to_string());
        fs::write(&staged, file.content)?;
        artifacts.push(StagedArtifact {
            staged,
            destination: file.path.clone(),
        });
    }
    validate_destinations(files, force)?;
    for file in files {
        if let Some(parent) = file.path.parent() {
            create_parents_without_symlinks(&current, parent)?;
        }
    }
    if let Err(error) = replace_artifacts(&current, &staging.path, &artifacts, force) {
        if let CliError::GeneratedFileExists(_) = &error {
            return Err(error);
        }
        staging.preserve();
        return Err(io::Error::other(format!(
            "{error}; staged recovery files were kept at {}",
            staging.path.display()
        ))
        .into());
    }
    staging.remove()?;
    Ok(())
}
