# `gd restack` Safety Contract

## Purpose

`gd restack <ENVIRONMENT>` rebuilds an ephemeral Gitworkflow integration branch
from the remote mainline plus the remote feature branches that were in the
environment before rebuilding and have not graduated to main. A user or agent
can deliberately remove selected features from the reconstructed branch.

## Contract

- `restack` rebuilds an environment branch; it does not rewrite feature
  branches.
- The command captures the environment's feature-branch membership before
  resetting/reconstructing it.
- The rebuild base is the fetched remote mainline. Graduate discovers it from
  `<remote>/HEAD`, with the same `--main` and `--remote` overrides and fallback
  behavior as `gd diff`.
- Every preview and apply starts with a mandatory remote fetch. `restack` has
  no stale-ref or offline mode; fetch failure stops before planning or creating
  an isolated work area.
- Only feature branches that have not graduated to main are re-merged.
- Retained features are re-merged once at their current remote tips, in the
  oldest-to-newest order of their first explicit merges on the original
  environment's first-parent history.
- Every retained feature is merged with `--no-ff` as a two-parent merge commit
  using `Merge branch '<feature>' into <environment>`. Restack never fast-
  forwards, squashes, or rebases feature commits.
- New merge commits use the normal `user.name` and `user.email` resolved from
  the source repository's Git configuration. Graduate clears ambient author,
  committer, and date overrides, then injects that configured identity as both
  author and committer while letting Git generate fresh timestamps. The shared
  identity is included in the plan; missing identity is a preflight error.
  Graduate does not invent a bot or preserve stale original authorship.
- Version 1 creates explicitly unsigned merge commits and declares this in the
  reviewed effects. It never inherits ambient signing/pinentry behavior. A
  remote that requires signed commits rejects the push safely; signing support
  needs a separate deliberate design.
- The feature follows Graduate's existing seam: deterministic planning and UI
  transitions live in a deep `graduate::restack` module; Git, process,
  filesystem, cache, terminal, and remote effects stay in `graduate-cli`.
  Restack's SQLite/service architecture is not ported.
- A feature merely reachable through another retained feature is not added as
  a separate top-level merge unless the original environment explicitly merged
  it.
- Planning fails before selection unless every ungraduated commit unique to the
  environment is attributable to a surviving, uniquely identified explicit
  feature merge. Direct environment commits, deleted feature refs, fast-forward
  history, octopus merges, or ambiguous merge-parent/ref mappings are reported
  as unsupported rather than guessed or silently dropped.
- Version 1 is a single-phase rebuild of the selected environment. It does not
  infer environment hierarchy, recreate `### Match '<environment>'` markers,
  or implement the Restack prototype's lower-environment phase model.
- Exact `### Match '<environment>'` commits whose tree equals their first
  parent's tree are recognized as old phase metadata, listed as dropped in the
  plan, and not recreated. Other direct environment commits, including other
  empty commits, remain reconstructability errors.
- The interactive flow shows the discovered feature branches and allows the
  user to exclude branches from the rebuilt environment. Every discovered
  explicit feature is selected by default; unchecking it means removal.
- Interactive selection uses a focused Ratatui checklist on stderr, with merge
  order, branch, short tip SHA, locally parsed Jira key when present, and rerere
  training availability. Compact terminals reflow those fields instead of
  hiding them; wide terminals collapse the same evidence into one row per
  feature. The selected row names retained features that block removal. `?`
  progressively reveals Page Up/Page Down, Home/End, keep-all/remove-all, and
  alternate cancel shortcuts for large inventories. `/` opens a lazygit-style
  filter prompt in the footer rather than reserving list space for an inactive
  input. It filters visible rows by branch name, reports the match count, and
  leaves hidden selections unchanged. An inline legend defines retained, removed, dependency, and
  reusable-history states. A persistent stage marker keeps selection, review,
  and publication visible; terminals smaller than the safe rendering floor
  receive content-aware resize guidance. It performs no Jira requests. A
  separate review screen shows captured base/environment OIDs, retained order,
  omissions, and the target ref, exact lease, canonical merge intent, and final
  tree before confirmation. It explains that omitted features leave their
  remote feature branches unchanged, translates the exact lease into the
  condition that stops publication, and labels the next action as opening the
  publish confirmation. That confirmation carries forward retained, omitted,
  and merge-outcome counts before accepting `Ctrl+Y`. It names up to three omitted
  branches and directs larger omission sets back to Review instead of creating
  an unbounded confirmation screen. `Esc` returns to Review while `q` abandons
  the reviewed plan without changing refs. The main summary stays focused on rewrite, impact, and the
  publication guard, while plan details progressively disclose captured base
  and result bindings, full feature identities, endpoint bindings, authorship,
  and signing policy. Plan details stay before the retained merge list so they
  remain reachable without traversing large inventories. Review supports line,
  page, and Home/End scrolling and is tested with hundreds of retained
  features.
