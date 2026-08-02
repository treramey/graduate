<h1 align="center">Graduate</h1>

**Inspect Jira Cloud from the terminal.**

Graduate is a Jira Cloud CLI and terminal interface. It shows which feature
branches are in an environment such as `qa` but have not reached `main`,
enriched with each branch's Jira ticket, and includes a guided setup for Jira
Cloud credentials.

## Install

Install the npm bootstrap package:

```bash
npm install --global @treramey/graduate
```

Other supported distribution paths:

```bash
nix run github:treramey/graduate
brew install treramey/tap/graduate
```

Or build directly from this repository:

```bash
cargo install --path crates/graduate-cli
```

## Run

```bash
gd auth setup jira
```

Graduate requires an explicit command. Run `gd --help` to list the available
commands.

## Promotion reports

Show remote feature branches that are present in an environment but have not
reached the repository's main branch:

```bash
gd diff qa
```

Move between branches with the arrow keys or `j`/`k`. A detail card above the
branch table describes the selected branch. On terminals that are short but
wide, Graduate moves the detail card beside the table so more rows stay
visible.

Press `s` to cycle the table sort between branch name, start date, last
activity, and ahead count; the active column heading shows the sort direction.
Ahead counts are right-aligned, and a row repeats its Jira key only when the
key differs from the branch name.

Graduate fetches `origin`, discovers the remote default branch (falling back
to `main`, `master`, `trunk`, or `develop`), and streams alphabetically sorted
rows into a terminal list. The list opens before the fetch and updates when
scanning begins. Pass `--main <branch>` to set the main branch. When Jira is
configured, a branch name that contains a Jira key such as `PROJ-123` gains
the ticket summary, status, assignee, and fix versions. Completed statuses
render green and canceled statuses are muted. Tickets that Jira cannot find
show a muted `not found`. Select a row and press `o` to open its ticket.

Press `h` to open a history sheet listing the selected branch's commits ahead
of main, newest first. Each row shows the commit's short SHA, subject, author,
and date. Press `h` or Escape to close the sheet.

Graduate renders a feature branch in red with a `⚠` marker beside its name
when an environment branch was merged into it, and shows a footer warning when
you select it. That branch's start
date and ahead count include environment history rather than its own work.
Before you promote it, rebuild the branch so it contains only its own commits,
for example by rebasing it onto main. Graduate follows each environment
branch's own merge commits, so a branch that only merged the main branch is
never flagged. Machine formats expose the same signal as a
`mergedEnvironments` field.

Non-interactive runs emit JSON by default, with camelCase report fields and
Jira issue data in the same `fields.status.name`, `fields.assignee.displayName`,
and `fields.fixVersions` shapes that Jira returns. Select another format with
`--format json|table|yaml|csv`, and write it to a file with `-o, --output <path>`:

```bash
mkdir -p reports
gd diff qa --format csv --output reports/qa.csv
```

Output paths are relative to the current directory and cannot traverse parent
directories or symbolic links outside it. Set `GIT_PAT` for headless fetch authentication;
tokens are not accepted as command-line flags. `--no-fetch` inspects existing
remote-tracking refs.

## Jira configuration

Run the interactive Jira authentication wizard:

```bash
gd auth setup jira
```

Graduate authenticates with a Jira site, an Atlassian account email, and an
Atlassian API token. The wizard masks the token, verifies it with the
read-only Jira `/rest/api/3/myself` endpoint, and saves it only after you
confirm the review screen. You can keep an existing token without displaying
it.

The wizard uses boxed, focusable inputs with explicit Continue, Connect, and
Save actions; move between them with Tab and Shift-Tab. Text fields support
cursor movement and Unicode-aware editing. Set `GRADUATE_REDUCED_MOTION=1` to
replace moving setup effects with short fades. Setup requires a terminal at
least 76 columns by 48 rows.

For unattended setup:

```bash
export ATLASSIAN_HOST=https://yourcompany.atlassian.net
export ATLASSIAN_EMAIL=you@example.com
export ATLASSIAN_TOKEN=...

gd auth setup jira --from-env
```

Preview local validation without networking or saving:

```bash
gd auth setup jira --from-env --dry-run
```

Add `--verify` to the dry-run for an explicit read-only Jira check. The default
configuration path is `~/.graduate/config.json`; override it with `--config` or
`GRADUATE_CONFIG`. Configuration files from earlier Graduate versions still
work; Graduate upgrades them on the next successful setup.

## Workspace

```text
crates/
├── graduate/      # I/O-independent Jira services and state transitions
└── graduate-cli/  # Clap, Crossterm, Ratatui, process errors, and terminal lifecycle
```

See [`docs/architecture.md`](docs/architecture.md) for the design rules.

## AI agent skills

Graduate generates portable Agent Skills from its command definitions. It
validates the complete set before replacing any existing skill files:

```bash
cargo run -- generate-skills --force
npx skills add https://github.com/treramey/graduate
```

See the [skills index](docs/skills.md).

## Development

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --locked
```

Create a disposable Git repository and exercise the promotion report without
touching a real remote or Jira configuration:

```bash
pnpm test:diff                         # interactive TUI
pnpm --silent test:diff -- --format json  # machine-readable JSON
pnpm --silent test:diff -- --format table
pnpm --silent test:diff -- --format yaml
pnpm --silent test:diff -- --format csv
```

The fixture contains two branches in `qa` but not `main`, plus one branch that
has already graduated and must be excluded. Set
`GRADUATE_DIFF_FIXTURE_KEEP=1` to retain the temporary repository for inspection
or to test `--output` files. The interactive launcher creates a pseudo-terminal
with the controlling terminal's dimensions; if no controlling terminal exists,
it exits with instructions instead of silently falling back to JSON.

## License

[MIT](LICENSE)
