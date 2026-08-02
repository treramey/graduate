# @treramey/graduate

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
