# Implementation Plan: `gd restack` inventory fallback (interactive v1)

Spec: `docs/specs/restack-inventory-fallback.md`.

## Overview

When `build_snapshot` cannot read an environment's merge history, offer an
explicit interactive fallback that builds the feature inventory from remote
tip reachability, shows the commits that will be dropped, and reuses the
existing isolated reconstruction, review, and lease-guarded publish. History
mode stays the default; the machine path is unchanged in v1.

## Architecture decisions

- The fallback is a second pure snapshot builder in `graduate::restack` over
  the same `RestackGraph`. No new Git walks: `FeatureRef.ancestors` already
  holds each tip's environment-only reachability, which is exactly what carried
  detection and orphan computation need.
- `RestackSnapshot` grows three fields (`inventory_mode`,
  `unsupported_history`, `carried_features`); `RestackPlan` grows
  `orphaned_commits`. History mode fills them with `History`, `None`, `[]`,
  `[]`, so every existing consumer keeps working and the plan JSON stays
  truthful in both modes.
- Orphans are computed at plan time from `(graph, retained)` and bound into the
  digest. The interaction also computes the count on every toggle for the
  checklist impact line; both use the same pure function.
- A new `RestackInteractionStage::UnsupportedHistory` sits before `Selection`.
  `RestackInteraction::new` stays for history mode; a new constructor takes the
  graph plus evidence and starts in the new stage.
- `RESTACK_SCHEMA_VERSION` bumps to 2. Persisted v1 sessions are rejected by
  the existing mismatch path.
- `plan_json` gains `inventory`, `carriedBranches`, `orphanedCommits`, and
  `effects.reusedResolutions`. `describe`/`schema` and the generated skill are
  regenerated because they describe that JSON; no new params.

## Dependency graph

```
T1 domain types + schema bump
 ├── T2 build_inventory_snapshot (+ carried, ordering)
 │    ├── T3 orphaned_commits + digest fields + build_plan
 │    │    ├── T5 CLI wiring: graph handoff, tip metadata, orphans in prepare, plan JSON
 │    │    │    ├── T7 TUI: checklist banner + carried rows + impact line
 │    │    │    ├── T8 TUI: review orphan section + confirmation line
 │    │    │    └── T9 e2e + Federated40 manual run
 │    │    └── T6 TUI: UnsupportedHistory screen (needs T4)
 │    └── T4 interaction stage + transitions
 └── T10 docs, describe/schema/skills regen, changeset
```

## Task list

### Phase 1: Domain foundation (pure, no I/O)

- [x] Task 1: Add inventory types and bump the schema version
- [x] Task 2: Build a reachability snapshot from a failed history proof
- [ ] Task 3: Compute orphaned commits and bind them into the plan digest
- [ ] Task 4: Add the `UnsupportedHistory` interaction stage

### Checkpoint: Foundation
- [ ] `cargo test --locked -p graduate` passes with new tests
- [ ] Existing history-mode snapshot/plan/digest tests unchanged in intent
- [ ] Review with human before wiring I/O

### Phase 2: CLI wiring

- [ ] Task 5: Hand the graph and evidence to the interactive flow and emit the new plan JSON fields

### Phase 3: TUI

- [ ] Task 6: Unsupported-history screen with `r` to rebuild and `Esc` to cancel
- [ ] Task 7: Checklist banner, carried rows, and "N commits will be dropped" impact line
- [ ] Task 8: Review orphan section, merge-order note, and confirmation line

### Checkpoint: Interactive flow
- [ ] `cargo test --workspace --locked` passes; clippy and fmt clean
- [ ] TUI tests render every new screen at the minimum supported size

### Phase 4: Verification and delivery

- [ ] Task 9: End-to-end guard tests and the Federated40 manual run
- [ ] Task 10: Docs, `describe`/`schema`/skills regeneration, CHANGELOG, changeset

### Checkpoint: Complete
- [ ] All spec success criteria met
- [ ] PR opened with spec linked

## Risks and mitigations

| Risk | Impact | Mitigation |
|------|--------|------------|
| Carried detection wrong → merging an already-contained tip yields no commit and `PlanError::MergeCount` fires | High | Unit-test carried detection on chains and diamonds; the plan validation is a second net |
| Orphan list disagrees with Git | High | Manual `git rev-list` cross-check on Federated40 before Ctrl+Y (spec, Testing) |
| Oldest-first order causes conflicts that history order would not | Medium | Conflicts land in the existing resume flow; order is stated on screen |
| Schema bump breaks a resume session mid-work | Low | No sessions exist; mismatch path already reports clearly |
| New snapshot fields break `deny_unknown_fields` round-trips in session storage | Medium | Serde round-trip test for `RestackSnapshot`/`RestackPlan` |
| TUI screens overflow the 60×24 minimum | Medium | `TestBackend` tests at minimum size, as the existing tests do |

## Parallelization

- T2, T3, T4 can proceed in parallel once T1 lands (T3 needs T2's output shape,
  but only the `RestackSnapshot` fields, which T1 defines).
- T6 depends on T4 and T5; T7 and T8 depend on T5. T6–T8 can be parallel.
- T10 can start documentation any time; regeneration waits for T5.

## Open questions

None. Decisions 1–8 in the spec are accepted.
