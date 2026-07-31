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
}

/// Options for comparing an environment branch with the main branch.
#[derive(Args)]
pub(crate) struct DiffArgs {
    /// Environment branch to inspect, such as qa, staging, or cycle.
    #[arg(value_name = "ENVIRONMENT")]
    pub(crate) environment: String,
    /// Main branch name. By default, use origin/HEAD, then try common names.
    #[arg(long, value_name = "BRANCH")]
    pub(crate) main: Option<String>,
    /// Remote that owns the environment and feature branches.
    #[arg(long, value_name = "REMOTE", default_value = "origin")]
    pub(crate) remote: String,
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
            .field("output_format", &self.output_format)
            .field("output", &self.output)
            .field("no_fetch", &self.no_fetch)
            .finish()
    }
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
    /// Interactive setup requires terminal-capable stdin and stderr and opens
    /// Ratatui for Jira account details, Atlassian API token, and Review & save.
    /// Use Tab and Shift-Tab to move and Enter to continue. The Atlassian token
    /// page opens only after you explicitly enter the token stage. Escape goes
    /// back, or cancels from Jira account details; Ctrl-C cancels from any
    /// stage. Use --from-env for unattended setup or --no-open to keep the
    /// token URL in the terminal without launching a browser.
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
}
