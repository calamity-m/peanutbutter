#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

export PEANUTBUTTER_PATH="${PEANUTBUTTER_PATH:-$repo_root/examples}"
export PB_STATE_FILE="${PB_STATE_FILE:-/tmp/pb-smoke-part03-state.tsv}"

cat <<'EOF'
Part 03 smoke test

Keys:
  Ctrl+T / F2  toggle fuzzy <-> browse
  Enter         preview / accept
  Tab           complete browse directories or variable suggestions
  Backspace     walk back out of prompts and browse paths
  Esc           cancel

The command is emitted to stdout and not executed.
EOF

cargo run -- execute
