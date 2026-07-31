# @treramey/graduate

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
