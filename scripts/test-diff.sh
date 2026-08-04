#!/usr/bin/env bash
set -euo pipefail

if [ "${1:-}" = "--" ]; then
  shift
fi

ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
FIXTURE=$(mktemp -d -t graduate-diff-fixture-XXXXXX)

cleanup() {
  if [ "${GRADUATE_DIFF_FIXTURE_KEEP:-0}" = 1 ]; then
    printf 'Fixture kept at: %s\n' "$FIXTURE" >&2
  else
    rm -rf "$FIXTURE"
  fi
}
trap cleanup EXIT

REMOTE="$FIXTURE/origin.git"
REPOSITORY="$FIXTURE/repository"
CONFIG="$FIXTURE/config.json"

git init --quiet --bare "$REMOTE"
git init --quiet --initial-branch=main "$REPOSITORY"
git -C "$REPOSITORY" config core.fsmonitor false
git -C "$REPOSITORY" config user.name "Graduate Test"
git -C "$REPOSITORY" config user.email "graduate-test@example.com"

printf 'base\n' > "$REPOSITORY/application.txt"
git -C "$REPOSITORY" add application.txt
GIT_AUTHOR_DATE=2024-01-01T00:00:00Z GIT_COMMITTER_DATE=2024-01-01T00:00:00Z \
  git -C "$REPOSITORY" commit --quiet -m "Create application"
git -C "$REPOSITORY" remote add origin "$REMOTE"
git -C "$REPOSITORY" push --quiet --set-upstream origin main

git -C "$REPOSITORY" checkout --quiet -b feature/DEMO-101-auth
printf 'authentication\n' > "$REPOSITORY/auth.txt"
git -C "$REPOSITORY" add auth.txt
GIT_AUTHOR_DATE=2024-02-01T00:00:00Z GIT_COMMITTER_DATE=2024-02-01T00:00:00Z \
  git -C "$REPOSITORY" commit --quiet -m "DEMO-101 Add authentication"
git -C "$REPOSITORY" push --quiet --set-upstream origin feature/DEMO-101-auth

git -C "$REPOSITORY" checkout --quiet main
git -C "$REPOSITORY" checkout --quiet -b feature/DEMO-202-search
printf 'search\n' > "$REPOSITORY/search.txt"
git -C "$REPOSITORY" add search.txt
GIT_AUTHOR_DATE=2024-03-01T00:00:00Z GIT_COMMITTER_DATE=2024-03-01T00:00:00Z \
  git -C "$REPOSITORY" commit --quiet -m "DEMO-202 Add search"
git -C "$REPOSITORY" push --quiet --set-upstream origin feature/DEMO-202-search

git -C "$REPOSITORY" checkout --quiet main
git -C "$REPOSITORY" checkout --quiet -b feature/DEMO-303-graduated
printf 'graduated\n' >> "$REPOSITORY/graduated.txt"
git -C "$REPOSITORY" add graduated.txt
GIT_AUTHOR_DATE=2024-04-01T00:00:00Z GIT_COMMITTER_DATE=2024-04-01T00:00:00Z \
  git -C "$REPOSITORY" commit --quiet -m "DEMO-303 Add graduated feature"
git -C "$REPOSITORY" push --quiet --set-upstream origin feature/DEMO-303-graduated
git -C "$REPOSITORY" checkout --quiet main
git -C "$REPOSITORY" merge --quiet --no-ff feature/DEMO-303-graduated -m "Graduate DEMO-303"
git -C "$REPOSITORY" push --quiet origin main

git -C "$REPOSITORY" checkout --quiet -b qa main
git -C "$REPOSITORY" merge --quiet --no-ff feature/DEMO-101-auth -m "Promote DEMO-101 to QA"
git -C "$REPOSITORY" merge --quiet --no-ff feature/DEMO-202-search -m "Promote DEMO-202 to QA"
git -C "$REPOSITORY" push --quiet --set-upstream origin qa
git -C "$REPOSITORY" remote set-head origin main

printf '%s\n' \
  "Created a promotion fixture:" \
  "  expected: feature/DEMO-101-auth" \
  "  expected: feature/DEMO-202-search" \
  "  excluded: feature/DEMO-303-graduated (already in main)" >&2

cargo build --quiet --locked --manifest-path "$ROOT/Cargo.toml"

cd "$REPOSITORY"

DIFF_COMMAND=(
  "$ROOT/target/debug/gd"
  --config "$CONFIG"
  diff qa
  --no-fetch
  "$@"
)

run_diff() {
  "${DIFF_COMMAND[@]}"
}

WANTS_TUI=1
for argument in "$@"; do
  case "$argument" in
    --report|--report=*|--params|--params=*|--format|--format=*|-o|--output|--output=*) WANTS_TUI=0 ;;
  esac
done

if [ "$WANTS_TUI" = 1 ]; then
  if (: <>/dev/tty) 2>/dev/null; then
    exec 3<>/dev/tty
    printf 'Opening the promotion-report TUI; press q to close.\n' >&3
    if ! command -v script >/dev/null 2>&1; then
      printf '%s\n' \
        'Could not open the TUI because the `script` pseudo-terminal command is unavailable.' >&3
      exit 2
    fi
    read -r TERMINAL_ROWS TERMINAL_COLUMNS < <(stty size <&3)
    if [ "$TERMINAL_ROWS" -le 0 ] || [ "$TERMINAL_COLUMNS" -le 0 ]; then
      TERMINAL_ROWS=30
      TERMINAL_COLUMNS=120
    fi
    printf -v ESCAPED_COMMAND '%q ' "${DIFF_COMMAND[@]}"
    PTY_COMMAND="stty rows $TERMINAL_ROWS cols $TERMINAL_COLUMNS; exec $ESCAPED_COMMAND"
    if script --version >/dev/null 2>&1; then
      # util-linux script (Linux): command is passed via -c
      script -qefc "$PTY_COMMAND" /dev/null <&3 >&3 2>&3
    else
      # BSD script (macOS): command is passed as positional arguments
      script -qe /dev/null /bin/sh -c "$PTY_COMMAND" <&3 >&3 2>&3
    fi
  else
    printf '%s\n' \
      'Could not open the TUI because this process has no controlling terminal.' \
      'Run `pnpm test:diff` directly in a terminal window.' >&2
    exit 2
  fi
else
  run_diff
fi
