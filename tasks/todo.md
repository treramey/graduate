# Tasks: `gd restack` inventory fallback (interactive v1)

Spec: `docs/specs/restack-inventory-fallback.md` · Plan: `tasks/plan.md`

Standing bar for every task: `cargo fmt --all --check`, `cargo clippy --locked
--all-targets`, `cargo test --workspace --locked` clean; no `unwrap`/`expect`/`panic`/
`todo`/`dbg!`; TUI tests on `TestBackend` only; terminal restoration preserved.

---

## Task 1: Add inventory types and bump the schema version

**Description:** Introduce the domain types the fallback needs and give
history mode its defaults so nothing else changes behaviour yet.

**Acceptance criteria:**
- [x] `InventoryMode { History, Reachability }`, `UnsupportedHistory`,
      `OrphanedCommit`, `CarriedFeature` exist with camelCase serde and
      `deny_unknown_fields` (enum excepted), matching the spec.
- [x] `RestackSnapshot` has `inventory_mode`, `unsupported_history`,
      `carried_features`; `RestackPlan` has `orphaned_commits`.
      `build_snapshot` fills `History`, `None`, `[]`; `build_plan` accepts and
      stores `orphaned_commits`.
- [x] `RESTACK_SCHEMA_VERSION == 2`; a serde round-trip test covers
      `RestackSnapshot` and a plan with every new field populated.

**Verification:**
- [x] `cargo test --locked -p graduate`
- [x] `cargo test --workspace --locked` (CLI compiles against new fields; existing JSON
      assertions updated only for `schemaVersion`)

**Dependencies:** None
**Files:** `crates/graduate/src/restack.rs`, `crates/graduate-cli/src/restack.rs`
(call sites), `crates/graduate-cli/tests/cli.rs` (schemaVersion assertions)
**Scope:** M

---

## Task 2: Build a reachability snapshot from a failed history proof

**Description:** Add `build_inventory_snapshot(graph, reason, tip_timestamps)`
producing top-level features, carried features, graduated features, dropped
markers, and the evidence, without any new graph walks.

**Acceptance criteria:**
- [x] Candidates = `feature_refs` whose tip is in `environment_ancestors` and
      not in `main_ancestors`. A candidate whose tip is in another candidate's
      `ancestors` is carried (all carriers listed, sorted); the rest are
      top-level `ExplicitFeature`s with empty `historical_merges`.
- [x] Top-level order is ascending tip timestamp, then name; a missing
      timestamp sorts last by name (documented in a doc comment).
- [x] `From<InventoryError> for UnsupportedHistory` maps every variant with its
      evidence; `graduated_features` and `dropped_markers` match history mode.
- [x] Tests: ambiguous-merge fixture yields expected top-level/carried split;
      chain A⊂B⊂C yields one top-level; diamond yields two top-level and one
      carried with two carriers; ordering and tie-break; every `InventoryError`
      variant maps.

**Verification:**
- [x] `cargo test --locked -p graduate restack::tests::inventory`

**Dependencies:** Task 1
**Files:** `crates/graduate/src/restack.rs`
**Scope:** S

---

## Task 3: Compute orphaned commits and bind them into the plan digest

**Description:** Add `orphaned_commit_ids(graph, snapshot, retained)` and make
the digest depend on inventory mode and the orphan set.

**Acceptance criteria:**
- [x] Orphans = `environment_ancestors \ main_ancestors`, single-parent, not a
      dropped marker, not in any retained feature's `ancestors`. Sorted by id.
- [x] `plan_digest` adds `inventory_mode` and one `orphaned_commit` field per
      sorted id, after existing fields. History-mode digests change only by
      the new `schema` and `inventory_mode` values.
- [x] `select_features` rejects a carried-only name with
      `SelectionError::IndirectOnly` (carried features are surfaced as
      indirect); `RetainedDependency` behaviour unchanged.
- [x] Tests: zero orphans when every unique commit is reached; correct set
      after removing a branch; merges and markers excluded; digest changes
      with mode and with orphan set, stable otherwise.

**Verification:**
- [x] `cargo test --locked -p graduate restack::tests`

**Deviation:** orphans are computed from the snapshot alone via a new
`RestackSnapshot.unattributed_commits` field (no `RestackGraph` needed at plan
time), and `build_plan` verifies the orphan rows it receives against
`orphaned_commit_ids` (`PlanError::OrphanedCommits`).

**Dependencies:** Task 2
**Files:** `crates/graduate/src/restack.rs`
**Scope:** S

---