- An unattended JSON parameter surface supports an agent-requested rebuild,
  including explicit branch removal, following the conventions already used by
  `gd diff --params`.
- `gd schema restack` emits the runtime machine contract without accessing a
  repository or network. The existing `gd describe restack --json` spelling is
  retained. Both describe every argument, strict preview and apply payload
  schema, execution mode, result kind, exit code, validation rule, and security
  invariant.
- Machine preview parameters contain only `removeBranches`; callers do not
  submit a full retained set. Machine apply parameters contain the same
  `removeBranches` selection plus the preview's `planDigest`. Every removal
  must be a unique member of the captured explicit environment inventory.
  Unknown, graduated, indirect-only, or duplicate names fail validation rather
  than being ignored.
- Planning rejects a removal when any commit attributable to that explicit
  feature remains reachable through a retained feature and reports the
  dependent branches. Checking only the feature's current tip is insufficient
  because a retained branch can contain an earlier part of the feature. Restack
  never cascades removals or claims code was removed when the graph keeps it.
- Unattended v1 is JSON-only. `--params` or `--dry-run` selects machine mode;
  stdout emits one schema-versioned plan, apply result, or abort result, while
  progress and errors stay on stderr. `--dry-run` without `--params` uses the
  default empty removal set, retaining every discovered feature. Successful
  machine operations exit 0. A new non-TTY restack without either selector is
  a usage error. Resume is a separate machine invocation selected by
  `--resume <token>` and does not require `--params`. Table, YAML, CSV, and
  output-file modes are out of scope.
- Machine mode also emits schema-versioned, redacted JSON errors on stderr with
  stable `code`, `message`, and `details` fields plus conflict continuation
  fields when relevant. Invalid usage/params exit 2; fetch, Git, conflict,
  stale-plan/session, validation, and push failures exit 1.
- JSON schema v1 stays execution-focused. Plans include kind/version, remote,
  environment and base refs/OIDs, configured author, ordered retained and
  removed branch identities, dropped markers, per-merge preview outcomes,
  final tree OID, digest, and declared effects. Apply results include the
  digest, old/new environment OIDs, tree OID, merged/removed branches,
  resolution counts, and `pushed: true`. Conflict errors add the branch,
  unresolved paths, resume token, work-area path, and expiry. Jira payloads and
  raw rerere data are excluded. Abort emits a token-free `restackAbortResult`
  containing schema version 1, the environment name, `aborted: true`, and
  false `sourceCheckoutChanged`, `localRefsChanged`, `remoteRefsChanged`, and
  `personalRerereChanged` effects.
- Restack treats its operator, agent, repository, refs, commit messages, paths,
  and remote metadata as untrusted inputs. It rejects control characters,
  invalid Git ref syntax, and percent-encoded octets in ref components. Text
  copied from a repository remains JSON data, not instructions; consuming
  agents must not execute or follow repository-derived text.
- Discovery/planning does not mutate. Interactive execution requires an
  explicit confirmation after branch selection; unattended execution requires
  `--apply` plus a reviewed digest. `--dry-run` and JSON parameters select a
  preview and branch removals but do not authorize a push.
- Machine preview returns a canonical SHA-256 `planDigest` over the schema
  version, remote and ref names, credential-redacted identities of the single
  effective fetch and push endpoints, every captured environment, base, and
  feature OID, configured author identity, ordered feature names and tip OIDs,
  removal selection, and expected final tree. Machine `--apply` must echo that
  digest in `--params`; a fresh fetch or changed endpoint that changes any
  input fails and requires a new preview. Interactive confirmation binds to
  the same in-memory plan. Display JSON, map iteration order, and generated
  merge commit OIDs are not digest inputs.
