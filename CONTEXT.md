# Agent usage context

- Like Drag, `gd` requires an explicit command. Run `gd --help` to inspect the
  command contract.
- `gd login` owns the current terminal interface. It requires terminal-capable
  stdin and stderr and renders to stderr.
- Keep login state deterministic and independent of Ratatui and Crossterm.
- Keep validated Jira sites, credentials, identities, and login acceptance
  rules in `graduate`; adapters may only consume those contracts.
- Never initialize a real terminal in tests. Render with `TestBackend` and inject state transitions directly.
- Regenerate repository-controlled skills with `gd generate-skills --force` whenever the public CLI changes.
- `gd login` uses Jira Cloud Basic authentication with the Jira site,
  Atlassian account email, and API token; it never prints the token.
- Use `gd login --from-env --dry-run` for secret-free local validation. Add
  `--verify` only when a read-only Jira network request is intended.
- Login verifies `/rest/api/3/myself` before saving and atomically writes the
  configuration with mode `0600` on Unix.
