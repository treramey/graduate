<h1 align="center">Graduate</h1>

**Inspect Jira Cloud from the terminal.**

Graduate is a Jira Cloud CLI and terminal interface. It keeps domain behavior
independent from terminal and network I/O so commands remain predictable and
testable. The `graduate` core owns validated Jira sites, credentials,
identities, and Jira authentication transitions; `graduate-cli` owns external I/O.

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

Like Drag, Graduate requires an explicit command. Run `gd --help` to list the
available commands.

## Promotion reports

Show remote feature branches that are present in an environment but have not
reached the repository's main branch:

```bash
gd diff qa
```

Move between branches with the arrow keys or `j`/`k`. The viewport scrolls
smoothly with the selection, and selection-specific Jira warnings clear when
you move to another branch. The default layout keeps a full-width detail card
above the branch table. Short terminals with enough horizontal room adapt to a
master-detail layout, placing the branch table beside a persistent inspector so
more rows remain visible. Compact-height reports also omit the Graduate artwork
to prioritize report content.

Graduate fetches `origin`, discovers the remote default branch (then falls back
to `main`, `master`, `trunk`, or `develop`), and streams alphabetically sorted
rows into a terminal list. The interface opens before the interactive fetch and
updates when scanning begins. Pass `--main <branch>` for a custom main branch.
Branch names containing a Jira key such as `PROJ-123` are enriched with the
ticket summary, status, assignee, and fix versions when Jira is configured.
Missing Jira tickets show a `not found` status. Select a row and press `o` to
open its ticket.

Press `h` to open a content-adaptive history sheet listing the selected
branch's commits ahead of the resolved main branch in newest-first order. The
sheet shows each commit's short SHA, subject, author, and date alongside its
list position, and uses `h` or Escape to close.

For machine use, Graduate follows API-native output conventions. Non-interactive
runs emit JSON by default, with camelCase report fields and Jira issue data in
the same `fields.status.name`, `fields.assignee.displayName`, and
`fields.fixVersions` shapes returned by Jira. Select another representation
with `--format json|table|yaml|csv`, and write it with `-o, --output <path>`:

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

Graduate uses the same Jira Cloud authentication method as Drag: Jira site,
Atlassian account email, and Atlassian API token. The token is masked, verified
with the read-only Jira `/rest/api/3/myself` endpoint, and saved only after the
review screen is confirmed. Existing tokens can be retained without being
loaded into an editable field.

The interactive wizard follows Drag's guided layout: boxed, focusable inputs;
explicit Continue, Connect, and Save actions; Tab and Shift-Tab navigation; and
a final connection manifest. Text fields support cursor movement and
Unicode-aware editing. Graduate omits Drag's Tempo authentication step. Set
`GRADUATE_REDUCED_MOTION=1` to replace moving setup effects with short fades.
The setup interface supports terminal panes at least 76 columns by 48 rows.

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
`GRADUATE_CONFIG`. Graduate stores connections in a versioned, provider-tagged
configuration so other ticket systems can be added without mixing credential
formats. Graduate reads the original flat Jira configuration and writes the new
schema after the next successful setup.

## Workspace

```text
crates/
├── graduate/      # I/O-independent Jira services and state transitions
└── graduate-cli/  # Clap, Crossterm, Ratatui, process errors, and terminal lifecycle
```

See [`docs/architecture.md`](docs/architecture.md) for the design rules.

## AI agent skills

Graduate includes repository-controlled Agent Skills generated from its CLI
contract. Generation stages and validates the complete update before replacing
existing skill artifacts:

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
pnpm --silent test:diff -- --format json  # clean API-native JSON
pnpm --silent test:diff -- --format table
pnpm --silent test:diff -- --format yaml
pnpm --silent test:diff -- --format csv
```

The fixture contains two branches in `qa` but not `main`, plus one branch that
has already graduated and must be excluded. Set
`GRADUATE_DIFF_FIXTURE_KEEP=1` to retain the temporary repository for inspection
or to test `--output` files. The interactive launcher creates a pseudo-terminal
with the controlling terminal's dimensions; if it cannot find one, it exits with
instructions instead of silently falling back to JSON.

## License

[MIT](LICENSE)
