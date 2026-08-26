use std::path::PathBuf;

use clap::{Args, Parser, Subcommand, ValueEnum};

/// Graduate's public command-line interface.
#[derive(Debug, Parser)]
#[command(
    name = "gd",
    version,
    about = "Inspect Jira Cloud from the terminal",
    propagate_version = true
)]
pub(crate) struct Cli {
    /// Override the configuration file (also available as GRADUATE_CONFIG).
    #[arg(long, global = true, value_name = "PATH")]
    pub(crate) config: Option<PathBuf>,

    #[command(subcommand)]
    pub(crate) command: Command,
}

/// Top-level Graduate commands.
#[derive(Debug, Subcommand)]
pub(crate) enum Command {
    /// Configure authentication for a ticket system.
    Auth(AuthArgs),
    /// Show feature branches in an environment that have not reached main.
    Diff(DiffArgs),
    /// Generate portable AI agent skills from Graduate's command contract.
    GenerateSkills(GenerateSkillsArgs),
    /// Review and safely publish an isolated environment reconstruction.
    #[command(hide = true)]
    Restack(RestackArgs),
}

/// Release-gated options for reviewing or applying an environment restack.
#[derive(Args)]
pub(crate) struct RestackArgs {
    /// Environment branch to reconstruct.
    #[arg(value_name = "ENVIRONMENT")]
    pub(crate) environment: String,
    /// Main branch name. Defaults to origin/HEAD, then common branch names.
    #[arg(long, value_name = "BRANCH")]
    pub(crate) main: Option<String>,
    /// Remote that owns the environment and feature branches.
    #[arg(long, value_name = "REMOTE")]
    pub(crate) remote: Option<String>,
    /// JSON parameters containing removeBranches and, for apply, planDigest.
    #[arg(long, value_name = "JSON", conflicts_with = "resume")]
    pub(crate) params: Option<String>,
    /// Publish a freshly reconstructed plan authorized by its reviewed digest.
    #[arg(long, conflicts_with = "abort")]
    pub(crate) apply: bool,
    /// Resume an isolated conflicted preview with an opaque continuation token.
    #[arg(long, value_name = "TOKEN", conflicts_with = "params")]
    pub(crate) resume: Option<String>,
    /// Delete an abandoned resumable session without changing repository refs.
    #[arg(long, requires = "resume", conflicts_with = "apply")]
    pub(crate) abort: bool,
}

impl std::fmt::Debug for RestackArgs {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RestackArgs")
            .field("environment", &self.environment)
            .field("main", &self.main)
            .field("remote", &self.remote)
            .field("params", &self.params.as_ref().map(|_| "<json>"))
            .field("apply", &self.apply)
            .field("resume", &self.resume.as_ref().map(|_| "<token>"))
            .field("abort", &self.abort)
            .finish()
    }
}

/// Options for comparing an environment branch with the main branch.
#[derive(Args)]
pub(crate) struct DiffArgs {
    /// Environment branch to inspect, such as qa, staging, or cycle.
    #[arg(value_name = "ENVIRONMENT")]
    pub(crate) environment: String,
    /// Main branch name. Defaults to origin/HEAD, then common branch names.
    #[arg(long, value_name = "BRANCH")]
    pub(crate) main: Option<String>,
    /// Remote that owns the environment and feature branches.
    #[arg(long, value_name = "REMOTE", default_value = "origin")]
    pub(crate) remote: String,
    /// Report to emit. Supplying this flag selects unattended output.
    #[arg(long, value_name = "REPORT", value_enum)]
    pub(crate) report: Option<DiffReport>,
    /// JSON parameters. Use {"branches":["feature/A","feature/B"]} to scope the report.
    #[arg(long, value_name = "JSON")]
    pub(crate) params: Option<String>,
    /// Output format. Non-interactive output defaults to json.
    #[arg(long = "format", value_name = "FORMAT", value_enum)]
    pub(crate) output_format: Option<ReportFormat>,
    /// Write formatted output to a relative file instead of stdout.
    #[arg(short = 'o', long, value_name = "PATH")]
    pub(crate) output: Option<PathBuf>,
    /// Inspect existing remote-tracking refs without fetching first.
    #[arg(long)]
    pub(crate) no_fetch: bool,
}

