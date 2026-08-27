//! Isolated repository state validation.

use std::fs;

use graduate::restack::{canonical_merge_message, RestackAuthor, RestackPlan};
use serde_json::json;

use super::errors::{
    require_plain_directory, require_plain_file, session_state_error, validation_error,
};
use super::isolated::IsolatedRepository;
use super::machine_output::machine_failure;
use crate::shared::error::CliError;

impl IsolatedRepository {
    pub(super) fn validate_index(&self) -> Result<(), CliError> {
        let unmerged = self.read_bytes(["ls-files", "-u", "-z"], "unmergedIndex")?;
        if !unmerged.is_empty() {
            return Err(validation_error("unmergedIndex"));
        }
        self.run_success(["diff", "--cached", "--check"], "stagedDiffCheck")?;
        self.run_success(["diff", "--check"], "worktreeDiffCheck")
    }

    pub(super) fn validate_commit(
        &self,
        commit: &str,
        first_parent: &str,
        feature_parent: &str,
        message: &str,
        author: &RestackAuthor,
    ) -> Result<(), CliError> {
        let parents = self.read_text(["rev-list", "--parents", "-n", "1", commit], "parents")?;
        let expected = format!("{commit} {first_parent} {feature_parent}");
        if parents != expected {
            return Err(validation_error("parents"));
        }
        let actual_message = self.read_text(["show", "-s", "--format=%B", commit], "message")?;
        if actual_message != message {
            return Err(validation_error("message"));
        }
        let identity = self.read_bytes(
            ["show", "-s", "--format=%an%x00%ae%x00%cn%x00%ce", commit],
            "identity",
        )?;
        let expected_identity = format!(
            "{}\0{}\0{}\0{}\n",
            author.name, author.email, author.name, author.email
        );
        if identity != expected_identity.as_bytes() {
            return Err(validation_error("identity"));
        }
        let raw = self.read_bytes(["cat-file", "commit", commit], "signature")?;
        let headers = raw
            .split(|byte| *byte == b'\n')
            .take_while(|line| !line.is_empty());
        if headers.into_iter().any(|line| line.starts_with(b"gpgsig ")) {
            return Err(validation_error("signature"));
        }
        Ok(())
    }

    pub(super) fn validate_clean_state(
        &self,
        previous: &str,
        commit: &str,
    ) -> Result<(), CliError> {
        let status = self.read_bytes(["status", "--porcelain=v1", "-z"], "status")?;
        if !status.is_empty() {
            return Err(validation_error("indexState"));
        }
        self.run_success(
            ["diff-tree", "--check", previous, commit],
            "resultDiffCheck",
        )
    }

    pub(super) fn validate_publication_plan(&self, plan: &RestackPlan) -> Result<(), CliError> {
        let head = self.read_text(["rev-parse", "HEAD"], "publicationHead")?;
        let tree = self.read_text(["rev-parse", "HEAD^{tree}"], "publicationTree")?;
        if head != plan.preview_commit || tree != plan.final_tree {
            return Err(validation_error("publicationResult"));
        }
        let mut first_parent = plan.snapshot.main_tip.as_str();
        for (merge, feature) in plan.merges.iter().zip(&plan.selection.retained) {
            let message = canonical_merge_message(&feature.name, &plan.snapshot.environment);
            self.validate_commit(
                &merge.commit,
                first_parent,
                &feature.tip,
                &message,
                &plan.author,
            )?;
            first_parent = &merge.commit;
        }
        self.validate_clean_state(&plan.snapshot.main_tip, &plan.preview_commit)
    }

    pub(super) fn unresolved_paths(&self) -> Result<Vec<String>, CliError> {
        let bytes = self.read_bytes(
            ["diff", "--name-only", "--diff-filter=U", "-z"],
            "conflictPaths",
        )?;
        bytes
            .split(|byte| *byte == 0)
            .filter(|path| !path.is_empty())
            .map(|path| {
                String::from_utf8(path.to_vec()).map_err(|_| {
                    machine_failure(
                        "invalid_path_encoding",
                        "an unresolved path is not valid UTF-8",
                        json!({}),
                    )
                })
            })
            .collect()
    }

    pub(super) fn validate_control_files(&self, source_objects: &[u8]) -> Result<(), CliError> {
        require_plain_directory(&self.root)?;
        require_plain_directory(&self.hooks)?;
        if fs::read_dir(&self.hooks)
            .map_err(|_| session_state_error("hooks"))?
            .next()
            .is_some()
        {
            return Err(session_state_error("hooks"));
        }
        for path in [&self.global_config, &self.root.join(".git/config")] {
            require_plain_file(path)?;
            if !fs::read(path)
                .map_err(|_| session_state_error("configuration"))?
                .is_empty()
            {
                return Err(session_state_error("configuration"));
            }
        }
        let alternates = self.root.join(".git/objects/info/alternates");
        require_plain_file(&alternates)?;
        let mut expected = source_objects.to_vec();
        expected.push(b'\n');
        if fs::read(alternates).map_err(|_| session_state_error("objectStore"))? != expected {
            return Err(session_state_error("objectStore"));
        }
        Ok(())
    }
}
