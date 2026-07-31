#!/usr/bin/env bash
set -euo pipefail

pnpm changeset version
VERSION=$(node -p "require('./npm/package.json').version")

sed -i.bak -E "s/^version = \"[^\"]+\"/version = \"${VERSION}\"/" Cargo.toml
sed -i.bak -E "s/(graduation = \{ version = \")[^\"]+/\1${VERSION}/" crates/graduation-cli/Cargo.toml
rm -f Cargo.toml.bak crates/graduation-cli/Cargo.toml.bak

cargo generate-lockfile
git add package.json npm Cargo.toml crates/graduation-cli/Cargo.toml Cargo.lock CHANGELOG.md .changeset
