//! Clean restack repository fixture.

use std::error::Error;
use std::path::{Path, PathBuf};

use assert_cmd::Command;

use crate::common::{gd_command, isolate_gd_storage};
use crate::git::{git_text, path_text, run_git};

pub(crate) struct RestackFixture {
    pub(super) directory: tempfile::TempDir,
    pub(super) source: PathBuf,
    pub(super) remote: PathBuf,
    pub(super) global: PathBuf,
    cache: PathBuf,
}

impl RestackFixture {
    pub(crate) fn new() -> Result<Self, Box<dyn Error>> {
        let directory = tempfile::tempdir()?;
        let source = directory.path().join("source");
        let remote = directory.path().join("remote.git");
        let global = directory.path().join("global.gitconfig");
        std::fs::write(&global, [])?;
        std::fs::create_dir(&source)?;
        std::fs::create_dir(&remote)?;
        run_git(&remote, &global, &["init", "--bare", "-q"])?;
        run_git(
            &remote,
            &global,
            &["symbolic-ref", "HEAD", "refs/heads/main"],
        )?;
        run_git(&source, &global, &["init", "-q", "-b", "main"])?;
        run_git(&source, &global, &["config", "user.name", "Fixture Author"])?;
        run_git(
            &source,
            &global,
            &["config", "user.email", "fixture@example.com"],
        )?;
        run_git(
            &source,
            &global,
            &["remote", "add", "origin", path_text(&remote)?],
        )?;
        std::fs::write(source.join("base"), "base\n")?;
        std::fs::write(source.join(".gitattributes"), "base filter=hostile\n")?;
        run_git(&source, &global, &["add", "base", ".gitattributes"])?;
        run_git(&source, &global, &["commit", "-q", "-m", "base"])?;
        run_git(&source, &global, &["push", "-q", "-u", "origin", "main"])?;
        run_git(
            &source,
            &global,
            &["checkout", "-q", "-b", "feature/a", "main"],
        )?;
        std::fs::write(source.join("feature-a"), "feature a\n")?;
        run_git(&source, &global, &["add", "feature-a"])?;
        run_git(&source, &global, &["commit", "-q", "-m", "feature a"])?;
        run_git(
            &source,
            &global,
            &["push", "-q", "-u", "origin", "feature/a"],
        )?;
        run_git(&source, &global, &["checkout", "-q", "-b", "qa", "main"])?;
        run_git(
            &source,
            &global,
            &[
                "merge",
                "-q",
                "--no-ff",
                "feature/a",
                "-m",
                "accepted feature a",
            ],
        )?;
        run_git(
            &source,
            &global,
            &["commit", "-q", "--allow-empty", "-m", "### Match 'qa'"],
        )?;
        run_git(&source, &global, &["push", "-q", "-u", "origin", "qa"])?;
        run_git(&source, &global, &["checkout", "-q", "main"])?;
        Ok(Self {
            cache: directory.path().join("cache"),
            directory,
            source,
            remote,
            global,
        })
    }

    /// Add `feature/b`, merge `qa` into it, then promote it into `qa`.
    pub(crate) fn add_environment_merging_feature(&self) -> Result<(), Box<dyn Error>> {
        self.git(&self.source, &["checkout", "-q", "-b", "feature/b", "main"])?;
        std::fs::write(self.source.join("feature-b"), "feature b\n")?;
        self.git(&self.source, &["add", "feature-b"])?;
        self.git(&self.source, &["commit", "-q", "-m", "feature b"])?;
        self.git(
            &self.source,
            &["merge", "-q", "--no-ff", "qa", "-m", "sync qa"],
        )?;
        self.git(&self.source, &["push", "-q", "-u", "origin", "feature/b"])?;
        self.git(&self.source, &["checkout", "-q", "qa"])?;
        self.git(
            &self.source,
            &[
                "merge",
                "-q",
                "--no-ff",
                "feature/b",
                "-m",
                "accepted feature b",
            ],
        )?;
        self.git(&self.source, &["push", "-q", "origin", "qa"])?;
        self.git(&self.source, &["checkout", "-q", "main"])
    }

    pub(crate) fn dry_run(&self) -> Result<std::process::Output, Box<dyn Error>> {
        Ok(self
            .command()?
            .current_dir(&self.source)
            .args(["restack", "qa", "--main", "main", "--dry-run"])
            .env("GIT_CONFIG_GLOBAL", &self.global)
            .output()?)
    }

    pub(crate) fn command(&self) -> Result<Command, Box<dyn Error>> {
        let mut command = gd_command()?;
        isolate_gd_storage(&mut command, &self.cache);
        Ok(command)
    }

    pub(crate) fn preview(
        &self,
        removals: &[&str],
    ) -> Result<std::process::Output, Box<dyn Error>> {
        self.preview_with_environment(removals, None)
    }

    pub(crate) fn preview_with_git_config_parameters(
        &self,
        removals: &[&str],
        parameters: &str,
    ) -> Result<std::process::Output, Box<dyn Error>> {
        self.preview_with_environment(removals, Some(parameters))
    }

