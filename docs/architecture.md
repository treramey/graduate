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
2. Like Drag, Graduate requires an explicit top-level command.
3. Interactive login verifies terminal-capable stdin and stderr before
   Crossterm enters raw mode and the alternate screen.
4. Login events drive deterministic transitions in `graduate::login`.
5. The CLI performs browser, Jira, and configuration I/O at workflow seams.
6. Ratatui renders a projection of login state to stderr.
7. A lifecycle guard restores the cursor, alternate screen, and raw mode on
   success or failure.

## Modules

- `graduate_cli::cli`: public command-line shape.
- `graduate_cli::terminal`: stderr terminal initialization and restoration.
- `graduate_cli::theme`: shared ANSI-palette visual language.
- `graduate_cli::error`: process-facing error categories and exit codes.
- `graduate_cli::generate_skills`: deterministic repository-controlled Agent Skill generation.
- `graduate::jira`: validated Jira sites, credentials, and identities shared by every delivery path.
- `graduate::login`: deterministic Jira onboarding state and secret-retention transitions.
- `graduate_cli::login`: environment and interactive login orchestration behind an injected verifier.
- `graduate_cli::login_tui`: masked token input, login navigation, review, and rendering.
- `graduate_cli::config`: validated Jira sites and atomic secret-restricted persistence.
- `graduate_cli::jira`: read-only Jira identity verification and future ticket-query boundary.

## Safety invariants

- Core state changes do not perform I/O.
- Jira sites, credentials, and identities are validated in core before an I/O
  adapter can use them.
- Interactive and unattended login share the same core credential and
  completion contracts.
- Login never writes interactive application data to stdout.
- Interactive login fails before enabling raw mode on a non-terminal.
- Terminal restoration attempts every pending cleanup step even if one fails.
- Tests never initialize the developer's real terminal.
- Generated skills are reproducible from the checked-in CLI generator.
- Tokens never appear in result text, errors, review screens, or debug output.
- Existing login tokens remain inside workflow state and are represented only
  as retainable credentials, never editable field values.
- Login creates a unique same-directory temporary file with exclusive access,
  verifies mode `0600` on Unix, then atomically replaces the configuration.
- Interactive and unattended execution verify Jira before persistence;
  unattended dry-runs do not save and require explicit opt-in for networking.
- Jira identity reads reject oversized `Content-Length` values and stop
  streaming at 64 KiB.
- Escape cancels an in-flight verification request. Ctrl-C cancels login.
- Skill generation checks every destination before writing the first file.

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
