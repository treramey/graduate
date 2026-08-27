//! Isolated reconstruction repository setup and Git plumbing.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use graduate::restack::{
    BranchIdentity, MergeOutcome, Reconstruction, RemoteEndpointIdentity, RestackAuthor,
    RestackSnapshot,
};
use sha2::{Digest, Sha256};

use super::errors::{isolated_setup_error, reconstruction_error, validation_error};
use super::source::{clear_isolated_environment, read_success_text};
use super::ISOLATED_WHITESPACE_POLICY;
use crate::error::CliError;
use crate::git_process;

pub(super) struct ReconstructionConflict {
    pub(super) merges: Vec<MergeOutcome>,
    pub(super) feature_index: usize,
    pub(super) expected_head: String,
    pub(super) expected_head_reflog: String,
    pub(super) feature: BranchIdentity,
    pub(super) unresolved_paths: Vec<String>,
}

pub(super) enum ReconstructionResult {
    Complete(Reconstruction),
    Conflict(ReconstructionConflict),
}

pub(super) struct FreshRestack<'a> {
    pub(super) isolated: &'a IsolatedRepository,
    pub(super) repository_id: String,
    pub(super) snapshot: RestackSnapshot,
    pub(super) remote_endpoints: RemoteEndpointIdentity,
    pub(super) author: RestackAuthor,
    pub(super) selection: graduate::restack::RestackSelection,
    pub(super) apply_digest: Option<&'a str>,
    pub(super) source: &'a Path,
    pub(super) remote: &'a git_process::RestackRemote,
}

pub(super) struct IsolatedRepository {
    pub(super) root: PathBuf,
    pub(super) hooks: PathBuf,
    pub(super) global_config: PathBuf,
}

impl IsolatedRepository {
    pub(super) fn create(root: &Path, source_objects: &[u8]) -> Result<Self, CliError> {
        let root = root.to_path_buf();
        let session = root.parent().ok_or_else(isolated_setup_error)?;
        let hooks = session.join("hooks");
        let global_config = session.join("global.gitconfig");
        fs::create_dir(&root).map_err(|_| isolated_setup_error())?;
        fs::create_dir(&hooks).map_err(|_| isolated_setup_error())?;
        fs::write(&global_config, []).map_err(|_| isolated_setup_error())?;

        let isolated = Self {
            root,
            hooks,
            global_config,
        };
        isolated.run_success(["init", "--quiet"], "initialize")?;
        fs::write(isolated.root.join(".git/config"), []).map_err(|_| isolated_setup_error())?;
        let alternates = isolated.root.join(".git/objects/info/alternates");
        let mut contents = source_objects.to_vec();
        contents.push(b'\n');
        fs::write(alternates, contents).map_err(|_| isolated_setup_error())?;
        Ok(isolated)
    }

    pub(super) fn open(root: PathBuf, source_objects: &[u8]) -> Result<Self, CliError> {
        let session = root.parent().ok_or_else(isolated_setup_error)?;
        let isolated = Self {
            hooks: session.join("hooks"),
            global_config: session.join("global.gitconfig"),
            root,
        };
        isolated.validate_control_files(source_objects)?;
        Ok(isolated)
    }

    pub(super) fn rerere_remaining(&self) -> Result<Vec<String>, CliError> {
        let output = self.read_text(["rerere", "remaining"], "rerereRemaining")?;
        Ok(output
            .lines()
            .filter(|path| !path.is_empty())
            .map(str::to_owned)
            .collect())
    }

    pub(super) fn head_reflog_digest(&self) -> Result<String, CliError> {
        let reflog = self.read_bytes(
            ["reflog", "show", "HEAD", "--format=%H%x00%gD"],
            "headReflog",
        )?;
        Ok(format!("{:x}", Sha256::digest(reflog)))
    }

    pub(super) fn stage_paths(&self, paths: &[String]) -> Result<(), CliError> {
        if paths.is_empty() {
            return Ok(());
        }
        let output = self
            .command()
            .arg("add")
            .arg("--")
            .args(paths)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .output()
            .map_err(|_| reconstruction_error("stageRerere"))?;
        if output.status.success() {
            Ok(())
        } else {
            Err(reconstruction_error("stageRerere"))
        }
    }

    pub(super) fn commit_tree(
        &self,
        tree: &str,
        first_parent: &str,
        feature_parent: &str,
        message: &str,
        author: &RestackAuthor,
    ) -> Result<String, CliError> {
        let mut command = self.command();
        command
            .args([
                "commit-tree",
                tree,
                "-p",
                first_parent,
                "-p",
                feature_parent,
                "-m",
                message,
            ])
            .env("GIT_AUTHOR_NAME", &author.name)
            .env("GIT_AUTHOR_EMAIL", &author.email)
            .env("GIT_COMMITTER_NAME", &author.name)
            .env("GIT_COMMITTER_EMAIL", &author.email)
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        read_success_text(command.output(), "commitTree")
    }

    pub(super) fn read_text<const N: usize>(
        &self,
        arguments: [&str; N],
        stage: &'static str,
    ) -> Result<String, CliError> {
        let bytes = self.read_bytes(arguments, stage)?;
        let text = String::from_utf8(bytes).map_err(|_| validation_error(stage))?;
        Ok(text.trim_end_matches(['\r', '\n']).to_owned())
    }

    pub(super) fn read_bytes<const N: usize>(
        &self,
        arguments: [&str; N],
        stage: &'static str,
    ) -> Result<Vec<u8>, CliError> {
        let mut command = self.command();
        command
            .args(arguments)
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        let output = command.output().map_err(|_| reconstruction_error(stage))?;
        if output.status.success() {
            Ok(output.stdout)
        } else {
            Err(validation_error(stage))
        }
    }

    pub(super) fn run_success<const N: usize>(
        &self,
        arguments: [&str; N],
        stage: &'static str,
    ) -> Result<(), CliError> {
        let output = self
            .command()
            .args(arguments)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .output()
            .map_err(|_| reconstruction_error(stage))?;
        if output.status.success() {
            Ok(())
        } else {
            Err(reconstruction_error(stage))
        }
    }

    pub(super) fn command(&self) -> Command {
        let mut command = Command::new("git");
        clear_isolated_environment(&mut command);
        command
            .current_dir(&self.root)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", &self.global_config)
            .env("GIT_CONFIG_COUNT", "8")
            .env("GIT_CONFIG_KEY_0", "core.hooksPath")
            .env("GIT_CONFIG_VALUE_0", &self.hooks)
            .env("GIT_CONFIG_KEY_1", "core.fsmonitor")
            .env("GIT_CONFIG_VALUE_1", "false")
            .env("GIT_CONFIG_KEY_2", "commit.gpgSign")
            .env("GIT_CONFIG_VALUE_2", "false")
            .env("GIT_CONFIG_KEY_3", "tag.gpgSign")
            .env("GIT_CONFIG_VALUE_3", "false")
            .env("GIT_CONFIG_KEY_4", "rerere.enabled")
            .env("GIT_CONFIG_VALUE_4", "true")
            .env("GIT_CONFIG_KEY_5", "rerere.autoupdate")
            .env("GIT_CONFIG_VALUE_5", "false")
            .env("GIT_CONFIG_KEY_6", "core.autocrlf")
            .env("GIT_CONFIG_VALUE_6", "false")
            .env("GIT_CONFIG_KEY_7", "core.whitespace")
            .env("GIT_CONFIG_VALUE_7", ISOLATED_WHITESPACE_POLICY)
            .env("GIT_MERGE_AUTOEDIT", "no")
            .env("GIT_TERMINAL_PROMPT", "0");
        command
    }
}
