//! Cross-cutting terminal, Git, Jira, configuration, and error infrastructure shared by every feature slice.

pub(crate) mod browser;
pub(crate) mod config;
pub(crate) mod environment_git;
pub(crate) mod error;
pub(crate) mod git_process;
pub(crate) mod jira;
pub(crate) mod terminal;
pub(crate) mod terminal_text;
pub(crate) mod theme;
