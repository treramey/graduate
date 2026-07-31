mod browser;
mod cli;
mod config;
mod error;
mod generate_skills;
mod jira;
mod login;
mod login_tui;
mod terminal;
mod terminal_text;
mod theme;

use std::process::ExitCode;

use clap::Parser;
use cli::{Cli, Command};
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
        Command::Login(args) => {
            let path = cli.config.unwrap_or(config::config_path()?);
            login::run(args, &path).await
        }
        Command::GenerateSkills(args) => generate_skills::run(&args),
    }
}
