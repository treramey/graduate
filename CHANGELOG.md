# Changelog

All notable changes to this project will be documented in this file.

## [Unreleased]

### Added

- Recovered promoted work whose feature branch was deleted after its pull
  request completed. The promotion report now also groups the non-merge
  commits unique to the environment by the Jira key in their subject and
  adds one row per key that no surviving branch already covers, so tickets
  merged from since-deleted branches appear instead of vanishing. Merge
  commits are skipped, so environment sync merges never produce rows.

### Fixed

- Flagged environment merges that an environment rebuild or pull-style
  self-merge moved off the environment branch's first-parent line, so stale
  feature branches carrying older environment history keep their
  `mergedEnvironments` warning. Merge subjects that name an environment as
  the merge target or source both count, so a `--no-ff` merge of an
  environment into a feature branch stays flagged even after the environment
  branch is reset.

### Changed

- Clarified customer-facing text: rewrote the README introduction and
  promotion-report guidance, simplified CLI help and setup wizard copy,
  labeled dry-run output explicitly, and added a remediation hint to the
  Jira authentication error.
- Excluded merge commits from promotion ahead counts and branch history, so
  branches that repeatedly sync main no longer report inflated commit counts.
- Standardized missing Jira tickets to a dash in the promotion status column,
  replacing the mixed `not found` and `no ticket` labels.
- Added a one-row margin between the promotion table and the footer bar.
- Let the Git history sheet grow to the full content viewport width instead
  of capping at 90 columns.
- Added a compact master-detail promotion layout for terminals that are short
  but wide, keeping the full-width details-above-table layout on larger
  screens.
- In the compact layout, dropped the Graduate artwork, the `Promotion report`
  heading, and decorative spacing before clipping report content, and used the
  selected branch as the inspector title.
- Paired a small Graduate wordmark with a right-aligned report summary and a
  one-row margin in the compact header.
- Reworked promotion Git history into a sheet that adapts to its content,
  showing the base branch, commit count, ordering, list position, SHA, author,
  and date, with local controls.
- Moved selected-branch details above the promotion table and presented them in
  a balanced two-column card that separates Jira status from branch metadata.
- Open the promotion-report interface before fetching remote branches.
- Left-aligned promotion table headings and the selected-branch card.
- Show Jira issue lookups that return HTTP 404 with a `not found` status.
- Removed the unnecessary authentication-window notice before interactive
  promotion-report fetches.
- Added compact Graduate artwork, balanced spacing, and an adjacent version to
  the terminal header.
- Added cursor movement and Unicode-aware editing to Jira setup fields.
- Kept promotion-list navigation moving smoothly through a scrolled viewport
  and cleared stale Jira ticket warnings when selecting another branch. Raised
  ticket details and added space between them and the branch table.
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