- Digest fields use their validated UTF-8 bytes without additional string
  normalization. Each field is encoded as its tag followed by its value, with
  both byte strings prefixed by an unsigned 64-bit big-endian byte length.
  Tags appear in this order: `schema`, `remote`, `remote_fetch_sha256`,
  `remote_push_sha256`, `environment`, `environment_ref`, `environment_tip`,
  `main`, `main_ref`, `main_tip`, `author_name`, `author_email`, repeated
  `feature_name`/`feature_tip`, repeated `removed_name`/`removed_tip`, then
  `final_tree`. Endpoint identities are lowercase SHA-256 hashes of the
  effective UTF-8 endpoint strings returned by Git, with local paths made
  absolute, canonical, and represented as file URLs first; raw endpoints are
  neither serialized nor persisted, and remotes with multiple effective fetch
  or push endpoints are rejected. Feature pairs follow discovered merge order;
  removed pairs follow that same inventory order regardless of request order.
  The digest is the lowercase hexadecimal SHA-256 of that byte stream.
- Preview performs the full disposable reconstruction, including rerere replay
  and validation, but never pushes. It reports each clean or rerere-resolved
  merge and the final tree OID. Fetch uses an explicit remote-heads refmap,
  disables tag following, and preserves `FETCH_HEAD`; only the selected remote's
  remote-tracking namespace is updated. Apply fetches and reconstructs again,
  requires the same plan digest, verifies that the final tree matches preview,
  then performs the leased push. New committer timestamps can produce different
  merge commit OIDs during ordinary apply. Review instead binds the configured
  identity, canonical parent topology, merge order and messages, and final tree.
- Reconstruction validation is Git-only: resolved index, no conflict markers
  or diff-check failures, canonical merge parents/order/messages, reviewed
  final tree, and unchanged remote inputs. Graduate runs no repository build or
  test commands; agents and CI own project-specific validation.
- An unresolved conflict turns the isolated work area into a persisted,
  resumable session. Output includes an opaque local `resumeToken` and the work
  area path. After a human or agent edits and stages the resolution,
  `gd restack <ENVIRONMENT> --resume <token>` verifies the session state,
  records rerere, completes the remaining preview, and returns the plan digest.
  After approval, adding `--apply` revalidates every remote input and pushes
  that exact reviewed result. Resume-apply is the exception to rebuilding a
  second time: it uses the reviewed session so a newly learned resolution is
  not lost. Graduate seals a completed session, then holds its lock through
  apply while revalidating its final commit, tree, parents, metadata, and plan
  digest. A token becomes unusable after successful apply or explicit abort; a
  failed publication preserves the sealed session and token for another
  validated attempt. Graduate persists a non-replayable publication state
  immediately before push, so a process or cleanup failure after a successful
  remote update cannot leave sealed authority behind. Abandoned sessions
  expire and are purged without changing repository refs.
- Resumable sessions live in Graduate's mode-restricted platform cache with a
  24-hour inactivity TTL. Resume refreshes activity; every restack invocation
  purges expired sessions; successful apply or explicit abort deletes the
  session immediately. V1 adds no separate cleanup command.
- Session capabilities combine a non-secret lookup identifier with an
  unguessable secret whose digest authenticates use of the session. A separate
  mode-restricted store key authenticates atomically replaced metadata, so the
  continuation secret cannot sign replacement sealed plans. The secret is
  returned only in conflict continuation output and is not stored in that
  metadata. The session holds an exclusive lock during every transition.
- Resume verifies the canonical source Git common directory, explicit
  environment, optional remote/main assertions, isolated control files,
  expected HEAD, preserved HEAD reflog, MERGE_HEAD, and a fully staged
  resolution. It rejects untracked or unstaged work and any commit created in
  the work area before Graduate creates the canonical merge commit.
- When a conflict remains after rerere, interactive mode pauses in the isolated
  work area, identifies unresolved files, restores the terminal, then prints a
  three-step edit, stage, and resume handoff before exiting without pushing. It
  explicitly forbids creating a commit because Graduate creates the canonical
  merge commit after validation. Machine mode reports the same work area and
  continuation capability through structured conflict details. The capability
  appears nowhere except these explicit continuation outputs. The work area is
  preserved so a human or coding agent can inspect and resolve it with normal
  repository tools. A conflicting feature is never silently removed.
- Rebuilds should reuse prior merge-conflict resolutions with Git `rerere`,
  trained only from relevant accepted merge commits on the captured remote
  environment history. The command neither imports nor modifies the user's
  personal rerere cache.
