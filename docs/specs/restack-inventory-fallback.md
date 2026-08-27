# Spec: `gd restack` inventory fallback

Status: reviewed, scoped to interactive-only for version 1. Nothing in this
document is implemented.

Scope decision: Graduate currently has one user. Version 1 ships the
interactive TUI path only. The machine `--params` mode, `describe`/`schema`
updates, skill regeneration, and a `gd diff` hint are deferred to a follow-up
and listed under "Deferred" at the end.

## Objective

Let `gd restack` rebuild an environment branch whose history is too broken to
read, without guessing.

Today restack learns which features are in an environment by reading the
environment's first-parent merge history. That works when the environment is
merely stale. It fails with `unsupported_history` when the environment has been
abused: a feature branch's own history pushed as the environment, direct
commits, deleted feature refs, or merges that could belong to several branches.
Those are exactly the environments restack exists to repair, and today the
only advice is "rebuild it by hand once, then come back".

The fallback replaces the history read with the reachability inventory `gd
diff` already uses. The user picks which reachable branches stay, sees exactly
which commits will be lost, and restack rebuilds from main as usual.

### User stories

- As a release engineer with a trashed `QA`, I run `gd restack QA`, see why the
  history cannot be read, choose to rebuild from inventory, tick the branches
  that belong in QA, see the commits that will be dropped, and publish.
- As the same engineer using `--dry-run`, I get today's `unsupported_history`
  error unchanged; the machine path does not gain inventory mode in version 1.
- As a maintainer of a healthy environment, nothing changes. History mode stays
  the default and the fallback never engages silently.

### Success looks like

Federated40 `QA` (3,690 remote refs, feature-internal merges on the spine)
goes from `error: unsupported_history` to a reviewed, publishable plan in one
interactive session, with the dropped commits listed before Ctrl+Y.

## Decisions

All of the following were reviewed and accepted.

1. The reachability predicate "remote tip reachable from the environment and
   not from main" (`promotion_candidates` in `environment_git.rs`) is the
   correct definition of "in the environment" when history cannot be trusted.
   It is already the definition `gd diff` reports.
2. Losing commits that no retained branch contains is acceptable when the user
   sees the list and confirms. That is the repair, not a defect.
3. Reused conflict resolutions (rerere training) are a history-mode benefit
   that inventory mode does not provide. Conflicts fall into the existing
   resume-token flow.
4. Merge order in inventory mode is a deterministic rule, not user-editable,
   in version 1.
5. `RESTACK_SCHEMA_VERSION` becomes 2; persisted v1 sessions are rejected as
   mismatched. No sessions exist to preserve.
6. Removing a carrier drops the branches it carries; carried branches are not
   promoted to top-level merges. The orphan list makes the consequence
   visible.
7. Merge order is not user-editable. Oldest tip first, then name.
8. No `gd diff` hint in version 1.

## Tech stack

Rust 2021 workspace. `graduate` (deterministic domain), `graduate-cli`
(Git via `gix` 0.86 and `git` subprocesses, Ratatui TUI, serde_json machine
contract). No new dependencies.

## Contract

### When the fallback is available

- The fallback is offered only after `build_snapshot` returns an
  `InventoryError`. A history that reads cleanly never uses inventory mode.
- History mode remains the default. Inventory mode is an explicit choice made
  by a key press on a dedicated TUI screen. The machine path (`--dry-run`,
  `--params`, `--apply` from a machine plan) does not offer it in version 1.
- The `unsupported_history` evidence that triggered the fallback is carried
  into the snapshot, the plan, the JSON, and the review screen, so the reason
  for the degraded mode is always visible.

### Inventory

- Candidates are remote refs under `refs/remotes/<remote>/`, excluding `HEAD`,
  main, the environment, known environments, and `backup/*` (existing
  `excluded_branch`), whose tip is reachable from the environment tip and not
  from the main tip.
- A candidate whose tip is reachable from another candidate's tip is
  **carried**. Carried branches are listed for information under their
  carrier and are not offered as top-level merges. Merging an already
  contained tip produces no merge commit and would break plan validation.
- Top-level features are the remaining candidates. Each becomes an
  `ExplicitFeature` with its current tip and an empty `historical_merges`.
- Graduated features (tip reachable from main) are reported as today.

### Merge order

