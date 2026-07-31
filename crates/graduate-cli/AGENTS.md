## Terminal behavior

- Render interactive output to stderr and preserve stdout for future structured command output.
- Verify terminal eligibility before entering raw mode.
- Keep terminal lifecycle cleanup centralized in `terminal.rs`.
- Test views with Ratatui's `TestBackend`; never initialize a real terminal in tests.

## Side-effect tests

- Exercise login networking through the injected `ConnectionVerifier`; tests
  must never contact live Jira services.
- Clear all four connection/configuration environment variables in CLI
  subprocess helpers before setting scenario-specific values.
- Keep tokens out of snapshots, assertion diagnostics, and rendered buffers.

## Feature layout

- Keep feature workflow in `<feature>.rs` and terminal state, events, and rendering in a flat `<feature>_tui.rs` sibling.
- Translate Crossterm events into deterministic core login transitions before changing workflow state.