## Task 4: Add the `UnsupportedHistory` interaction stage

**Description:** Let `RestackInteraction` start on an evidence screen and move
to selection only by an explicit accept.

**Acceptance criteria:**
- [x] `RestackInteractionStage::UnsupportedHistory` exists.
      `RestackInteraction::from_unsupported(graph, reason, tip_timestamps)`
      starts there holding the evidence; `new(snapshot)` is unchanged.
- [x] Actions `AcceptInventoryFallback` (→ builds the inventory snapshot, all
      retained, stage `Selection`) and `Cancel` are the only ones handled in
      that stage; every other action is a no-op there.
- [x] Interaction exposes `orphaned_commit_count()` for the current retained
      set and `inventory_mode()`, `unsupported_history()`, `carried_features()`
      accessors for rendering.
- [x] Tests: accept moves to selection with every top-level feature retained;
      cancel exits; toggling in the checklist never re-enters the stage; count
      updates after a toggle.

**Verification:**
- [x] `cargo test --locked -p graduate restack::tests::interaction`

**Deviation:** the constructor is `RestackInteraction::from_inventory(snapshot)`;
the CLI builds the snapshot with `build_inventory_snapshot` so the interaction
never holds a `RestackGraph`.

**Dependencies:** Tasks 2, 3
**Files:** `crates/graduate/src/restack.rs`
**Scope:** S

---

### Checkpoint: Foundation
- [x] `cargo test --workspace --locked` green, clippy and fmt clean
- [x] Human review of the domain API before I/O wiring

---

## Task 5: Wire the fallback through the CLI and extend the plan JSON

**Description:** Carry the graph and evidence from inspection to the TUI,
compute orphan rows and tip timestamps in `graduate-cli`, and emit the new
plan fields.

**Acceptance criteria:**
- [x] `restack_snapshot` returns `Err(Unsupported { error, graph })` so the
      graph survives the failure; `discover_interactive` turns that into an
      `InteractiveDiscovery` variant carrying graph + evidence + tip
      timestamps (author seconds of each candidate tip, read with `gix`).
      `preview` (machine path) still fails with `unsupported_history`.
- [x] `prepare_interactive` computes orphan ids for the selection, loads
      subject/author/date for each (reuse the formatting in
      `non_merge_commits_excluding`), and passes `Vec<OrphanedCommit>` to
      `build_plan`. History mode passes `[]`.
- [x] `plan_json` emits `inventory {mode, reason}`, `carriedBranches`,
      `orphanedCommits`, `effects.reusedResolutions` (false in reachability
      mode, true in history mode) per the spec.
- [x] Tests: `environment_git` temp repo whose environment spine contains a
      feature-internal merge returns `Unsupported` with a graph that has the
      expected candidates; `plan_json` snapshot test for both modes.

**Verification:**
- [x] `cargo test --locked -p graduate-cli`

**Deviation:** orphan rows for every candidate commit are captured once at
discovery (`InteractiveDiscovery.commit_rows`) and persisted in
`SessionMetadata.orphaned_commits` so conflict resume rebuilds the same plan.

**Dependencies:** Tasks 1–4
**Files:** `crates/graduate-cli/src/environment_git.rs`,
`crates/graduate-cli/src/restack.rs`
**Scope:** M

---

## Task 6: Unsupported-history screen

**Description:** Render the evidence in plain words and offer `r` to rebuild
from inventory or `Esc` to cancel.

**Acceptance criteria:**
- [x] Screen shows: one-sentence plain explanation per `kind` (e.g. "Merge
      886faef4 on QA's history brings in 0bbff862, which 17 branches contain;
      restack cannot tell which one it meant."), the evidence list (branches
      or commits, scrollable), what inventory mode does and does not do
      (reachability membership, oldest-first order, no reused resolutions,
      commits not on a kept branch are dropped), and the footer controls.
- [x] `r` maps to `AcceptInventoryFallback`, `Esc`/`q` to `Cancel`; the
      workflow header names the stage.
- [x] Fits 60×24; wider terminals get the same content with less wrapping.
- [x] Tests: `TestBackend` renders at 60×24 and 100×30 with every
      `InventoryError` kind; key mapping test; too-small guidance still works.

**Verification:**
- [x] `cargo test --locked -p graduate-cli restack_tui`

**Dependencies:** Tasks 4, 5
**Files:** `crates/graduate-cli/src/restack_tui.rs`
**Scope:** S

---

## Task 7: Checklist banner, carried rows, and drop-count impact line

**Description:** Make inventory mode visible in the selection screen.

