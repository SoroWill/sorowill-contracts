#!/usr/bin/env bash
#
# Regenerates the resource-cost profile (contracts/will/src/profile.rs) and
# fails if it no longer matches the "## Current profile" table committed in
# docs/RESOURCE_COSTS.md, so a change that meaningfully shifts an entry
# point's ledger footprint can't silently leave the published numbers stale
# (issue #268).
#
# The profile is a deterministic measurement against soroban-sdk's mock Env
# (no wall-clock timing, no randomness), so an exact diff is the right check
# here, not a fuzzy tolerance.
#
# Usage: scripts/check_resource_costs.sh

set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

extract_table() {
  awk '/^\| entry point \|/{flag=1} flag && /^\|/{print} flag && !/^\|/{flag=0}'
}

echo "Running the resource-cost profile..."
profile_output="$(cargo test -p will --lib profile -- --nocapture 2>&1)"

generated_table="$(printf '%s\n' "$profile_output" | extract_table)"
committed_table="$(extract_table < docs/RESOURCE_COSTS.md)"

if [ -z "$generated_table" ]; then
  echo "Could not find a profile table in the test output. Full output:" >&2
  printf '%s\n' "$profile_output" >&2
  exit 1
fi

if [ "$generated_table" != "$committed_table" ]; then
  echo "Resource costs have drifted from docs/RESOURCE_COSTS.md." >&2
  echo "Re-run 'cargo test -p will --lib profile -- --nocapture' and update" >&2
  echo "the '## Current profile' table with the new numbers." >&2
  echo >&2
  echo "--- committed ---" >&2
  printf '%s\n' "$committed_table" >&2
  echo "--- freshly measured ---" >&2
  printf '%s\n' "$generated_table" >&2
  exit 1
fi

echo "Resource costs match docs/RESOURCE_COSTS.md."
