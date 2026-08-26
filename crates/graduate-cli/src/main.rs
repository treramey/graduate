mod browser;
mod cli;
mod config;
mod diff;
mod diff_tui;
mod environment_git;
mod error;
mod generate_skills;
mod git_process;
mod jira;
mod jira_auth;
mod jira_auth_tui;
mod restack;
mod restack_session;
mod restack_tui;
mod terminal;
mod terminal_text;
mod theme;

use std::process::ExitCode;

use clap::Parser;
use cli::{AuthCommand, Cli, Command, SetupSystem};
use error::{CliError, MachineError};

#[tokio::main]
async fn main() -> ExitCode {
    let restack_invocation = is_restack_invocation();
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(error) => {
            let exit_code = error.exit_code();
            if restack_invocation && exit_code != 0 {
                let error = MachineError::usage(
                    "invalid_usage",
                    "the restack machine invocation is not valid",
                    serde_json::json!({}),
                );
                eprintln!("{error}");
                return ExitCode::from(error.exit_code());
            }
            let _ = error.print();
            return ExitCode::from(u8::try_from(exit_code).unwrap_or(error::EXIT_USAGE));
        }
    };

    match run(cli).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            if error.is_machine() {
                eprintln!("{error}");
            } else {
                eprintln!("error: {error}");
            }
            ExitCode::from(error.exit_code())
        }
    }
}

fn is_restack_invocation() -> bool {
    let mut arguments = std::env::args_os().skip(1);
    while let Some(argument) = arguments.next() {
        if argument == "--config" {
            let _ = arguments.next();
            continue;
        }
        if argument
            .to_str()
            .is_some_and(|argument| argument.starts_with("--config="))
        {
            continue;
        }
        if argument == "restack" {
            return true;
        }
        if !argument.to_string_lossy().starts_with('-') {
            return false;
        }
    }
    false
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
        Command::Restack(args) => restack::run(args),
    }
}
