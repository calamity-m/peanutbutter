#!/usr/bin/env bash
# Called by prek during pre-push. Git passes pushed refs via stdin as:
#   <local ref> <local sha> <remote ref> <remote sha>
# Only run strict clippy (dead_code included) when pushing to main.
set -e

while read -r local_ref local_sha remote_ref remote_sha; do
    if [ "$remote_ref" = "refs/heads/main" ]; then
        cargo clippy -- -D warnings
        exit 0
    fi
done
