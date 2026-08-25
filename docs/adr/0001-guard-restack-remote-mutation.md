---
status: accepted
---

# Guard restack remote mutation with reviewed, isolated reconstruction

`gd restack` is Graduate's first workflow that can change a remote Git ref. We
will let it replace only a selected remote environment branch. It must not
change feature branches, the source checkout or index, local branches
including the local environment branch, or the user's personal rerere cache.
A mandatory fetch can update the source repository's remote-tracking refs.
Deterministic planning belongs in `graduate`; fetching, Git execution,
filesystem and cache access, terminal handling, and publication belong in
`graduate-cli`.

## Decision

Each preview starts with a fetch and captures the mainline, environment, and
explicit feature ref OIDs. It performs the complete reconstruction in an
isolated Git work area but does not push. The resulting restack plan is
immutable. Its digest binds the captured OIDs, configured author, ordered
feature identities and tips, removal selection, reviewed final tree, schema
version, and remote and ref names. An interactive confirmation binds to the
same in-memory plan. Machine apply must provide the preview digest. Ordinary
apply can create different merge commit OIDs because it reconstructs the plan
with new committer timestamps. Review binds the final tree and canonical merge
identity, order, parents, and messages instead of the preview commit OIDs.

Apply fetches again and rejects any changed or deleted input. Except when it
continues a reviewed resumable session, apply reconstructs the plan again and
requires the same final tree. Graduate then re-reads every captured input ref
and pushes the isolated result directly to the remote environment ref. The
force update uses an exact lease against the captured environment OID and does
not update a local environment ref. Git cannot atomically lease refs that a
push does not update, so revalidation of the mainline and feature refs is a
best-effort pre-push guard. The environment lease is the final atomic guard.

Graduate trains and replays rerere only in the isolated work area. Training
uses relevant accepted explicit merges from the captured environment history.
It neither reads nor writes the user's personal rerere cache.

If rerere cannot resolve a conflict, Graduate preserves the isolated work area
as a resumable session in its mode-restricted platform cache. The session has
an opaque token and binds the source repository, captured refs, selected
features, partial merge position, and work-area state. Resume must reject an
expired, locked, or changed session and must verify the expected repository,
environment, HEAD, merge parent, and staged resolution before continuing. A
resumed apply publishes that exact reviewed session so a newly recorded
resolution is not lost. Successful apply and explicit abort delete the session
immediately and make its token unusable; a failed publication leaves the sealed
session and token available for another validated attempt. Inactive sessions
expire after 24 hours. After preview completes, Graduate seals the session.
Apply must hold the session lock through publication and revalidate the final
commit, tree, parents, metadata, and plan digest before it pushes.

The public `gd restack` command must remain hidden or absent from releases
until every safety slice in issue #12 is complete. Implementation branches may
build internal pieces, but no release may expose a partially guarded mutation
workflow.

## Considered options

- Rebuilding in the source checkout would reuse Git's normal state, but it
  could change the user's checkout, index, refs, and rerere cache.
- Applying an earlier preview after refs move would be faster, but the reviewed
  plan would no longer describe the remote inputs.
- Updating a local environment ref before push would make the result easy to
  inspect, but a failed push would still mutate local state.
- Publishing the command one safety slice at a time would provide earlier
  feedback, but it would expose a remote rewrite before all fail-closed guards
  exist.

## Consequences

Preview and ordinary apply repeat work, and isolated sessions consume temporary
disk space. In return, a dirty source checkout remains safe, review is bound to
exact inputs and output, conflict work can continue without relaxing the plan,
and concurrent remote changes fail closed. Remotes that require signed commits
will reject version 1 because its reviewed merge commits are explicitly
unsigned.

The complete product contract and delivery slices are recorded in
[`docs/gd-restack.md`](../gd-restack.md).
