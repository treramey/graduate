---
status: accepted
---

# Exclude tainted features from restack

A feature branch that merged its environment branch into itself reaches every
other feature the environment had promoted at that moment. Graduate calls such
a branch a tainted feature. Retaining it in a rebuild would re-import every
other unreleased feature through its tip, including features the operator
deliberately removed, and would defeat the purpose of the rebuild.

## Decision

`graduate::restack` detects tainted features in both history and inventory
mode. An environment merge is a two-parent merge on the environment's
first-parent history whose second parent is not on main. A feature is tainted
when its tip reaches an environment merge that is neither one of the merges
that promoted the feature itself nor a merge on the feature's own first-parent
line (the environment may have been fast-forwarded onto the branch). The
snapshot records each tainted feature with its tip and the absorbed merge ids
and binds them into the plan digest through the schema version.

A tainted feature can never be retained, interactively or by machine. The
interactive checklist starts it removed, marks it with a `↳ tainted` sub-row,
rejects toggling it back on, and names the remediation: recreate the branch
from main and cherry-pick its commits. The machine path unions every tainted
feature into `removeBranches` before selection, so `--dry-run` without
`--params` still keeps every other discovered feature. `restackPlan` schema
version 3 adds `taintedBranches`; persisted schema-2 sessions fail closed as
mismatched.

Commit attribution follows ownership, not reachability. A tainted feature owns
only the commits it reaches without passing through an absorbed merge, so
removing it never trips the retained-dependency rule for the features whose
work it absorbed, and in inventory mode it never carries them.

## Consequences

- A rebuild is safe by default even when a branch merged the environment.
- Owners of tainted branches must recreate them; Graduate does not attempt to
  strip environment merges from a feature branch.
- The rule is conservative when the environment was fast-forwarded onto a
  feature: merges on that feature's own first-parent line never taint it.
