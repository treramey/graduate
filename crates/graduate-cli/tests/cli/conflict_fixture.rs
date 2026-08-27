//! Conflicting restack repository fixture.

use std::error::Error;
use std::path::{Path, PathBuf};

use assert_cmd::Command;

use crate::common::{gd_command, isolate_gd_storage, structured_restack_error};
use crate::git::{git_text, isolated_git, path_text, run_git};

pub(crate) struct ConflictRestackFixture {
    pub(super) _directory: tempfile::TempDir,
    pub(super) source: PathBuf,
    pub(super) remote: PathBuf,
    pub(super) global: PathBuf,
    cache: PathBuf,
}

impl ConflictRestackFixture {
    pub(crate) fn new() -> Result<Self, Box<dyn Error>> {
        let directory = tempfile::tempdir()?;
        let source = directory.path().join("source");
        let remote = directory.path().join("remote.git");
        let global = directory.path().join("global.gitconfig");
        let cache = directory.path().join("cache");
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
        std::fs::write(source.join("conflict"), "base\n")?;
        run_git(&source, &global, &["add", "conflict"])?;
        run_git(&source, &global, &["commit", "-q", "-m", "base"])?;
        run_git(&source, &global, &["push", "-q", "-u", "origin", "main"])?;
        run_git(
            &source,
            &global,
            &["checkout", "-q", "-b", "feature/conflict", "main"],
        )?;
        std::fs::write(source.join("conflict"), "feature\n")?;
        run_git(&source, &global, &["commit", "-q", "-am", "feature"])?;
        run_git(
            &source,
            &global,
            &["push", "-q", "-u", "origin", "feature/conflict"],
        )?;
        run_git(&source, &global, &["checkout", "-q", "main"])?;
        std::fs::write(source.join("conflict"), "main old\n")?;
        run_git(&source, &global, &["commit", "-q", "-am", "main old"])?;
        run_git(&source, &global, &["push", "-q", "origin", "main"])?;
        run_git(&source, &global, &["checkout", "-q", "-b", "qa", "main"])?;
        let merge = isolated_git(&global)
            .current_dir(&source)
            .args(["merge", "--no-ff", "--no-commit", "feature/conflict"])
            .output()?;
        if merge.status.success() {
            return Err("fixture merge unexpectedly succeeded".into());
        }
        std::fs::write(source.join("conflict"), "accepted\n")?;
        run_git(&source, &global, &["add", "conflict"])?;
        run_git(
            &source,
            &global,
            &["commit", "-q", "-m", "accepted conflict"],
        )?;
        run_git(&source, &global, &["push", "-q", "-u", "origin", "qa"])?;
        run_git(&source, &global, &["checkout", "-q", "main"])?;
        Ok(Self {
            _directory: directory,
            source,
            remote,
            global,
            cache,
        })
    }

    fn command(&self) -> Result<Command, Box<dyn Error>> {
        let mut command = gd_command()?;
        isolate_gd_storage(&mut command, &self.cache);
        Ok(command)
    }

    pub(crate) fn preview(&self) -> Result<std::process::Output, Box<dyn Error>> {
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
            ])
            .env("GIT_CONFIG_GLOBAL", &self.global)
            .output()?)
    }

    pub(crate) fn resume(
        &self,
        token: &str,
        environment: &str,
        source: &Path,
    ) -> Result<std::process::Output, Box<dyn Error>> {
        Ok(self
            .command()?
            .current_dir(source)
            .args(["restack", environment, "--resume", token])
            .env("GIT_CONFIG_GLOBAL", &self.global)
            .output()?)
    }

    pub(crate) fn seal_manual_resolution(
        &self,
    ) -> Result<(String, PathBuf, serde_json::Value), Box<dyn Error>> {
        self.advance_main()?;
        let conflict = structured_restack_error(self.preview()?)?;
        let token = conflict["details"]["resumeToken"]
            .as_str()
            .ok_or("resume token")?
            .to_owned();
        let work_area = PathBuf::from(
            conflict["details"]["workArea"]
                .as_str()
                .ok_or("work area")?,
        );
        std::fs::write(work_area.join("conflict"), "manual resolution\n")?;
        self.git(&work_area, &["add", "conflict"])?;
        let output = self.resume(&token, "qa", &self.source)?;
        if !output.status.success() {
            return Err(format!(
                "restack resume failed: {}",
                String::from_utf8_lossy(&output.stderr)
            )
            .into());
        }
        let plan = serde_json::from_slice(&output.stdout)?;
        Ok((token, work_area, plan))
    }

    pub(crate) fn resume_apply(&self, token: &str) -> Result<std::process::Output, Box<dyn Error>> {
        Ok(self
            .command()?
            .current_dir(&self.source)
            .args(["restack", "qa", "--resume", token, "--apply"])
            .env("GIT_CONFIG_GLOBAL", &self.global)
            .output()?)
    }

    #[cfg(unix)]
    pub(crate) fn resume_apply_with_path(
        &self,
        token: &str,
        path: &std::ffi::OsStr,
    ) -> Result<std::process::Output, Box<dyn Error>> {
        Ok(self
            .command()?
            .current_dir(&self.source)
            .args(["restack", "qa", "--resume", token, "--apply"])
            .env("GIT_CONFIG_GLOBAL", &self.global)
            .env("PATH", path)
            .output()?)
    }

    pub(crate) fn abort(&self, token: &str) -> Result<std::process::Output, Box<dyn Error>> {
        Ok(self
            .command()?
            .current_dir(&self.source)
            .args(["restack", "qa", "--resume", token, "--abort"])
            .env("GIT_CONFIG_GLOBAL", &self.global)
            .output()?)
    }

    pub(crate) fn advance_main(&self) -> Result<(), Box<dyn Error>> {
        std::fs::write(self.source.join("conflict"), "main new\n")?;
        self.git(&self.source, &["commit", "-q", "-am", "main new"])?;
        self.git(&self.source, &["push", "-q", "origin", "main"])
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
