mod browser;
mod cli;
mod config;
mod diff;
mod diff_tui;
mod error;
mod generate_skills;
mod jira;
mod jira_auth;
mod jira_auth_tui;
mod terminal;
mod terminal_text;
mod theme;

use std::process::ExitCode;

use clap::Parser;
use cli::{AuthCommand, Cli, Command, SetupSystem};
use error::CliError;

#[tokio::main]
async fn main() -> ExitCode {
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(error) => {
            let exit_code = error.exit_code();
            let _ = error.print();
            return ExitCode::from(u8::try_from(exit_code).unwrap_or(error::EXIT_USAGE));
        }
    };

    match run(cli).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::from(error.exit_code())
        }
    }
}

async fn run(cli: Cli) -> Result<(), CliError> {
    match cli.command {
        Command::Auth(args) => {
            let path = cli.config.unwrap_or(config::config_path()?);
            match args.command {
                AuthCommand::Setup(args) => match args.system {
                    SetupSystem::Jira(args) => jira_auth::run(args, &path).await,
                },
            }
        }
        Command::GenerateSkills(args) => generate_skills::run(&args),
        Command::Diff(args) => {
            let path = cli.config.unwrap_or(config::config_path()?);
            diff::run(args, &path).await
        }
    }
}