impl std::fmt::Debug for DiffArgs {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("DiffArgs")
            .field("environment", &self.environment)
            .field("main", &self.main)
            .field("remote", &self.remote)
            .field("report", &self.report)
            .field("params", &self.params)
            .field("output_format", &self.output_format)
            .field("output", &self.output)
            .field("no_fetch", &self.no_fetch)
            .finish()
    }
}

#[derive(Clone, Copy, Debug, ValueEnum)]
pub(crate) enum DiffReport {
    Branches,
    Age,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
pub(crate) enum ReportFormat {
    Json,
    Table,
    Yaml,
    Csv,
}

/// Authentication commands.
#[derive(Debug, Args)]
pub(crate) struct AuthArgs {
    #[command(subcommand)]
    pub(crate) command: AuthCommand,
}

/// Authentication operations.
#[derive(Debug, Subcommand)]
pub(crate) enum AuthCommand {
    /// Connect, verify, and save credentials for a ticket system.
    Setup(SetupArgs),
}

/// Ticket systems that Graduate can configure.
#[derive(Debug, Args)]
pub(crate) struct SetupArgs {
    #[command(subcommand)]
    pub(crate) system: SetupSystem,
}

/// Provider-specific authentication setup.
#[derive(Debug, Subcommand)]
pub(crate) enum SetupSystem {
    /// Configure Jira Cloud using Atlassian credentials.
    ///
    /// Interactive setup must run in a terminal. It opens a full-screen
    /// interface with three stages: Jira account details, Atlassian API token,
    /// and Review & save. Use Tab and Shift-Tab to move and Enter to continue.
    /// The Atlassian token page opens only after you enter the token stage.
    /// Escape goes back; on the Jira account page it cancels. Ctrl-C cancels
    /// from any stage. Use --from-env for unattended setup or --no-open to
    /// keep the token URL in the terminal without launching a browser.
    Jira(JiraSetupArgs),
}

/// Jira authentication setup options.
#[derive(Debug, Args)]
pub(crate) struct JiraSetupArgs {
    /// Verify and save ATLASSIAN_HOST, ATLASSIAN_EMAIL, and ATLASSIAN_TOKEN without prompting.
    #[arg(long)]
    pub(crate) from_env: bool,
    /// Print the Atlassian token URL without launching a browser.
    #[arg(long)]
    pub(crate) no_open: bool,
    /// Validate unattended setup and report planned effects without saving.
    #[arg(long, requires = "from_env")]
    pub(crate) dry_run: bool,
    /// Perform a read-only Jira check during an unattended dry-run.
    #[arg(long, requires_all = ["from_env", "dry_run"])]
    pub(crate) verify: bool,
}

/// Options for deterministic skill generation.
#[derive(Debug, Args)]
pub(crate) struct GenerateSkillsArgs {
    /// Relative directory where generated skill folders are written.
    #[arg(long, value_name = "DIR", default_value = "skills")]
    pub(crate) output_dir: PathBuf,
    /// Replace existing generated skill files.
    #[arg(long)]
    pub(crate) force: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diff_parses_google_workspace_style_output_flags() -> Result<(), clap::Error> {
        let cli = Cli::try_parse_from([
            "gd",
            "diff",
            "qa",
            "--format",
            "json",
            "--output",
            "report.json",
        ])?;
        let debug = format!("{cli:?}");

        assert!(debug.contains("Json"));
        assert!(debug.contains("report.json"));
        Ok(())
    }

    #[test]
    fn diff_selects_the_age_report_explicitly() -> Result<(), clap::Error> {
        let cli = Cli::try_parse_from(["gd", "diff", "qa", "--report", "age"])?;

        assert!(format!("{cli:?}").contains("Age"));
        Ok(())
    }

    #[test]
    fn diff_accepts_json_parameters_for_multiple_branches() -> Result<(), clap::Error> {
        let cli = Cli::try_parse_from([
            "gd",
            "diff",
            "qa",
            "--params",
            r#"{"branches":["feature/PROJ-1","feature/PROJ-2"]}"#,
        ])?;
        let debug = format!("{cli:?}");

        assert!(debug.contains("feature/PROJ-1"));
        assert!(debug.contains("feature/PROJ-2"));
        Ok(())
    }
}
