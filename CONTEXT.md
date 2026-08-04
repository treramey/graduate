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
- `gd diff <environment>` reports remote feature branches reachable from the
  environment but not main, discovers the remote default branch unless
  `--main` is set, and maps Jira keys from branch names.
- Promotion reports use Gitoxide for refs and commit traversal. Their fetch
  seam preserves Git credential-helper behavior and supports non-persistent
  PAT authentication.
- `gd diff --report age` emits a non-interactive, schema-versioned projection
  of unique unshipped commit age, including explicit UTC thresholds and the
  branches carrying the oldest work. The TUI opens the same projection with
  `a` after scanning completes.
- Promotion reports flag feature branches that had an environment branch
  merged into them by matching the environment's own merge commits in the
  branch's ahead-of-main history; merges of main are never flagged. Flagged
  rows render red with a `⚠` marker beside the branch name, selection shows a
  footer warning, and machine formats carry `mergedEnvironments`.
- The interactive promotion table sorts by branch name; `s` cycles branch,
  started, last, and ahead sorts. Machine formats always stay alphabetical.
- Promotion report automation defaults to JSON with camelCase fields and
  Jira-native issue field shapes. Use `--format` for table, YAML, or CSV and
  `--output` only with a safe relative path. PATs come from `GIT_PAT`, never a
  command-line flag.
