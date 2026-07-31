---
name: graduate
description: Configure Jira Cloud with Graduate. Use when an agent needs to inspect Graduate's CLI contract or establish its Jira connection.
---

# Graduate

Graduate is a Jira Cloud CLI with explicit commands, like Drag. Run `gd --help`
before constructing a command dynamically.

## Configure Jira

Interactive setup verifies Jira before saving:

```sh
gd auth setup jira
```

For automation, provide `ATLASSIAN_HOST`, `ATLASSIAN_EMAIL`, and
`ATLASSIAN_TOKEN`, then run `gd auth setup jira --from-env`. Use `--dry-run` before
saving. Dry-run networking requires explicit `--verify`. Never print or copy
the token from environment variables or `~/.graduate/config.json`.

Interactive setup requires terminal-capable stdin and stderr. Use Tab and
Shift-Tab to move, Enter to continue, Escape to go back or cancel, and Ctrl-C
to cancel from any stage.

## Inspect promotion gaps

Run `gd diff <environment>` to stream feature branches that are in an
environment but not the remote default branch. Use `--main <branch>` to
override main. Non-interactive runs default to API-native JSON. Use `--format
json|table|yaml|csv` and `--output <relative-path>` for other report surfaces.
Configured Jira credentials enrich keys found in branch names. Provide headless
Git authentication only through `GIT_PAT`; never print it.

## Current command contract

```text
Inspect Jira Cloud from the terminal

Usage: gd [OPTIONS] <COMMAND>

Commands:
  auth             Configure authentication for a ticket system
  diff             Show feature branches in an environment that have not reached main
  generate-skills  Generate portable AI agent skills from Graduate's command contract
  help             Print this message or the help of the given subcommand(s)

Options:
      --config <PATH>
          Override the configuration file (also available as GRADUATE_CONFIG)

  -h, --help
          Print help

  -V, --version
          Print version
```
