#!/usr/bin/env bash
set -euo pipefail

pnpm changeset version
VERSION=$(node -p "require('./npm/package.json').version")

sed -i.bak -E "s/^version = \"[^\"]+\"/version = \"${VERSION}\"/" Cargo.toml
sed -i.bak -E "s/(graduate = \{ version = \")[^\"]+/\1${VERSION}/" crates/graduate-cli/Cargo.toml
rm -f Cargo.toml.bak crates/graduate-cli/Cargo.toml.bak

cargo generate-lockfile
git add package.json npm Cargo.toml crates/graduate-cli/Cargo.toml Cargo.lock CHANGELOG.md .changeset
