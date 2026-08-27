# Changelog

All notable changes to this project will be documented in this file.

## [Unreleased]

### Added

- `gd restack` can now rebuild an environment whose history cannot be read.
  When the history proof fails, the interactive flow explains the blocking
  commit and offers to rebuild from inventory: membership from remote tips
  reachable from the environment but not main, oldest tip merged first, no
  reused conflict resolutions, and every commit no retained branch contains
  listed as dropped before publishing. The `restackPlan` schema is now version
  2 with `inventory`, `carriedBranches`, `orphanedCommits`, and
  `effects.reusedResolutions`; the machine path still reports
  `unsupported_history`.
- Added `gd schema restack` and an explicit `gd restack --dry-run` machine
  preview. A dry-run without `--params` safely retains every discovered
  feature, while `--params` can still select exact removals.
- Added `gd describe restack --json` for side-effect-free runtime discovery of
  restack arguments, payload schemas, execution modes, result kinds,
  validation rules, and security invariants.
- Added `gd restack <environment>` for interactive and agent-reviewed rebuilds
  of ephemeral integration branches. It uses isolated unsigned reconstruction,
  reusable conflict resolutions, resumable conflict handoff, immutable plan
  digests, fresh ref and endpoint validation, and exact leased publication.
- Added `gd diff --params '{"branches":[...]}'` for agent-friendly branch and
  age reports scoped to several exact feature branches, with strict validation
  for missing, unpromoted, or already-graduated selections.
- Added an advanced `gd diff --report age` report with authored-date-derived
  year buckets, explicit 90-day and one-year decision thresholds,
  oldest-branch details, structured JSON/YAML/CSV output, and an `a` hotkey
  that opens the same scrollable report in a TUI modal.
- Recovered promoted work whose feature branch was deleted after its pull
  request completed. The promotion report now also groups the non-merge
  commits unique to the environment by the Jira key in their subject and
  adds one row per key that no surviving branch already covers, so tickets
  merged from since-deleted branches appear instead of vanishing. Merge
  commits are skipped, so environment sync merges never produce rows.

### Fixed

- `gd restack` no longer fails reconstruction with `stagedDiffCheck` when a
  feature branch contains trailing whitespace or similar whitespace errors.
  The isolated `git diff --check` now rejects only leftover conflict markers.
- `gd restack` inspection no longer walks every remote branch's full history
  back to the root. Feature walks stop at commits already reachable from main
  and only the commits the reconstruction proof reads are loaded, taking a
  3,700-branch repository from about 90 seconds to under 2 seconds.
- Interactive `gd restack` failures now include the structured details that
  name the blocking merge commit or ambiguous branches.
- Made the final restack checkpoint harder to trigger accidentally: publication
  now requires `Ctrl+Y`, rewrite scope and collaborator impact are explicit,
  lease protection leads with its plain-language outcome, and Back versus
  Abandon labels state what each exit preserves.
- Prevented undersized restack screens from accepting hidden progression or
  publication actions, made wrapped-height safety checks match Ratatui, fixed
  upward navigation after jumping to the end of Review, and clarified that
  excluded features are omitted from the environment rather than deleted.
- Made restack publication decisions explicit: review now explains the effect
  and recoverability of omitted features, translates lease and signing policy
  into plain-language consequences, puts omissions before retained details on
  compact terminals, names the publish-confirmation action, and repeats
  reviewed impact at the final publication checkpoint. The bounded confirmation labels
  current and reviewed tips, previews up to three omitted branches, distinguishes
  Back from Cancel, and removes repetitive and low-value safety copy. Plan
  details advertise the exact evidence they reveal, while redundant target,
  binding, and signing rows no longer compete with the primary review decision.
  Reviews with hundreds of retained features keep plan details near the top and
  support Home/End jumps across the full merge order.
- Hardened the interactive restack TUI at its safety boundaries: minimum-size
  confirmation keeps the publish instruction visible, selected dependencies
  name the retained features blocking removal, conflict handoff explains the
  edit-stage-resume sequence and forbids manual commits, and technical review
  exposes exact feature identities. Selection now keeps its primary controls
  concise, reveals bulk and navigation shortcuts on demand, opens filtering in
  a lazygit-style footer prompt, reports match counts, and aligns wide Unicode
  branch names by terminal columns.
- Preserved Jira keys and reusable conflict-resolution evidence in compact
  restack checklists, added a safe undersized-terminal state, exposed workflow
  progress, branch filtering, and complete navigation controls, and made large
  feature inventories faster to traverse and select in batches. Wide terminals
  now use denser one-line evidence rows, while review keeps plan bindings visible
  and the checklist defines every state symbol inline. Narrow terminals keep
  complete dependency-rejection guidance visible instead of clipping the
  blocking branch.
- Rejected percent-encoded octets in Git ref components and documented that
  agents and repository-derived content are untrusted inputs.
- Kept the interactive restack checklist viewport stable while navigating long
  feature lists.
- Reworked interactive restack selection and review around compact workflow
  headers, aligned feature columns, dependency markers, live selection totals,
  an impact-first rewrite summary, semantic merge outcomes, optional technical
  details, and clearer staged-action controls.
- Flagged environment merges that an environment rebuild or pull-style
  self-merge moved off the environment branch's first-parent line, so stale
  feature branches carrying older environment history keep their
  `mergedEnvironments` warning. Merge subjects that name an environment as
  the merge target or source both count, so a `--no-ff` merge of an
  environment into a feature branch stays flagged even after the environment
  branch is reset.

### Changed

- Simplified the interactive Jira setup around compact inline prompts, a plain
  review table, Quickshell-style control tiles, and contextual controls inspired
  by Omarchy Quattro, while
  reducing its minimum terminal size to 60 columns by 24 rows and supporting
  arrow-key navigation. Added
  `pnpm test:setup` for safely exercising the flow with a local verifier.
- Based promotion and age-report totals on the authoritative environment versus
  main commit inventory, exposed ahead/behind inventories in structured output,
  and called out environments that are behind main in the completed TUI.
- Standardized the Git-history and age-report modals on the same wide content
  viewport, while preserving responsive narrowing on smaller terminals.
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
