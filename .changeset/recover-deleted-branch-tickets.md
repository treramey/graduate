---
"@treramey/graduate": minor
---

Recover promoted work whose feature branch no longer exists. Hosting
platforms often delete the source branch when a pull request completes,
which made that work invisible to the branch-based promotion report even
though its commits still sit in the environment. Graduate now also scans
the non-merge commits unique to the environment, groups them by the Jira
key in their subject, and adds one row per key that no surviving branch
already covers. Merge commits are skipped, so environment sync merges
never produce rows.
