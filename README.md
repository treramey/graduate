<h1 align="center">Graduate</h1>

**Inspect and rebuild Git workflow environments from the terminal.**

Graduate shows which feature branches are in an environment such as `qa` but
have not reached `main`. It can also rebuild that ephemeral environment from
its explicit feature merges. Jira Cloud enrichment links branch work to its
tickets, and a guided setup configures Jira credentials.

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

## Restack environment branches

`gd restack` rebuilds an ephemeral integration branch from the fetched remote
mainline and the environment's explicit feature merges. It never rewrites a
feature branch. Use it only for an environment branch that your team treats as
replaceable.

For an interactive review, run:

```bash
gd restack qa
```

Graduate fetches the remote, checks that it can attribute all environment-only
work, and selects every ungraduated feature by default. Uncheck a feature to
remove it from the rebuilt environment. Graduate reconstructs the selection in
an isolated work area and shows the exact refs, merge order, removals, final
tree, unsigned-commit policy, and environment lease. It pushes only after you
confirm that review.

Agents use the JSON-only preview and apply flow. Preview first and retain the
returned `planDigest`:

```bash
gd restack qa --params '{"removeBranches":["feature/PROJ-123"]}'
```

After reviewing the schema-v1 `restackPlan`, pass the same removal selection
and digest with the separate apply flag:

```bash
gd restack qa --params '{"removeBranches":["feature/PROJ-123"],"planDigest":"<PLAN_DIGEST>"}' --apply
```

Machine results go to stdout as JSON. Machine errors go to stderr as redacted
schema-v1 `restackError` JSON. `--params` never authorizes a push by itself.
Restack always fetches; it has no stale-ref, alternate-format, or output-file
mode. Use `--main <branch>` or `--remote <remote>` to override discovery.

If reconstruction conflicts, Graduate preserves the isolated work area for 24
hours and returns its path plus an opaque `resumeToken`. Resolve files there,
stage the complete resolution, and continue the preview:

```bash
git -C <WORK_AREA> add <RESOLVED_PATHS>
gd restack qa --resume <RESUME_TOKEN>
```

Review the returned plan, then publish that sealed session or discard it:

```bash
gd restack qa --resume <RESUME_TOKEN> --apply
gd restack qa --resume <RESUME_TOKEN> --abort
```

Do not print, log, or share the resume token. Successful apply and abort
consume it. A rejected push keeps a sealed session available for a validated
retry.

Restack validates Git state, not project behavior. It checks the resolved
index, conflict markers, whitespace errors, canonical merge parents and
messages, the final tree, configured Git identity, remote endpoints, and every
reviewed ref. It does not run repository tests or builds. Every generated merge
commit is unsigned. A remote that requires signed commits rejects the push
without weakening that policy.

Immediately before publication, Graduate revalidates the mainline,
environment, retained features, and removed features. It pushes only the
remote environment ref with an exact `--force-with-lease`. A moved or deleted
input, changed endpoint, identity change, or lease race fails closed and
requires a fresh preview. Preview can update the selected remote's
remote-tracking refs, but reconstruction and publication do not change the
source checkout, local branches, hooks, or personal rerere cache.

See the complete [`gd restack` safety contract](docs/gd-restack.md).

## Promotion reports

Show remote feature branches that are present in an environment but have not
reached the repository's main branch:

```bash
gd diff qa
```

Branches deleted after their pull request completed still appear: Graduate
groups the non-merge commits unique to the environment by the Jira key in
their subject and adds one row per key, named after the key, when no
surviving branch covers it.

Move between branches with the arrow keys or `j`/`k`. A detail card above the
branch table describes the selected branch. On terminals that are short but
wide, Graduate moves the detail card beside the table so more rows stay
visible.

Press `s` to cycle the table sort between branch name, start date, last
activity, and ahead count; the active column heading shows the sort direction.
Ahead counts are right-aligned. The Jira column only shows ticket keys that
Jira validated; every other branch leaves the column blank and shows a muted
`not found`.

Graduate fetches `origin`, discovers the remote default branch (falling back
to `main`, `master`, `trunk`, or `develop`), and streams alphabetically sorted
rows into a terminal list. The list opens before the fetch and updates when
scanning begins. Pass `--main <branch>` to set the main branch. When Jira is
configured, a branch name that contains a Jira key such as `PROJ-123` gains
the ticket summary, status, assignee, and fix versions. Completed statuses
render green and canceled statuses are muted. Tickets that Jira cannot find
show a blank key and a muted `not found`. When nearly every lookup misses,
the footer warns you to check the Jira site and project access, because Jira
answers 404 for tickets an account cannot browse. Select a row and press `o`
to open its ticket.

Press `h` to open a history sheet listing the selected branch's commits ahead
of main, newest first. Each row shows the commit's short SHA, subject, author,
and date. Press `h` or Escape to close the sheet.

