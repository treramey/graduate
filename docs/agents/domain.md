# Domain Docs

How engineering agents should consume this repository's domain documentation.

## Before exploring

- Read `CONTEXT.md` at the repository root.
- Read ADRs under `docs/adr/` that touch the area being changed.
- If either source does not exist, proceed silently.

## File structure

This repository uses a single-context layout:

```text
/
├── CONTEXT.md
└── docs/adr/
```

Use terms defined in `CONTEXT.md`, and surface conflicts with existing ADRs
instead of silently overriding them.
