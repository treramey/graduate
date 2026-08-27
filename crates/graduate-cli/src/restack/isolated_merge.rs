//! Isolated merge reconstruction, rerere training, and manual completion.

use std::process::Stdio;

use graduate::restack::{
    canonical_merge_message, BranchIdentity, MergeOutcome, MergeResolution, Reconstruction,
    RestackAuthor, RestackSnapshot,
};

use super::errors::{reconstruction_error, session_state_error, validation_error};
use super::isolated::{IsolatedRepository, ReconstructionConflict, ReconstructionResult};
use crate::shared::error::CliError;

impl IsolatedRepository {
    pub(super) fn reconstruct(
        &self,
        main_tip: &str,
        environment: &str,
        retained: &[BranchIdentity],
        author: &RestackAuthor,
        start_index: usize,
        mut merges: Vec<MergeOutcome>,
    ) -> Result<ReconstructionResult, CliError> {
        let mut previous = if start_index == 0 {
            self.run_success(
                ["checkout", "--detach", "--quiet", main_tip, "--"],
                "checkoutBase",
            )?;
            main_tip.to_owned()
        } else {
            self.read_text(["rev-parse", "HEAD"], "continuationHead")?
        };
        for (feature_index, feature) in retained.iter().enumerate().skip(start_index) {
            let mut merge_command = self.command();
            merge_command
                .args([
                    "merge",
                    "--no-ff",
                    "--no-commit",
                    "--no-edit",
                    "--no-gpg-sign",
                    feature.tip.as_str(),
                ])
                .env("GIT_AUTHOR_NAME", &author.name)
                .env("GIT_AUTHOR_EMAIL", &author.email)
                .env("GIT_COMMITTER_NAME", &author.name)
                .env("GIT_COMMITTER_EMAIL", &author.email)
                .stdout(Stdio::null())
                .stderr(Stdio::null());
            let merge = merge_command
                .output()
                .map_err(|_| reconstruction_error("merge"))?;
            let resolution = if merge.status.success() {
                MergeResolution::Clean
            } else {
                let conflicted = self.unresolved_paths()?;
                if conflicted.is_empty() {
                    return Err(reconstruction_error("merge"));
                }
                self.run_success(["rerere"], "rerereReplay")?;
                let remaining = self.rerere_remaining()?;
                let resolved = conflicted
                    .iter()
                    .filter(|path| !remaining.contains(path))
                    .cloned()
                    .collect::<Vec<_>>();
                self.stage_paths(&resolved)?;
                if !remaining.is_empty() {
                    return Ok(ReconstructionResult::Conflict(ReconstructionConflict {
                        merges,
                        feature_index,
                        expected_head: previous,
                        expected_head_reflog: self.head_reflog_digest()?,
                        feature: feature.clone(),
                        unresolved_paths: remaining,
                    }));
                }
                MergeResolution::Reused
            };
            self.validate_index()?;
            let tree = self.read_text(["write-tree"], "writeTree")?;
            let message = canonical_merge_message(&feature.name, environment);
            let commit = self.commit_tree(&tree, &previous, &feature.tip, &message, author)?;
            self.run_success(["reset", "--hard", "--quiet", &commit], "resetResult")?;
            self.validate_commit(&commit, &previous, &feature.tip, &message, author)?;
            self.validate_clean_state(&previous, &commit)?;
            previous.clone_from(&commit);
            merges.push(MergeOutcome {
                branch: feature.name.clone(),
                tip: feature.tip.clone(),
                commit,
                tree,
                resolution,
            });
        }
        self.validate_clean_state(main_tip, &previous)?;
        let final_tree = self.read_text(["rev-parse", "HEAD^{tree}"], "finalTree")?;
        if let Some(last) = merges.last() {
            if last.tree != final_tree {
                return Err(validation_error("finalTree"));
            }
        } else {
            let base_tree =
                self.read_text(["rev-parse", &format!("{main_tip}^{{tree}}")], "baseTree")?;
            if base_tree != final_tree || previous != main_tip {
                return Err(validation_error("finalTree"));
            }
        }
        Ok(ReconstructionResult::Complete(Reconstruction {
            merges,
            final_tree,
            preview_commit: previous,
        }))
    }

