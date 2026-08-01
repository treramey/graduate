# Changelog

All notable changes to this project will be documented in this file.

## [Unreleased]

### Changed

- Kept promotion-list navigation moving smoothly through a scrolled viewport
  and cleared stale Jira ticket warnings when selecting another branch.
- Hardened promotion report completion, Jira request concurrency, branch-key
  parsing, CSV error fields, main-branch fallback, and atomic report exports.
- Added interruption recovery and mutation-boundary validation to generated
  Agent Skill publication.
- Installed the CLI executable as `gd`.
- Named the CLI and core Cargo packages `graduate-cli` and `graduate`, with
  crate directories aligned to their package names.
- Standardized the npm package, Nix package, Homebrew formula, release
  artifacts, configuration namespace, and generated skill on `graduate`.
- Reworked command help around Graduate's Jira workflow and documented the guided
  login controls in the same direct style as Drag.
- Matched Drag's interactive login fields, focusable actions, responsive
  layout, status treatments, and motion while omitting Tempo authentication.
- Matched Drag's required-command behavior and removed the placeholder
  Home/Help terminal shell.
- Moved Jira validation, credential and identity contracts, and shared login
  completion policy into the `graduate` core crate.
- Moved Jira authentication to `gd auth setup jira` and introduced versioned,
  provider-tagged connection configuration for future ticket systems.
- Reduced the login interface's minimum width to 76 columns so it works in
  common split-pane layouts.
- Standardized the product-facing brand as Graduate.
- Made generated Agent Skill updates transactional and rejected output paths
  that escape the repository or traverse symbolic links.

### Added

- Added an `h` shortcut to the interactive promotion report for viewing the
  selected branch's Git history ahead of the resolved main branch.
- Added `gd diff <environment>`, a streaming promotion report for feature
  branches that reached an environment but not main, with Jira enrichment,
  ticket opening, API-native JSON/table/YAML/CSV output, safe file export, and
  environment-only PAT authentication.
- Initial Rust workspace with an I/O-independent core crate and a Ratatui CLI.
- Unit, rendering, and CLI integration tests.
- Deterministic repository-controlled Agent Skill generation and validation.
- Changesets-driven versioning, native release archives with provenance, npm,
  Homebrew, Nix, dependency updates, and scheduled security auditing.
- Interactive and unattended Jira login using Drag's Atlassian API-token
  authentication method, read-only identity verification, masked token entry,
  retained credentials, and atomic secret-restricted configuration storage.
- Bounded Jira identity reads, cancellable verification, secure unique
  configuration staging, transactional skill preflight, and locked Nix inputs.