Press `a` after the scan completes to open the age report. It buckets unique
unshipped commits into exactly the authored years present in Git history—there
are no filler years or synthetic cutoff bucket. It calls out work written in
the last 90 days and work older than one year, and identifies the branches
carrying commits from the oldest observed year. Use the arrow keys or `j`/`k`
to scroll long reports. Press `a` or Escape to close the report.
The history and age-report modals share the same wide content viewport and
narrow only when the terminal requires it.

Graduate renders a feature branch in red with a `⚠` marker beside its name
when an environment branch was merged into it, and shows a footer warning when
you select it. That branch's start
date and ahead count include environment history rather than its own work.
Before you promote it, rebuild the branch so it contains only its own commits,
for example by rebasing it onto main. Graduate follows each environment
branch's own merge commits, so a branch that only merged the main branch is
never flagged. Environment merges stay flagged even after the environment
branch is rebuilt or synced with pull-style self-merges, because Graduate
also recognizes merge commits whose recorded subject names an environment
branch as the merge target or source, including `--no-ff` merges of an
environment into a feature branch. Machine formats expose the same signal as
a `mergedEnvironments` field.

Non-interactive runs emit JSON by default, with camelCase report fields and
Jira issue data in the same `fields.status.name`, `fields.assignee.displayName`,
and `fields.fixVersions` shapes that Jira returns. Select another format with
`--format json|table|yaml|csv`, and write it to a file with `-o, --output <path>`:

Branch-report JSON includes an authoritative `commitInventory`; its
`aheadOfMain` and `behindMain` objects each contain an explicit count and the
complete non-merge commit list. After an interactive scan completes, the title
also calls out when the environment is behind main.

```bash
mkdir -p reports
gd diff qa --format csv --output reports/qa.csv
```

Agents can scope a report to several exact remote feature branches with JSON
parameters:

```bash
gd diff qa --params '{"branches":["feature/PROJ-123","feature/PROJ-456"]}'
gd diff qa --report age --params '{"branches":["feature/PROJ-123","feature/PROJ-456"]}'
```

`--params` selects unattended output, sorts and deduplicates the requested
names, and applies the same scope to branch or age reports. Graduate fails with
an explicit error if a requested branch does not exist, has not reached the
environment, or has already reached main. Scoped reports include only the
named remote branches and do not add recovered rows for deleted branch refs.

Select the advanced age report explicitly for automation or a printable table:

```bash
gd diff qa --report age
gd diff qa --report age --format table
```

Supplying `--report branches|age` or `--params` selects unattended output even
in a terminal.
The age report's JSON schema v2 includes the authoritative commit inventory, a
UTC `asOf` date, explicit inclusive/exclusive threshold dates, percentages,
stable assessment kinds, and the branches carrying the oldest work. Aggregate
totals count each non-merge commit reachable from the environment but absent
from main once, even when no surviving branch row attributes it. `buckets`
contains only years observed in commit authored dates and `oldestYear`
identifies the oldest observed year. Behind-main commits are included only as
a count and inventory, without age buckets or assessments. CSV rows carry a
`rowType` so agents can distinguish inventories, commits, age buckets, decision
thresholds, and oldest-branch details without parsing display text.

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

The wizard uses compact inline prompts with explicit Continue, Connect, and
Save actions; move between them with the Up and Down arrow keys, Tab, or
Shift-Tab. Text fields support
cursor movement and Unicode-aware editing. Set `GRADUATE_REDUCED_MOTION=1` to
disable the remaining setup motion. Setup requires a terminal at least 60
columns by 24 rows.

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

Exercise the Jira setup UI with an isolated local verifier and temporary
configuration:

```bash
pnpm test:setup
```

The launcher prints safe fixture values before opening the TUI. It does not
open a browser or contact Jira, and it deletes the saved configuration after
the test exits. Set `GRADUATE_SETUP_FIXTURE_KEEP=1` to inspect the temporary
configuration afterward.

Create a disposable Git repository and exercise the promotion report without
touching a real remote or Jira configuration:

```bash
pnpm test:diff                         # interactive TUI
pnpm --silent test:diff -- --format json  # machine-readable JSON
pnpm --silent test:diff -- --format table
pnpm --silent test:diff -- --format yaml
pnpm --silent test:diff -- --format csv
pnpm --silent test:diff -- --report age
pnpm --silent test:diff -- --report age --format table
```

The fixture contains two branches in `qa` but not `main`, plus one branch that
has already graduated and must be excluded. Set
`GRADUATE_DIFF_FIXTURE_KEEP=1` to retain the temporary repository for inspection
or to test `--output` files. The interactive launcher creates a pseudo-terminal
with the controlling terminal's dimensions; if no controlling terminal exists,
it exits with instructions instead of silently falling back to JSON.

## License

[MIT](LICENSE)
