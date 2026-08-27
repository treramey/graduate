---
"@treramey/graduate": minor
---

Let interactive `gd restack` rebuild an environment whose history cannot be read by choosing which reachable branches stay and showing every commit that would be dropped; `restackPlan` moves to schema version 2 with `inventory`, `carriedBranches`, `orphanedCommits`, and `effects.reusedResolutions`. Reconstruction validation now rejects only leftover conflict markers, not whitespace errors in feature content.