- All rerere training and merge execution occurs in a temporary isolated Git
  work area backed by the repository's objects. The user's checkout, index,
  local environment ref, uncommitted changes, and personal rerere cache remain
  untouched.
- The final commit is pushed directly from the isolated work area without
  updating a local environment ref. The force update uses an explicit lease
  against the environment OID captured after fetch. Any concurrent remote
  change fails closed and requires a fresh discovery/rebuild.
- Preview and apply resolve and bind one effective fetch endpoint and one
  effective push endpoint for the selected remote. Apply uses the captured
  endpoint directly rather than resolving the remote name again, suppresses
  source and global push hooks, and never stores or reports a credential-bearing
  URL. A changed endpoint produces a different digest.
- Immediately before push, Graduate re-reads the remote environment, mainline,
  and all discovered feature refs (retained and removed) from the fetch and,
  when distinct, push endpoints and compares their OIDs with the reviewed plan.
  It also rechecks the configured identity and isolated result. Any moved or
  deleted input aborts. The target environment's explicit lease remains the
  atomic server-side guard; input-ref revalidation is best-effort because Git
  cannot atomically lease refs it is not updating.
- Delivery used one parent issue and bounded implementation slices for shared
  graph/core planning, isolated preview/rerere, resumable leased apply, and
  TUI/JSON/docs. The command became public only after every safety slice was
  complete.

## Important tradeoffs

- Before restack, Graduate only inspected Git history and reported promotion
  risk. `restack` is its first Git mutation workflow.
- Graduate's existing warning concerns repairing a feature branch polluted by
  an environment merge. The Restack prototype instead rebuilds an integration
  branch from a persisted, ordered topic membership list. These are different
  operations and cannot safely share an underspecified command.
- Restack's object-level `git merge-tree` and `git commit-tree` approach avoids
  touching the working tree or index. Its SQLite workspace, web UI, providers,
  promotion model, and broad command suite would significantly expand
  Graduate and should not be imported without a demonstrated requirement.
- The prototype updates the local environment ref before a
  `--force-with-lease` push, can leave incomplete rebuild records, mutates topic
  membership on conflicts, and does not wire its documented dry-run flag into
  the current public rebuild path. Those behaviors should not be copied as an
  implicit safety contract.
- Gitworkflow treats topic branches as first-class history and integration
  branches as ephemeral combinations that may be rewound and rebuilt. This
  supports reconstructing the environment without rewriting topics.
- `gd diff` already computes the central inventory: surviving remote feature
  refs reachable from the environment and absent from main. Reusing that
  predicate avoids two definitions of "not graduated." Recovered Jira-key rows
  for deleted refs are report-only and cannot be re-merged.
- The stricter reconstructability check intentionally makes `restack` reject
  some histories that `diff` can still report. This protects a shared branch
  rewrite from losing unattributed work.
- Git `rerere` normally learns resolutions as conflicts are manually resolved.
  Git's `contrib/rerere-train.sh` demonstrates training `.git/rr-cache` from
  existing merge commits, but it checks out historical parents and uses hard
  resets. `gd` needs an isolated or otherwise non-destructive adaptation rather
  than running that script in the user's working tree.
- An isolated work area costs temporary disk space and requires careful cleanup
  and credential/remote propagation, but it keeps a mutating command usable
  from dirty checkouts and makes failed reconstruction disposable.
- Merge order affects both resulting history and conflict applicability.
  `gd diff`'s alphabetical presentation order is not automatically a safe merge
  order.

## Relevant user reasoning

- The desired public entry point is a new `gd restack` command.
- A separate Restack prototype already explores integration-branch
  reconstruction and is available locally as design evidence.
- The command should infer its default feature set from the pre-reset
  environment, not require a separate SQLite membership database.
- The user wants prior conflict resolutions reused, removable branches exposed
  through a selection list, and the same capability available to an agent via
  JSON parameters.
- In the expected modern workflow, a coding agent may perform the restack,
  inspect a preserved conflict work area, and resolve the conflict itself; the
  CLI should not force conflict editing through an embedded TUI editor.

## Delivery record

The feature was delivered through the following bounded slices. The list is a
historical implementation outline, not an additional user-facing command
contract.

### 1. Record the contract

