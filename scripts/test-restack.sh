#!/usr/bin/env bash
set -euo pipefail

if [ "${1:-}" = "--" ]; then
  shift
fi

ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
FIXTURE=$(mktemp -d -t graduate-restack-fixture-XXXXXX)

cleanup() {
  if [ "${GRADUATE_RESTACK_FIXTURE_KEEP:-0}" = 1 ]; then
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

commit_file() {
  local branch=$1 path=$2 contents=$3 message=$4 date=$5
  git -C "$REPOSITORY" checkout --quiet main
  git -C "$REPOSITORY" checkout --quiet -b "$branch"
  printf '%s\n' "$contents" > "$REPOSITORY/$path"
  git -C "$REPOSITORY" add "$path"
  GIT_AUTHOR_DATE="$date" GIT_COMMITTER_DATE="$date" \
    git -C "$REPOSITORY" commit --quiet -m "$message"
  git -C "$REPOSITORY" push --quiet --set-upstream origin "$branch"
}

printf 'base\n' > "$REPOSITORY/application.txt"
git -C "$REPOSITORY" add application.txt
GIT_AUTHOR_DATE=2024-01-01T00:00:00Z GIT_COMMITTER_DATE=2024-01-01T00:00:00Z \
  git -C "$REPOSITORY" commit --quiet -m "Create application"
git -C "$REPOSITORY" remote add origin "$REMOTE"
git -C "$REPOSITORY" push --quiet --set-upstream origin main

commit_file feature/DEMO-101-auth auth.txt authentication \
  "DEMO-101 Add authentication" 2024-02-01T00:00:00Z
commit_file feature/DEMO-202-search search.txt search \
  "DEMO-202 Add search" 2024-03-01T00:00:00Z
commit_file feature/DEMO-303-graduated graduated.txt graduated \
  "DEMO-303 Add graduated feature" 2024-04-01T00:00:00Z

git -C "$REPOSITORY" checkout --quiet -b qa main
git -C "$REPOSITORY" merge --quiet --no-ff feature/DEMO-101-auth -m "Promote DEMO-101 to QA"
git -C "$REPOSITORY" merge --quiet --no-ff feature/DEMO-202-search -m "Promote DEMO-202 to QA"
git -C "$REPOSITORY" merge --quiet --no-ff feature/DEMO-303-graduated -m "Promote DEMO-303 to QA"
git -C "$REPOSITORY" push --quiet --set-upstream origin qa

git -C "$REPOSITORY" checkout --quiet main
git -C "$REPOSITORY" merge --quiet --no-ff feature/DEMO-303-graduated -m "Graduate DEMO-303"
git -C "$REPOSITORY" push --quiet origin main
git -C "$REPOSITORY" remote set-head origin main

printf '%s\n' \
  "Created a disposable restack fixture:" \
  "  retained: feature/DEMO-101-auth" \
  "  retained: feature/DEMO-202-search" \
  "  skipped:  feature/DEMO-303-graduated (already in main)" >&2

cargo build --quiet --locked --manifest-path "$ROOT/Cargo.toml"

cd "$REPOSITORY"
RESTACK_COMMAND=("$ROOT/target/debug/gd" --config "$CONFIG" restack qa)

if [ "${1:-}" = "--smoke" ]; then
  shift
  if [ "$#" -ne 0 ]; then
    printf 'Usage: pnpm test:restack -- --smoke\n' >&2
    exit 2
  fi
  command -v python3 >/dev/null 2>&1 || {
    printf 'The smoke test requires python3 to inspect JSON output.\n' >&2
    exit 2
  }

  LOCAL_HEAD=$(git rev-parse HEAD)
  LOCAL_QA=$(git rev-parse refs/heads/qa)
  REMOTE_QA=$(git --git-dir="$REMOTE" rev-parse refs/heads/qa)
  PLAN=$("${RESTACK_COMMAND[@]}" --params '{"removeBranches":[]}')
  DIGEST=$(python3 -c \
    'import json,sys; value=json.load(sys.stdin); assert value["kind"] == "restackPlan"; assert value["effects"]["pushed"] is False; print(value["planDigest"])' \
    <<<"$PLAN")
  PARAMS=$(python3 -c \
    'import json,sys; print(json.dumps({"removeBranches": [], "planDigest": sys.argv[1]}))' \
    "$DIGEST")
  RESULT=$("${RESTACK_COMMAND[@]}" --params "$PARAMS" --apply)
  python3 -c \
    'import json,sys; value=json.load(sys.stdin); assert value["kind"] == "restackResult"; assert value["pushed"] is True; assert value["planDigest"] == sys.argv[1]' \
    "$DIGEST" <<<"$RESULT"

  test "$(git rev-parse HEAD)" = "$LOCAL_HEAD"
  test "$(git rev-parse refs/heads/qa)" = "$LOCAL_QA"
  NEW_REMOTE_QA=$(git --git-dir="$REMOTE" rev-parse refs/heads/qa)
  test "$NEW_REMOTE_QA" != "$REMOTE_QA"
  printf 'Restack preview and leased apply passed; source checkout and local qa ref were unchanged.\n' >&2
  printf '%s\n' "$RESULT"
  exit 0
fi

if [ "$#" -gt 0 ]; then
  exec "${RESTACK_COMMAND[@]}" "$@"
fi

if ! (: <>/dev/tty) 2>/dev/null; then
  printf '%s\n' \
    'Could not open the TUI because this process has no controlling terminal.' \
    'Run `pnpm test:restack` directly in a terminal, or use `pnpm test:restack -- --smoke`.' >&2
  exit 2
fi
if ! command -v script >/dev/null 2>&1; then
  printf 'Could not open the TUI because the `script` pseudo-terminal command is unavailable.\n' >&2
  exit 2
fi

exec 3<>/dev/tty
printf 'Opening the restack TUI against a disposable remote.\n' >&3
read -r TERMINAL_ROWS TERMINAL_COLUMNS < <(stty size <&3)
if [ "$TERMINAL_ROWS" -le 0 ] || [ "$TERMINAL_COLUMNS" -le 0 ]; then
  TERMINAL_ROWS=30
  TERMINAL_COLUMNS=120
fi
printf -v ESCAPED_COMMAND '%q ' "${RESTACK_COMMAND[@]}"
PTY_COMMAND="stty rows $TERMINAL_ROWS cols $TERMINAL_COLUMNS; exec $ESCAPED_COMMAND"
if script --version >/dev/null 2>&1; then
  script -qefc "$PTY_COMMAND" /dev/null <&3 >&3 2>&3
else
  script -qe /dev/null /bin/sh -c "$PTY_COMMAND" <&3 >&3 2>&3
fi