- Top-level features are ordered by the author timestamp of their tip,
  oldest first, then by name. The rule is stated on the review screen.
- Rationale: oldest work was most likely merged first, which gives the best
  chance that later merges see the same base they saw originally. This is a
  heuristic for conflict friendliness only; it never decides membership.

### Orphaned commits

- An orphaned commit is a non-merge commit reachable from the environment, not
  reachable from main, and not reachable from any retained top-level feature
  tip. Dropped `### Match` markers are excluded as today.
- Orphaned commits are recomputed whenever the retained set changes, listed
  with short id, subject, author, and date, and included in the plan digest
  by id.
- Publishing requires the same Ctrl+Y gate as today. The confirmation text
  states the orphan count. The digest binds the reviewed orphan set.

### Removal validation

- `select_features` rules apply unchanged: duplicate, unknown, graduated, and
  carried-only names are rejected with evidence.
- `RetainedDependency` is preserved: a feature cannot be removed while a
  retained feature's tip still reaches its commits. Removing a carrier also
  removes the branches it carries unless another retained carrier reaches
  them.

### Reconstruction and publication

- `train_resolutions` runs as today; with no historical merges it does
  nothing. `MergeResolution::Reused` cannot occur in inventory mode.
- Isolated reconstruction, conflict persistence, resume, lease-guarded push,
  and endpoint binding are unchanged.
- `--apply` requires a digest produced in the same mode. A history-mode digest
  cannot apply an inventory-mode plan and vice versa because the mode is a
  digest field.

### What the fallback never does

- Pick a feature for an ambiguous merge by recency, commit count, name match,
  or merge message.
- Engage without an explicit user or params choice.
- Drop commits without listing them first.
- Rewrite feature branches.

## Domain model changes (`crates/graduate/src/restack.rs`)

```rust
/// How the feature inventory was discovered.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum InventoryMode {
    /// Explicit first-parent merges proved every commit's owner.
    History,
    /// History was unreadable; membership comes from remote tip reachability.
    Reachability,
}

/// Why history mode was unavailable, kept verbatim from the failed proof.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UnsupportedHistory {
    pub kind: String,            // "ambiguousFeatureRefs", "directCommit", ...
    pub commit: Option<String>,
    pub feature_parent: Option<String>,
    pub branches: Vec<String>,
    pub parents: Option<usize>,
}

/// A commit the rebuilt environment will not contain.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OrphanedCommit {
    pub commit: String,
    pub subject: String,
    pub author: String,
    pub date: String,
}

/// A branch whose tip another top-level feature already contains.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CarriedFeature {
    pub name: String,
    pub tip: String,
    pub carriers: Vec<String>,
}

pub struct RestackSnapshot {
    // existing fields unchanged ...
    pub inventory_mode: InventoryMode,
    pub unsupported_history: Option<UnsupportedHistory>,
    pub carried_features: Vec<CarriedFeature>,
}
```

New pure functions, all unit-tested with fixture graphs like the existing
`build_snapshot` tests:

```rust
/// Build a reachability snapshot after `build_snapshot` failed.
pub fn build_inventory_snapshot(
    graph: &RestackGraph,
    reason: UnsupportedHistory,
    tip_timestamps: &BTreeMap<String, i64>,
) -> RestackSnapshot;

/// Non-merge environment-only commits no retained tip reaches.
pub fn orphaned_commits(
    graph: &RestackGraph,
    snapshot: &RestackSnapshot,
    retained: &[BranchIdentity],
) -> Vec<String>;
```

`RestackGraph` gains nothing; `FeatureRef.ancestors` already holds each tip's
environment-only reachability, which is exactly what carried detection and
orphan computation need. `RestackPlan` gains `orphaned_commits:
Vec<OrphanedCommit>`. `plan_digest` adds fields `inventory_mode` and one
`orphaned_commit` entry per id, in sorted order. `RESTACK_SCHEMA_VERSION`
becomes `2`.

`RestackInteraction` gains a stage `UnsupportedHistory` before `Selection`,
with transitions `accept_inventory_fallback` and `cancel`. Existing stages
and transitions are unchanged.

## CLI changes (`crates/graduate-cli`)

### `restack.rs`