1. Create a parent GitHub issue from this plan plus linked implementation
   issues for graph/core planning, isolated preview/rerere, resumable leased
   apply, and TUI/JSON/docs. Claim each before implementation, following
   `docs/agents/issue-tracker.md`.
2. Add an ADR for the first remote-branch mutation workflow: ephemeral
   environment semantics, immutable preview/digest, isolated rerere training,
   resumable conflicts, and leased publication.
3. Update `CONTEXT.md` with the agreed meanings of environment, explicit
   feature merge, restack plan, graduated feature, and resumable session.
4. Keep the top-level command hidden or off the release branch until all
   slices satisfy the complete contract; do not publish a partially guarded
   mutating surface.

### 2. Add deterministic core planning

1. Add `crates/graduate/src/restack.rs` and export it from `lib.rs`.
2. Define typed snapshot, explicit-merge, branch identity, removal selection,
   dropped-marker, plan, merge-outcome, effect, and planning-error contracts.
3. Build plans from already-inspected graph data: enforce full attribution,
   first-explicit-merge ordering, graduation filtering, exact marker handling,
   strict removals, dependency rejection, and canonical merge intent.
4. Compute a canonical SHA-256 plan digest over schema version, remote/ref
   names, all captured OIDs, author identity, ordered branch inputs, removals,
   and expected final tree. Avoid hashing display JSON or map iteration order.
5. Model checklist/review/cancel transitions as deterministic core state so
   Ratatui tests can inject actions without touching a terminal.

### 3. Share environment graph inspection

1. Extract only the Gitoxide logic needed by both commands from `diff.rs` into
   a focused CLI Git module: ref validation, fetch credentials, remote-mainline
   discovery, reachability-based ungraduated candidates, inventories, and
   object lookup.
2. Keep `gd diff` output and ordering unchanged while moving this logic.
3. Add restack-only first-parent analysis that maps canonical two-parent merge
   commits to surviving remote feature refs, records first merge order and all
   relevant historical merges for rerere training, recognizes exact empty
   phase markers, and proves that every unique environment commit is covered.
4. Treat direct commits, fast-forwards, octopus merges, deleted refs, and
   ambiguous mappings as typed reconstructability failures with evidence.

### 4. Implement isolated reconstruction

1. Add `crates/graduate-cli/src/restack.rs` as the workflow module. It owns
   mandatory fetch, snapshot creation, core planning, preview, apply,
   revalidation, rendering choice, and redacted error translation.
2. Create a mode-restricted isolated Git work area that borrows local objects
   without checking out or changing the source repository. Never persist a
   credential-bearing remote URL.
3. Implement the behavior of Git's `contrib/rerere-train.sh` in controlled Rust
   orchestration: replay only relevant original environment merge parents,
   capture their accepted trees into the isolated rr-cache, and restore the
   disposable work area after each training merge. Do not invoke an installed
   contrib script or use the personal rr-cache.
4. Reset the isolated rebuild to the captured remote mainline, then merge each
   retained captured tip with `--no-ff`, canonical messages, configured
   identity, editor/signing disabled, an empty isolated hooks directory, and no
   shell interpolation. Every training and reconstruction command that can run
   Git hooks must override `core.hooksPath`; repository and global hooks must
   not run.
   Replace global and system Git configuration with empty isolated files and
   clear injected configuration before reconstruction. Apply only Graduate's
   allowlisted settings, so repository attributes cannot obtain executable
   merge-driver or clean/smudge-filter definitions from source, global,
   system, or ambient configuration.
5. Keep `rerere.autoupdate` disabled. Explicitly verify rerere remaining paths,
   the unmerged index, conflict markers, `git diff --check`, expected parents,
   order, messages, and final tree before considering preview complete.
6. On ordinary machine apply, repeat reconstruction from freshly fetched refs,
   verify the supplied digest and preview tree, re-read every input ref, and
   push the temporary commit directly to the environment with an exact-OID
   force-with-lease. Never update the source repository's local environment
   ref.

### 5. Persist and resume conflicts

1. Add `crates/graduate-cli/src/restack_session.rs` for cache location,
   atomic metadata, permissions, locking, expiry, cleanup, and resume-token
   validation.
2. Persist the source repository identity, environment/base/feature snapshot,
   selection, partial merge position, expected merge parent, work-area state,
   and expiry—but no PAT or credential-bearing URL.