    pub(super) fn train_resolutions(
        &self,
        snapshot: &RestackSnapshot,
        retained: &[BranchIdentity],
        author: &RestackAuthor,
    ) -> Result<(), CliError> {
        for feature in &snapshot.features {
            if !retained
                .iter()
                .any(|retained| retained.name == feature.name)
            {
                continue;
            }
            for historical in &feature.historical_merges {
                self.run_success(
                    [
                        "checkout",
                        "--detach",
                        "--quiet",
                        &historical.first_parent,
                        "--",
                    ],
                    "trainingCheckout",
                )?;
                let merge = self
                    .command()
                    .args([
                        "merge",
                        "--no-ff",
                        "--no-commit",
                        "--no-edit",
                        "--no-gpg-sign",
                        &historical.feature_parent,
                    ])
                    .env("GIT_AUTHOR_NAME", &author.name)
                    .env("GIT_AUTHOR_EMAIL", &author.email)
                    .env("GIT_COMMITTER_NAME", &author.name)
                    .env("GIT_COMMITTER_EMAIL", &author.email)
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .output()
                    .map_err(|_| reconstruction_error("trainingMerge"))?;
                if merge.status.success() {
                    self.run_success(
                        ["reset", "--hard", "--quiet", &historical.first_parent],
                        "trainingReset",
                    )?;
                    continue;
                }
                if self.unresolved_paths()?.is_empty() {
                    return Err(reconstruction_error("trainingMerge"));
                }
                self.run_success(["rerere"], "trainingPreimage")?;
                self.run_success(
                    ["checkout", "--quiet", &historical.commit, "--", "."],
                    "trainingResolution",
                )?;
                let accepted_tree = self.read_text(["write-tree"], "trainingTree")?;
                if accepted_tree != historical.tree {
                    return Err(validation_error("trainingTree"));
                }
                self.run_success(["rerere"], "trainingPostimage")?;
                self.run_success(
                    ["reset", "--hard", "--quiet", &historical.first_parent],
                    "trainingReset",
                )?;
            }
        }
        Ok(())
    }

    pub(super) fn complete_manual_merge(
        &self,
        expected_head: &str,
        expected_head_reflog: &str,
        feature: &BranchIdentity,
        environment: &str,
        author: &RestackAuthor,
    ) -> Result<MergeOutcome, CliError> {
        let head = self.read_text(["rev-parse", "HEAD"], "resumeHead")?;
        if head != expected_head {
            return Err(session_state_error("agentCommit"));
        }
        if self.head_reflog_digest()? != expected_head_reflog {
            return Err(session_state_error("agentCommit"));
        }
        let merge_head = self.read_text(["rev-parse", "MERGE_HEAD"], "resumeMergeHead")?;
        if merge_head != feature.tip {
            return Err(session_state_error("mergeParent"));
        }
        if !self.unresolved_paths()?.is_empty() {
            return Err(session_state_error("resolutionNotStaged"));
        }
        let unstaged = self.read_bytes(["diff", "--name-only", "-z"], "unstagedState")?;
        let untracked = self.read_bytes(
            ["ls-files", "--others", "--exclude-standard", "-z"],
            "untrackedState",
        )?;
        if !unstaged.is_empty() || !untracked.is_empty() {
            return Err(session_state_error("resolutionNotStaged"));
        }
        self.run_success(["rerere"], "recordResolution")?;
        if !self.rerere_remaining()?.is_empty() {
            return Err(session_state_error("resolutionNotStaged"));
        }
        self.validate_index()?;
        let tree = self.read_text(["write-tree"], "writeTree")?;
        let message = canonical_merge_message(&feature.name, environment);
        let commit = self.commit_tree(&tree, expected_head, &feature.tip, &message, author)?;
        self.run_success(["reset", "--hard", "--quiet", &commit], "resetResult")?;
        self.validate_commit(&commit, expected_head, &feature.tip, &message, author)?;
        self.validate_clean_state(expected_head, &commit)?;
        Ok(MergeOutcome {
            branch: feature.name.clone(),
            tip: feature.tip.clone(),
            commit,
            tree,
            resolution: MergeResolution::Manual,
        })
    }
}