- `discover_interactive` catches `RestackInspectionError::Unsupported`. It
  keeps the `RestackGraph` and the evidence, and hands both to the TUI so the
  user can choose the fallback. `preview` (machine path) is unchanged and
  still fails with `unsupported_history`.
- `prepare_interactive` computes `orphaned_commits` for the selection and
  passes them to `build_plan`.
- `plan_json` (used by `write_success` details and by the machine path) adds:

```json
"inventory": {
  "mode": "reachability",
  "reason": { "kind": "ambiguousFeatureRefs", "mergeCommit": "…", "featureParent": "…", "branches": ["…"] }
},
"carriedBranches": [{ "name": "…", "tip": "…", "carriers": ["…"] }],
"orphanedCommits": [{ "commit": "…", "subject": "…", "author": "…", "date": "YYYY-MM-DD" }],
"effects": { "…existing…", "reusedResolutions": false }
```

  History-mode plans emit `"inventory": {"mode": "history", "reason": null}`,
  `"carriedBranches": []`, `"orphanedCommits": []`, and
  `"reusedResolutions": true`.

### `environment_git.rs`

- `restack_snapshot` returns the `RestackGraph` alongside the error on
  `Unsupported`, so the fallback does not re-walk anything. The graph already
  holds environment and main ancestor sets and every feature's environment-
  only ancestors; no new full-history walks are introduced.
- A helper reads tip author timestamps for ordering and subject/author/date
  for orphan rows, reusing the formatting in `non_merge_commits_excluding`.

### `restack_tui.rs`

