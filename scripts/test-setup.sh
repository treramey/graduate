#!/usr/bin/env bash
set -euo pipefail

ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
FIXTURE=$(mktemp -d -t graduate-setup-fixture-XXXXXX)
CONFIG="$FIXTURE/config.json"

cleanup() {
  if [ "${GRADUATE_SETUP_FIXTURE_KEEP:-0}" = 1 ]; then
    printf 'Fixture kept at: %s\n' "$FIXTURE" >&2
  else
    rm -rf "$FIXTURE"
  fi
}
trap cleanup EXIT

if ! (: <>/dev/tty) 2>/dev/null; then
  printf '%s\n' \
    'Could not open the setup TUI because this process has no controlling terminal.' \
    'Run `pnpm test:setup` directly in a terminal window.' >&2
  exit 2
fi

if ! command -v script >/dev/null 2>&1; then
  printf '%s\n' \
    'Could not open the setup TUI because the `script` pseudo-terminal command is unavailable.' >&2
  exit 2
fi

cargo build --quiet --locked --manifest-path "$ROOT/Cargo.toml" --features setup-ui-fixture

COMMAND=(
  env GRADUATE_SETUP_UI_FIXTURE=1
  "$ROOT/target/debug/gd"
  --config "$CONFIG"
  auth setup jira
  --no-open
)

exec 3<>/dev/tty
printf '%s\n' \
  'Opening the isolated Jira setup TUI.' \
  '' \
  'Use these fixture values:' \
  '  Jira site: demo.atlassian.net' \
  '  Email:     tester@example.com' \
  '  API token: demo-token' \
  '' \
  'The verifier is local; no browser or network request will run.' \
  "The saved configuration will be deleted when the test exits." >&3

read -r TERMINAL_ROWS TERMINAL_COLUMNS < <(stty size <&3)
if [ "$TERMINAL_ROWS" -le 0 ] || [ "$TERMINAL_COLUMNS" -le 0 ]; then
  TERMINAL_ROWS=30
  TERMINAL_COLUMNS=100
fi

printf -v ESCAPED_COMMAND '%q ' "${COMMAND[@]}"
PTY_COMMAND="stty rows $TERMINAL_ROWS cols $TERMINAL_COLUMNS; exec $ESCAPED_COMMAND"
if script --version >/dev/null 2>&1; then
  script -qefc "$PTY_COMMAND" /dev/null <&3 >&3 2>&3
else
  script -qe /dev/null /bin/sh -c "$PTY_COMMAND" <&3 >&3 2>&3
fi