    fn preview_with_environment(
        &self,
        removals: &[&str],
        git_config_parameters: Option<&str>,
    ) -> Result<std::process::Output, Box<dyn Error>> {
        let params = serde_json::json!({"removeBranches": removals}).to_string();
        let mut command = self.command()?;
        command
            .current_dir(&self.source)
            .args(["restack", "qa", "--main", "main", "--params", &params])
            .env("GIT_CONFIG_GLOBAL", &self.global);
        if let Some(parameters) = git_config_parameters {
            command.env("GIT_CONFIG_PARAMETERS", parameters);
        }
        Ok(command.output()?)
    }

    pub(crate) fn apply(
        &self,
        removals: &[&str],
        digest: &str,
    ) -> Result<std::process::Output, Box<dyn Error>> {
        let params = serde_json::json!({
            "removeBranches": removals,
            "planDigest": digest,
        })
        .to_string();
        Ok(self
            .command()?
            .current_dir(&self.source)
            .args([
                "restack", "qa", "--main", "main", "--params", &params, "--apply",
            ])
            .env("GIT_CONFIG_GLOBAL", &self.global)
            .output()?)
    }

    #[cfg(unix)]
    pub(crate) fn apply_with_path(
        &self,
        removals: &[&str],
        digest: &str,
        path: &std::ffi::OsStr,
    ) -> Result<std::process::Output, Box<dyn Error>> {
        let params = serde_json::json!({
            "removeBranches": removals,
            "planDigest": digest,
        })
        .to_string();
        Ok(self
            .command()?
            .current_dir(&self.source)
            .args([
                "restack", "qa", "--main", "main", "--params", &params, "--apply",
            ])
            .env("GIT_CONFIG_GLOBAL", &self.global)
            .env("PATH", path)
            .output()?)
    }

    pub(crate) fn apply_without_digest(&self) -> Result<std::process::Output, Box<dyn Error>> {
        Ok(self
            .command()?
            .current_dir(&self.source)
            .args([
                "restack",
                "qa",
                "--main",
                "main",
                "--params",
                r#"{"removeBranches":[]}"#,
                "--apply",
            ])
            .env("GIT_CONFIG_GLOBAL", &self.global)
            .output()?)
    }

    pub(crate) fn advance_feature_from_separate_clone(&self) -> Result<String, Box<dyn Error>> {
        let writer = self.directory.path().join("writer");
        run_git(
            self.directory.path(),
            &self.global,
            &["clone", "-q", path_text(&self.remote)?, path_text(&writer)?],
        )?;
        run_git(
            &writer,
            &self.global,
            &["config", "user.name", "Other Author"],
        )?;
        run_git(
            &writer,
            &self.global,
            &["config", "user.email", "other@example.com"],
        )?;
        run_git(&writer, &self.global, &["checkout", "-q", "feature/a"])?;
        std::fs::write(writer.join("feature-a-two"), "feature a two\n")?;
        run_git(&writer, &self.global, &["add", "feature-a-two"])?;
        run_git(&writer, &self.global, &["commit", "-q", "-m", "advance a"])?;
        run_git(
            &writer,
            &self.global,
            &["push", "-q", "origin", "feature/a"],
        )?;
        self.git_text(&writer, &["rev-parse", "HEAD"])
    }

    pub(crate) fn advance_main_from_separate_clone(&self) -> Result<String, Box<dyn Error>> {
        let writer = self.directory.path().join("main-writer");
        run_git(
            self.directory.path(),
            &self.global,
            &["clone", "-q", path_text(&self.remote)?, path_text(&writer)?],
        )?;
        run_git(
            &writer,
            &self.global,
            &["config", "user.name", "Other Author"],
        )?;
        run_git(
            &writer,
            &self.global,
            &["config", "user.email", "other@example.com"],
        )?;
        std::fs::write(writer.join("main-two"), "main two\n")?;
        run_git(&writer, &self.global, &["add", "main-two"])?;
        run_git(
            &writer,
            &self.global,
            &["commit", "-q", "-m", "advance main"],
        )?;
        run_git(&writer, &self.global, &["push", "-q", "origin", "main"])?;
        self.git_text(&writer, &["rev-parse", "HEAD"])
    }

    pub(crate) fn advance_environment_from_separate_clone(&self) -> Result<String, Box<dyn Error>> {
        let writer = self.directory.path().join("environment-writer");
        run_git(
            self.directory.path(),
            &self.global,
            &["clone", "-q", path_text(&self.remote)?, path_text(&writer)?],
        )?;
        run_git(
            &writer,
            &self.global,
            &["config", "user.name", "Other Author"],
        )?;
        run_git(
            &writer,
            &self.global,
            &["config", "user.email", "other@example.com"],
        )?;
        run_git(&writer, &self.global, &["checkout", "-q", "qa"])?;
        run_git(
            &writer,
            &self.global,
            &["commit", "-q", "--allow-empty", "-m", "### Match 'qa'"],
        )?;
        run_git(&writer, &self.global, &["push", "-q", "origin", "qa"])?;
        self.git_text(&writer, &["rev-parse", "HEAD"])
    }

    pub(crate) fn git(&self, path: &Path, arguments: &[&str]) -> Result<(), Box<dyn Error>> {
        run_git(path, &self.global, arguments)
    }

    pub(crate) fn git_text(
        &self,
        path: &Path,
        arguments: &[&str],
    ) -> Result<String, Box<dyn Error>> {
        git_text(path, &self.global, arguments)
    }
}