3. On resume, require the expected source repository and environment, reject
   expired/locked/tampered state, require no agent-created commits, verify HEAD
   and merge parent, validate staged conflict resolution, run rerere, create the
   canonical merge commit, and continue preview. Seal the session when preview
   completes.
4. Let `--resume <token> --apply` publish the exact reviewed session after the
   same all-ref revalidation and environment lease. Hold the session lock and
   revalidate the final commit, tree, parents, metadata, and digest through the
   push. Make apply single-use.
5. Add `--resume <token> --abort` to delete an abandoned session immediately;
   otherwise purge after 24 hours of inactivity.

### 6. Add CLI and TUI surfaces

1. Add `RestackArgs` to `cli.rs` with `ENVIRONMENT`, `--main`, `--remote`,
   `--params`, `--dry-run`, `--apply`, `--resume`, and `--abort`, including
   Clap constraints:
   a new machine apply requires params plus a digest; resume takes no params;
   resume/abort combinations are explicit; there is no `--no-fetch` or
   output-format surface.
2. Route the command from `main.rs`. Select interactive mode only when stdin
   and stderr are terminals and neither params nor resume selects machine mode.
3. Add `restack_tui.rs`: loading, checked branch list, review, confirmation,
   success/cancel, and conflict handoff views. Use stderr, the shared terminal
   guard/theme, deterministic actions, and `TestBackend`; restore the terminal
   before printing a preserved conflict path.
4. Serialize the agreed schema-v1 plan/result to stdout and machine errors to
   stderr. Return the opaque resume capability only in the documented machine
   conflict continuation field or the post-restoration interactive handoff
   command. Redact it from all other output. Always redact PATs, credential
   helper values, remote credentials, and raw rerere contents.

### 7. Verify behavior

1. Core unit tests: canonical digest stability; ordering; graduation; strict
   removals; duplicate/unknown names; retained-branch dependencies; marker
   recognition; and every reconstructability error.
2. Real-Git fixture tests with a bare remote: canonical rebuild, current remote
   tips, mainline advancement, all branches removed, dirty source checkout
   untouched, no local branch mutation, expected remote-tracking updates, no
   preview push, exact leased apply, environment race, base/feature pre-push
   drift, deleted refs, direct commits, fast-forwards, octopus/ambiguous merges,
   protected/rejected pushes, and hostile repository and global hook
   configuration that never executes.
3. Rerere fixtures: train and replay an accepted old resolution; changed
   conflict that remains unresolved; no personal-cache read/write; conflict
   session creation; agent-staged resume; tampered/expired/concurrent resume;
   single-use apply; abort and expiry cleanup.
4. CLI contract tests: help/Clap constraints, non-TTY requirements, preview and
   apply JSON, structured stderr failures and exit codes, digest mismatch,
   secret redaction, and conflict continuation fields.
5. TUI tests: default-all-selected checklist, toggle/navigation, dependency
   rejection, review effects, confirmation/cancel, terminal restoration, and
   conflict handoff through Ratatui `TestBackend` only.
6. Run `cargo fmt --all -- --check`,
   `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`,
   `cargo test --workspace --locked`, generated-skill verification, and the Nix
   flake check used by CI.

### 8. Publish the user-visible feature

1. Update `README.md`, `docs/architecture.md`, and `CHANGELOG.md` with human and
   agent preview/apply/resume examples and the new safety invariants.
2. Extend the generated Graduate skill source in `generate_skills.rs`, run
   `cargo run --locked -- generate-skills --force`, and commit the generated
   `skills/graduate/SKILL.md` and `docs/skills.md` changes.
3. Add a minor changeset for `@treramey/graduate` because `restack` is a new
   user-visible command.

## Investigated context

- Graduate command surface: `crates/graduate-cli/src/cli.rs`
- Graduate promotion warning: `README.md`
- Graduate architecture: `docs/architecture.md`
- Sibling Restack command wrapper and rebuild algorithm (local design evidence
  reviewed during planning)
- Gitworkflow reference: <https://github.com/rocketraman/gitworkflow>
- Gitworkflow aliases: <https://gist.github.com/rocketraman/1fdc93feb30aa00f6f3a9d7d732102a9>
- Git rerere manual: <https://git-scm.com/docs/git-rerere>
- Git rerere training reference: <https://code.googlesource.com/git/+/8da1481bdcd6c85a0e8839df61a16180b9434f10/contrib/rerere-train.sh>
