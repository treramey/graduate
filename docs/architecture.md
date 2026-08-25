# Architecture

## Workspace

```text
crates/
├── graduate/      # I/O-independent Jira services and state transitions
└── graduate-cli/  # Command parsing, terminal lifecycle, events, and rendering
```

The core crate has no terminal, filesystem, network, or process dependencies.
The CLI owns side effects and translates external events into typed core
actions.

## Command flow

1. Clap parses arguments without terminating inside application code.
2. Graduate requires an explicit top-level command.
3. Interactive Jira setup verifies terminal-capable stdin and stderr before
   Crossterm enters raw mode and the alternate screen.
4. Jira authentication events drive deterministic transitions in
   `graduate::jira_auth`.
5. The CLI performs browser, Jira, and configuration I/O at workflow seams.
6. Ratatui renders a projection of Jira authentication state to stderr.
7. A lifecycle guard restores the cursor, alternate screen, and raw mode on
   success or failure.
8. Promotion reports fetch through the Git credential boundary, inspect refs
   and commit graphs with Gitoxide, then stream branch and Jira updates into a
   Ratatui list or unattended table.

## Modules

- `graduate_cli::cli`: public command-line shape.
- `graduate_cli::terminal`: stderr terminal initialization and restoration.
- `graduate_cli::theme`: shared ANSI-palette visual language.
- `graduate_cli::error`: process-facing error categories and exit codes.
- `graduate_cli::generate_skills`: deterministic repository-controlled Agent Skill generation.
- `graduate::promotion`: authoritative environment commit inventories,
  promotion-report attribution rows, Jira enrichment states, deterministic
  branch-to-ticket mapping, and commit-age projections.
- `graduate::restack`: deterministic restack graph snapshots, explicit feature
  ordering, commit attribution, marker recognition, and unsupported-history
  evidence.
- `graduate::jira`: validated Jira sites, credentials, and identities shared by every delivery path.
- `graduate::jira_auth`: deterministic Jira onboarding state and secret-retention
  transitions.
- `graduate_cli::jira_auth`: environment and interactive Jira authentication
  orchestration behind an injected verifier.
- `graduate_cli::jira_auth_tui`: masked token input, setup navigation, review,
  and rendering.
- `graduate_cli::config`: validated Jira sites and atomic secret-restricted persistence.
- `graduate_cli::jira`: read-only Jira identity and issue-query boundary.
- `graduate_cli::diff`: Git fetch, Gitoxide graph inspection, Jira enrichment,
  unattended output, and CSV export.
- `graduate_cli::environment_git`: shared Gitoxide ref, reachability, promotion
  inventory, and restack snapshot inspection.
- `graduate_cli::diff_tui`: streaming promotion list, selection, ticket details,
  commit history, age-report modal, and open-ticket events.

## Safety invariants

- Core state changes do not perform I/O.
- Jira sites, credentials, and identities are validated in core before an I/O
  adapter can use them.
- Interactive and unattended setup share the same core credential and
  completion contracts.
- Jira setup never writes interactive application data to stdout.
- Interactive setup fails before enabling raw mode on a non-terminal.
- Terminal restoration attempts every pending cleanup step even if one fails.
- Human and TUI renderers treat external content as untrusted and prevent
  remote values from introducing terminal controls, synthetic rows, or
  misleading diagnostics.
- Tests never initialize the developer's real terminal.
- Generated skills are reproducible from the checked-in CLI generator.
- Tokens never appear in result text, errors, review screens, or debug output.
- Existing Jira tokens remain inside workflow state and are represented only
  as retainable credentials, never editable field values.
- Jira setup creates a unique same-directory temporary file with exclusive access,
  verifies mode `0600` on Unix, then atomically replaces the configuration.
- Interactive and unattended execution verify Jira before persistence;
  unattended dry-runs do not save and require explicit opt-in for networking.
- Jira response reads reject oversized `Content-Length` values and stop
  streaming at 64 KiB.
- Escape cancels an in-flight verification request. Ctrl-C cancels setup.
- Skill generation checks every destination before writing the first file.
- Configuration is versioned and stores provider-tagged connections; provider
  credentials cannot be combined across ticket systems.
- Promotion candidates are remote branch tips reachable from the environment
  branch but not from main. PATs are passed to a one-shot credential helper and
  never persisted in Git configuration.
- Aggregate promotion and age totals come from the environment-versus-main
  non-merge commit inventory. Branch and Jira rows only attribute that work;
  they cannot remove commits from aggregate totals.
- JSON-scoped promotion reports validate every requested remote branch against
  the same environment-to-main candidate rule and fail instead of silently
  omitting an invalid selection.
- Non-interactive promotion reports default to structured JSON. Alternative
  formats use the bounded `--format` surface, and `--output` rejects absolute
  paths, parent traversal, and symbolic-link destinations.
- Restack inventory accepts only uniquely mapped two-parent explicit feature
  merges and exact empty phase markers; direct work, fast-forwards, octopus
  merges, deleted feature refs, and ambiguous mappings fail with evidence.

## Distribution

Changesets synchronize Cargo and npm versions. A release tag builds checksummed
native archives for Linux, macOS, and Windows, attaches build provenance,
publishes the npm bootstrap package, and updates the Homebrew tap. The same
workspace is available as a Nix flake with locked inputs.

## Adding behavior

Put deterministic services and state transitions in `graduate`. Keep
terminal, filesystem, process, prompt, and network behavior in
`graduate-cli`. For a larger feature, place workflow in `<feature>.rs` and
rendering and terminal events in `<feature>_tui.rs`.
