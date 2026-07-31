# Agent usage context

- Like Drag, `gd` requires an explicit command. Run `gd --help` to inspect the
  command contract.
- A ticket system is an external ticket manager such as Jira or Linear. Jira is
  the only supported ticket system today.
- A connection is one ticket system's provider-specific credentials and
  verified identity.
- `gd auth setup jira` owns the current terminal interface. It requires
  terminal-capable stdin and stderr and renders to stderr.
- Keep Jira authentication state deterministic and independent of Ratatui and
  Crossterm.
- Keep validated Jira sites, credentials, identities, and authentication
  acceptance rules in `graduate`; adapters may only consume those contracts.
- Never initialize a real terminal in tests. Render with `TestBackend` and inject state transitions directly.
- Regenerate repository-controlled skills with `gd generate-skills --force` whenever the public CLI changes.
- `gd auth setup jira` uses Jira Cloud Basic authentication with the Jira site,
  Atlassian account email, and API token; it never prints the token.
- Use `gd auth setup jira --from-env --dry-run` for secret-free local
  validation. Add `--verify` only for a read-only Jira network request.
- Jira setup verifies `/rest/api/3/myself` before saving and atomically writes
  the configuration with mode `0600` on Unix.
