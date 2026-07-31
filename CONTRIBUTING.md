# Contributing

Discuss substantial behavior or compatibility changes before implementation.

## Local checks

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --locked
```

If installed, also run `cargo deny check`.

Keep deterministic behavior in `crates/graduate` and side effects in
`crates/graduate-cli`. Public changes require tests, README updates, and a
`CHANGELOG.md` entry.

User-visible changes also require a changeset. Run `pnpm install` once, then
`pnpm changeset`, select `@treramey/graduate`, and commit the generated file.
Merging the resulting release-version PR prepares Cargo, npm, and changelog
versions; pushing its `vX.Y.Z` tag publishes native archives, npm, and Homebrew
artifacts.
