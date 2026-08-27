# @treramey/graduate

## 1.4.0

### Minor Changes

- 0ca5829: Add runtime schema discovery and an explicit dry-run option for agent-driven restack previews.
- 0ca5829: Add runtime JSON introspection for restack and harden encoded ref validation.
- d94f082: Publish `gd restack` with interactive review, agent JSON previews, resumable conflict resolution, and exact leased environment updates.
- cde06c9: Add an advanced authored-date-derived commit-age report to `gd diff`, an
  interactive age-report modal, consistent wide sizing across report modals, and
  JSON parameters for reports scoped to several feature branches. Base aggregate
  reporting on the authoritative environment-versus-main commit inventory and
  expose when an environment is behind main.

### Patch Changes

- 0ca5829: Clarify restack review consequences and the final publication checkpoint.
- fb23f43: Require deliberate restack publication and clarify rewrite impact at confirmation.
- 0ca5829: Block hidden restack actions and correct review scrolling and safety language.
- 0ca5829: Harden the interactive restack review, dependency guidance, conflict recovery, filter treatment, and compact terminal layouts.
- 6c28fea: Add the release-gated interactive restack checklist, safety review, confirmation, and conflict handoff.
- 3336acd: Add digest-authorized clean restack publication with fresh reconstruction and an exact remote lease.
- 0ca5829: Keep interactive restack evidence visible in compact terminals and add faster inventory controls.
- a85b3d7: Add isolated conflict-resolution reuse and resumable restack previews.
- a345539: Add single-use resumed restack publication and explicit session abort.
- e909343: Simplify the interactive Jira setup with compact inline prompts, arrow-key navigation, a plain review screen, and an isolated local test launcher.
- 0ca5829: Make interactive restack selection and review compact, structured, and impact-first while preserving smooth navigation through long feature lists.

## 1.3.0

### Minor Changes

- 1ba5b4c: Recover promoted work whose feature branch no longer exists. Hosting
  platforms often delete the source branch when a pull request completes,
  which made that work invisible to the branch-based promotion report even
  though its commits still sit in the environment. Graduate now also scans
  the non-merge commits unique to the environment, groups them by the Jira
  key in their subject, and adds one row per key that no surviving branch
  already covers. Merge commits are skipped, so environment sync merges
  never produce rows.

### Patch Changes

- 949b39e: Show only Jira-validated ticket keys in promotion report ticket columns; branches without a validated ticket leave the key blank and read `not found`.
- 520267d: Keep the text-presentation `⚠` merge-warning marker; the emoji-presentation form left a stray trailing character on the footer row in terminals that draw it two columns wide.
- 949b39e: Keep environment-merge warnings on stale feature branches after the
  environment branch is rebuilt or synced with pull-style self-merges.
  Graduate now recognizes merge commits whose recorded subject names an
  environment branch as the merge target or source, so `--no-ff` merges of an
  environment into a feature branch stay flagged even after the environment
  branch is reset.
- a126624: Always show a promotion row's Jira key instead of hiding keys that match the branch name, and label branches whose names carry no Jira key with a muted `no ticket` status.
- 949b39e: Warn in the promotion report footer when nearly every Jira lookup returns not found, pointing at the Jira site or project access instead of individual branches.

## 1.2.1

### Patch Changes

- f6a9bda: Clarify customer-facing text: rewrite the README introduction and promotion-report guidance, simplify CLI help and setup wizard copy, label dry-run output explicitly, and add a remediation hint to the Jira authentication error.
- 86c820a: Improve promotion-report table legibility: size the branch column to its content with wider column gaps, right-align ahead counts, repeat a Jira key only when it differs from the branch name, show explicit `not found` and `loading…` statuses with color-coded done and canceled tickets, mark environment-merged branches with a `⚠` beside the name, and add an `s` key that cycles the table sort by branch, start date, last activity, or ahead count.

## 1.2.0

### Minor Changes

- f9ed0dd: Flag promotion-report branches that have had an environment branch merged into them. Flagged rows render in red, selecting one explains in the footer that its ahead count and dates include environment commits, and JSON/CSV outputs gain a `mergedEnvironments` field.

### Patch Changes

- 785a232: Exclude merge commits from promotion ahead counts and the history sheet, so long-lived branches that sync often with main no longer report inflated ahead numbers.
- 785a232: Show a dash for missing Jira tickets in the promotion report status column, add a margin between the report table and the footer bar, and let the Git history sheet grow to the full viewport width.

## 1.1.1

### Patch Changes

- e5b4d81: Present promotion Git history in a content-adaptive sheet with explicit comparison context, commit metadata, and local controls.
- e5b4d81: Left-align promotion table headings and the selected-branch card.
- e5b4d81: Move selected-branch details above the promotion table and present them in a balanced two-column card that separates Jira status from branch metadata.
- e5b4d81: Remove the unnecessary authentication-window notice before interactive `gd diff` fetches.
- e5b4d81: Show Jira issue lookups that return HTTP 404 with a `not found` status.
- e5b4d81: Open the promotion-report interface before fetching remote branches.
- e5b4d81: Use a compact master-detail layout when interactive promotion reports have
  limited vertical space, hide the brand artwork, and prioritize available rows
  for selected-branch metadata while right-aligning the report summary and
  omitting its redundant heading. Pair the compact summary with a small Graduate
  wordmark and remove vertical inspector padding before clipping metadata.
  Use the branch name as the compact inspector title.

## 1.1.0

### Minor Changes

- 99b3f9f: Add a Git history modal to interactive promotion reports.

### Patch Changes

- baa1b98: Add compact Graduate artwork, balanced spacing, and an adjacent version to the terminal header.
- 2dcadb0: Harden promotion report streaming and exports, and recover interrupted Agent Skill publication.
- 0ffadf5: Keep promotion-list navigation moving smoothly through a scrolled viewport and
  clear stale Jira ticket warnings when selecting another branch. Raise ticket
  details and add space between them and the branch table.
- baa1b98: Add cursor movement and Unicode-aware editing to interactive Jira setup fields.

## 1.0.0

### Major Changes

- 67b0b44: Replace `gd login` with provider-aware `gd auth setup jira` and migrate Jira credentials into versioned connection configuration.
- a948a9d: Standardize Rust packages, distribution artifacts, configuration, and generated skills on Graduate while keeping `gd` as the installed CLI command.

### Minor Changes

- d49f82c: Add streaming environment promotion reports with Jira enrichment and API-native JSON, table, YAML, and CSV output.
- a948a9d: Add verified interactive and unattended Jira login with a split-pane-friendly layout, masked Atlassian API-token handling, bounded and cancellable verification, and secure atomic configuration storage.

### Patch Changes

- d49f82c: Stage and validate generated Agent Skills before replacing repository artifacts.

## 0.1.0

### Minor Changes

- Initial Rust CLI/TUI workspace, native distribution, and Agent Skill infrastructure.
