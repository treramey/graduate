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

Interactive setup must run in a terminal. Use Tab and
Shift-Tab to move, Enter to continue, Escape to go back or cancel, and Ctrl-C
to cancel from any stage.

## Inspect promotion gaps

Run `gd diff <environment>` to stream feature branches that are in an
environment but not the remote default branch. Use `--main <branch>` to
override main. Non-interactive runs default to JSON. Use `--format
json|table|yaml|csv` and `--output <relative-path>` for other report surfaces.
Configured Jira credentials enrich keys found in branch names. Provide headless
Git authentication only through `GIT_PAT`; never print it.

Branch-report JSON includes an authoritative `commitInventory`. Its
`aheadOfMain` and `behindMain` objects each contain an explicit count and the
complete ordered non-merge commit inventory. Use `behindMain.count` to decide
whether the environment is out of sync with main; behind commits are diagnostic
and do not receive age assessments. The completed interactive report calls out
the same behind count.

Scope a report to exact remote branches with `--params
'{"branches":["feature/PROJ-123","feature/PROJ-456"]}'`. This always selects
unattended output and works with either the default branch report or `--report
age`. Requested branches must exist, be in the environment, and remain absent
from main; invalid selections fail explicitly.

Use `gd diff <environment> --report age` for the advanced commit-age report.
Supplying `--report branches|age` always selects unattended output. Prefer JSON
for agents: age schema v2 has `schemaVersion`, UTC `asOf`, stable assessment
kinds, explicit threshold dates, `oldestYear`, `oldestBranches`, and the same
authoritative commit inventory. Its buckets contain only years found in commit
authored dates. Aggregate totals count every non-merge commit reachable from
the environment and absent from main once, including commits without a branch
or Jira attribution row. In the interactive report, press `a` after scanning
completes to open the same scrollable age projection.

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
