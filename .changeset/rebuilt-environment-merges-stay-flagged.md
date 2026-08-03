---
"@treramey/graduate": patch
---

Keep environment-merge warnings on stale feature branches after the
environment branch is rebuilt or synced with pull-style self-merges.
Graduate now recognizes merge commits whose recorded subject names an
environment branch as the merge target or source, so `--no-ff` merges of an
environment into a feature branch stay flagged even after the environment
branch is reset.
