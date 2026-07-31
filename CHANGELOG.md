# Changelog

All notable changes to this project will be documented in this file.

## [Unreleased]

### Changed

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
- Named the Jira authentication command `gd login`.
- Reduced the login interface's minimum width to 76 columns so it works in
  common split-pane layouts.
- Standardized the product-facing brand as Graduate.

### Added

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
