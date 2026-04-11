#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

export PEANUTBUTTER_PATH="${PEANUTBUTTER_PATH:-$repo_root/examples}"
export PB_STATE_FILE="${PB_STATE_FILE:-/tmp/pb-smoke-part04-state.tsv}"

tmp_script="$(mktemp)"
trap 'rm -f "$tmp_script"' EXIT

cargo run -- --bash C+b > "$tmp_script"
bash -n "$tmp_script"

cat <<EOF
Part 04 bash smoke test

Generated bash integration:
  $tmp_script

Validation completed:
  - \`pb --bash C+b\` produced shell code
  - \`bash -n\` accepted the generated script

Manual interactive smoke:
  1. eval "\$(cargo run -- --bash C+b)"
  2. Press Ctrl+B inside an interactive bash prompt
  3. Select a snippet and confirm it is inserted into the readline buffer without executing
EOF