**Acceptance criteria:**
- [x] In reachability mode a one-line banner above the list reads
      "Inventory mode: reachability · oldest tip first · no reused resolutions"
      (≥100 columns) or "Inventory mode · no rerere" (narrower), sharing the
      row with the drop count. The full sentence lives on the
      unsupported-history screen.
- [x] Carried branches render as indented, non-selectable rows under their
      first carrier with "carried by X, Y"; the cursor skips them; filter
      matches them and shows their carrier.
- [x] Impact summary adds "N commits will be dropped" from
      `orphaned_commit_count()`, updating on every toggle; 0 renders as
      "no commits dropped".
- [x] Tests: banner absent in history mode; carried rows and cursor skipping;
      impact line updates after a toggle; compact width still shows the line.

**Verification:**
- [x] `cargo test --locked -p graduate-cli restack_tui`

**Dependencies:** Task 5
**Files:** `crates/graduate-cli/src/restack_tui.rs`
**Scope:** S

---

## Task 8: Review orphan section, merge-order note, and confirmation line

**Description:** Put the loss in front of the user before Ctrl+Y.

**Acceptance criteria:**
- [x] Review shows a "Dropped commits (N)" section listing short id, date,
      author, subject per orphan, scrollable with the existing review scroll;
      the merge-order rule appears once under the retained list in
      reachability mode; the reason evidence is in the technical details view.
- [x] Confirmation adds "Drops N commits that no retained branch contains."
      when N > 0; `confirmation_minimum_height` accounts for it.
- [x] `success_text` mentions the mode and the dropped count so the terminal
      record after publish is complete.
- [x] Tests: review with 0, 1, and 200 orphans at minimum and wide sizes;
      confirmation height; success text.

**Verification:**
- [x] `cargo test --locked -p graduate-cli restack_tui`

**Dependencies:** Task 5
**Files:** `crates/graduate-cli/src/restack_tui.rs`
**Scope:** S

---

### Checkpoint: Interactive flow
- [x] `cargo test --workspace --locked` green, clippy and fmt clean
- [x] Every new screen has a `TestBackend` test at 60×24

---

## Task 9: End-to-end guards and the Federated40 manual run

**Description:** Prove the machine path did not change and that the tool tells
the truth on the real repository.

**Acceptance criteria:**
- [x] `tests/cli.rs`: a fixture environment with a feature-internal merge on
      its spine makes `--dry-run` fail with `unsupported_history` and no
      inventory fields; a clean fixture's `--dry-run` plan has
      `schemaVersion: 2`, `inventory.mode = "history"`, empty
      `carriedBranches`/`orphanedCommits`, `reusedResolutions: true`.
- [ ] Manual: `gd restack QA` on Federated40 reaches review; the plan's
      `orphanedCommits` ids equal the `git rev-list`/`comm` check in the spec;
      inspection phase under 5 s. Record the command output and the check
      result in the PR description. Do not publish unless the check matches
      and you intend to rewrite QA.

**Verification:**
- [x] `cargo test --locked --test cli restack`
- [x] Manual check recorded

**Dependencies:** Tasks 5–8
**Files:** `crates/graduate-cli/tests/cli.rs`
**Scope:** S

---

## Task 10: Docs, contract regeneration, CHANGELOG, changeset

**Description:** Keep the written contract truthful.

**Acceptance criteria:**
- [ ] `docs/gd-restack.md` Contract gains a paragraph on the inventory
      fallback (trigger, explicit choice, membership rule, order rule, orphan
      disclosure, no rerere, schema 2) and the Delivery record gets a step.
- [ ] `README.md` restack section mentions the fallback in two sentences.
- [ ] `cargo run --locked -- generate-skills --force` regenerated; `describe
      restack` / `schema restack` reflect the new plan fields; their tests
      updated.
- [ ] `CHANGELOG.md` Unreleased → Added entry; `.changeset/restack-inventory-
      fallback.md` with `minor`.

**Verification:**
- [ ] `cargo test --workspace --locked`; `git diff --stat skills docs/skills.md` shows
      only generated changes

**Dependencies:** Task 5 (JSON shape final)
**Files:** `docs/gd-restack.md`, `README.md`, `CHANGELOG.md`, `.changeset/…`,
`skills/`, `docs/skills.md`, `crates/graduate-cli/src/describe.rs`
**Scope:** M

---

### Checkpoint: Complete
- [ ] Spec success criteria 1–6 checked off
- [ ] PR opened on its own branch off `main`, linking the spec