- New screen for stage `UnsupportedHistory`: the evidence in plain words
  ("Merge 886faef4 on QA's history brings in 0bbff862, which 17 branches
  contain; restack cannot tell which one it meant."), the list of branches or
  commits from the evidence, and two actions: `r` rebuild from inventory,
  `Esc` cancel.
- Checklist banner in inventory mode: "Inventory mode: membership from
  reachability, merge order by tip age, no reused conflict resolutions."
  Carried branches render as indented informational rows under their carrier.
  The impact summary adds "N commits will be dropped".
- Review adds an "Orphaned commits" section (count, then rows, scrollable
  like retained features) and the merge-order rule. Confirmation adds one
  line: "Drops N commits that no retained branch contains."
- Every new screen fits the existing minimum sizes and has a `TestBackend`
  test.

### `describe.rs`, `schema restack`, generated skills

- The plan JSON gains fields, so `describe restack` and `schema restack`
  output and the generated skill must be regenerated to stay truthful. No
  new params are documented in version 1.

## Commands

```
Build:   cargo build --locked
Test:    cargo test --workspace --locked
Lint:    cargo clippy --workspace --locked --all-targets
Format:  cargo fmt --all --check
Skills:  cargo run --locked -- generate-skills --force
Perf:    cd <large repo> && time gd restack QA   (interactive; inspection phase)
```

## Project structure

```
crates/graduate/src/restack.rs            domain types, build_inventory_snapshot, orphaned_commits, digest, interaction stage
crates/graduate-cli/src/restack.rs        params mode, fallback wiring, plan JSON, orphan rows
crates/graduate-cli/src/restack_tui.rs    UnsupportedHistory screen, banners, review/confirmation sections
crates/graduate-cli/src/environment_git.rs graph handoff on Unsupported, tip metadata helper
crates/graduate-cli/src/describe.rs       contract description
crates/graduate-cli/tests/cli.rs          end-to-end fixtures
docs/gd-restack.md                        contract update
docs/specs/restack-inventory-fallback.md  this document
.changeset/restack-inventory-fallback.md  minor
```

## Code style

Follow the existing shape: pure functions over `RestackGraph` in the domain
crate, `Result` everywhere, no `unwrap`/`expect`/`panic`, evidence-bearing
error enums, camelCase serde with `deny_unknown_fields`.

```rust
pub fn orphaned_commits(
    graph: &RestackGraph,
    snapshot: &RestackSnapshot,
    retained: &[BranchIdentity],
) -> Vec<String> {
    let reached = graph
        .feature_refs
        .iter()
        .filter(|feature| retained.iter().any(|kept| kept.name == feature.name))
        .flat_map(|feature| feature.ancestors.iter())
        .collect::<BTreeSet<_>>();
    graph
        .environment_ancestors
        .difference(&graph.main_ancestors)
        .filter(|id| !reached.contains(id))
        .filter(|id| graph.commits.get(*id).is_some_and(|c| c.parents.len() == 1))
        .filter(|id| snapshot.dropped_markers.iter().all(|m| m.commit != **id))
        .cloned()
        .collect()
}
```

## Testing strategy

Domain (`crates/graduate`, unit tests beside the code):

- Inventory snapshot from a graph with an ambiguous merge: candidates,
  carried detection, oldest-first ordering, tie-break by name.
- Orphans: zero when every unique commit is reached; correct set after
  removing one branch; markers excluded; merges excluded.
- Digest changes when mode changes or the orphan set changes; unchanged
  otherwise.
- `select_features` rejects a carried-only name; `RetainedDependency` still
  fires.
- Interaction: `UnsupportedHistory` → `Selection` only via
  `accept_inventory_fallback`; cancel from that stage exits.

CLI (`crates/graduate-cli`, `TestBackend` for TUI, temp repos for Git):

- TUI: the new screen, checklist banner, carried rows, review orphan section,
  and confirmation line render at the minimum supported size.
- `environment_git`: a temp repo whose environment spine contains a
  feature-internal merge fails history mode and yields the expected graph.

End-to-end (`crates/graduate-cli/tests/cli.rs`, existing fixture style):

- `--dry-run` on a broken environment still fails with `unsupported_history`
  (guards that the machine path did not gain the fallback by accident).
- A history-mode `--dry-run` plan on a clean environment includes
  `inventory.mode = "history"`, empty `carriedBranches` and
  `orphanedCommits`, `reusedResolutions: true`, and `schemaVersion: 2`.

The interactive publish path is covered by `restack.rs` unit tests that drive
`interactive_workflow` pieces with a `TestBackend` terminal where the existing
tests already do so, plus the manual Federated40 run below.

Manual: Federated40 `QA` end to end. Before Ctrl+Y, verify the orphan list
against Git directly:

```fish
# every non-merge commit QA has that master lacks
git rev-list --no-merges origin/master..origin/QA > /tmp/unique
# minus everything reachable from the retained tips
git rev-list --no-merges origin/master..origin/<tip1> origin/master..origin/<tip2> ... > /tmp/kept
comm -23 (sort /tmp/unique | psub) (sort /tmp/kept | psub)
```

The result must equal the plan's `orphanedCommits` ids. Inspection stays under
a few seconds.

## Boundaries

- Always: run `cargo fmt --all --check`, `cargo clippy --locked
  --all-targets`, `cargo test --workspace --locked` before each commit; keep TUI tests on
  `TestBackend`; keep terminal restoration on every exit path; add a `minor`
  changeset; update `docs/gd-restack.md`, `README.md`, and `CHANGELOG.md`;
  regenerate skills after changing the CLI contract.
- Ask first: changing the reachability predicate shared with `gd diff`;
  letting users reorder merges; any alternative to bumping the schema version;
  adding a dependency.
- Never: infer a branch for an ambiguous merge; engage inventory mode without
  an explicit choice; publish without the orphan list in the reviewed plan;
  push from the source checkout; touch feature branches.

## Success criteria

1. `gd restack QA` on Federated40 shows the unsupported-history screen, and
   after `r` reaches a publishable plan; the confirmation shows the orphan
   count.
2. The plan's orphaned commits equal the `git rev-list` check above.
3. `gd restack QA --dry-run` on Federated40 fails exactly as today.
4. A clean-history environment produces a plan identical to today apart from
   the new `inventory`, `carriedBranches`, `orphanedCommits`, and
   `reusedResolutions` fields and `schemaVersion: 2`.
5. All new behavior is covered by the tests listed above; the full suite
   passes; clippy and fmt are clean; no `unwrap`/`expect`/`panic`.
6. Inspection on Federated40 completes in under 5 seconds.

## Deferred to a follow-up

- Machine path: `"mode": "inventory"` in `--params`, `"fallback": "inventory"`
  hint in the `unsupported_history` details, `invalid_mode` on clean history,
  and the copy-pasteable rerun command in the interactive error text.
- `describe restack` / `schema restack` documentation of the mode param.
- A `gd diff` footer hint when an environment's history is unreadable.
- User-editable merge order.
- Promoting a carried branch to a top-level merge when its carrier is removed.
