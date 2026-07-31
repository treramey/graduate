---
name: grad
description: Configure Jira Cloud with Graduate. Use when an agent needs to inspect Graduate's CLI contract or establish its Jira connection.
---

# Graduate

Graduate is a Jira Cloud CLI with explicit commands, like Drag. Run `gd --help`
before constructing a command dynamically.

## Configure Jira

Interactive login verifies Jira before saving:

```sh
gd login
```

For automation, provide `ATLASSIAN_HOST`, `ATLASSIAN_EMAIL`, and
`ATLASSIAN_TOKEN`, then run `gd login --from-env`. Use `--dry-run` before
saving. Dry-run networking requires explicit `--verify`. Never print or copy
the token from environment variables or `~/.grad/config.json`.

Interactive login requires terminal-capable stdin and stderr. Use Tab and
Shift-Tab to move, Enter to continue, Escape to go back or cancel, and Ctrl-C
to cancel from any stage.

## Current command contract

```text
Inspect Jira Cloud from the terminal

Usage: gd [OPTIONS] <COMMAND>

Commands:
  login            Connect Jira, verify the account, then save
  generate-skills  Generate portable AI agent skills from Graduate's command contract
  help             Print this message or the help of the given subcommand(s)

Options:
      --config <PATH>
          Override the configuration file (also available as GRAD_CONFIG)

  -h, --help
          Print help

  -V, --version
          Print version
```
