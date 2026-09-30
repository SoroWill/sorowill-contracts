#!/bin/bash
set -euo pipefail

# check-contract-version.sh — verify that CONTRACT_VERSION (in
# contracts/will/src/lib.rs), the crate `version` in
# contracts/will/Cargo.toml, and (when HEAD is tagged) the git version tag all
# agree on the same semantic version (#501).
#
# test.yml has invoked this script on every pull request ("Verify
# CONTRACT_VERSION matches git version tag") since check-changelog.sh's
# CONTRACT_VERSION guard was added for #439, and CONTRIBUTING.md's "Contract
# versioning" section documents the same check under "Before opening a PR" --
# but the script itself was never committed, so every PR touching
# contracts/will/src/lib.rs has been failing this CI step (or the step never
# actually ran, depending on how `bash <missing file>` was tolerated). Worse,
# the check it was meant to perform was already failing on main:
# contracts/will/Cargo.toml carried `version = "0.1.0"` while
# CONTRACT_VERSION decoded to "1.2.0" -- exactly the drift this script exists
# to catch, undetected because the script didn't exist (#501).
#
# This only checks *source-level* consistency, i.e. what will be true of the
# next build -- it cannot tell you whether a specific already-deployed
# contract instance is running a binary that matches this source tree. That
# is a separate, later check: scripts/deploy-testnet.sh calls the freshly
# deployed contract's `get_contract_version` right after deployment and
# compares it against CONTRACT_VERSION, because a source-only check can never
# catch a stale or wrong Wasm artifact being deployed.

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$repo_root"

LIB_RS="contracts/will/src/lib.rs"
CARGO_TOML="contracts/will/Cargo.toml"

# --- Extract CONTRACT_VERSION from lib.rs and decode it to semver ----------

raw_version_line="$(grep -E '^pub const CONTRACT_VERSION: u32 = ' "$LIB_RS" || true)"
if [[ -z "$raw_version_line" ]]; then
  echo "::error::could not find 'pub const CONTRACT_VERSION: u32 = ...' in $LIB_RS"
  exit 1
fi

# Pull out the numeric literal, stripping `_` digit separators (e.g. 1_002_000).
contract_version_raw="$(echo "$raw_version_line" | sed -E 's/^pub const CONTRACT_VERSION: u32 = ([0-9_]+);.*/\1/')"
contract_version="${contract_version_raw//_/}"

if ! [[ "$contract_version" =~ ^[0-9]+$ ]]; then
  echo "::error::failed to parse CONTRACT_VERSION as an integer from: $raw_version_line"
  exit 1
fi

major=$((contract_version / 1000000))
minor=$(((contract_version / 1000) % 1000))
patch=$((contract_version % 1000))
decoded_version="${major}.${minor}.${patch}"

# --- Extract the crate version from Cargo.toml ------------------------------

cargo_version="$(grep -m1 -E '^version = "' "$CARGO_TOML" | sed -E 's/^version = "(.*)"$/\1/')"

if [[ -z "$cargo_version" ]]; then
  echo "::error::could not find 'version = \"...\"' in $CARGO_TOML"
  exit 1
fi

echo "CONTRACT_VERSION ($LIB_RS):    $contract_version -> $decoded_version"
echo "crate version    ($CARGO_TOML): $cargo_version"

status=0

if [[ "$decoded_version" != "$cargo_version" ]]; then
  echo "::error::CONTRACT_VERSION decodes to '$decoded_version' but $CARGO_TOML has version '$cargo_version' -- bump both together (see CONTRIBUTING.md#contract-versioning)"
  status=1
fi

# --- If HEAD is exactly a git tag, it must match too ------------------------

git_tag="$(git -C "$repo_root" describe --tags --exact-match 2>/dev/null || true)"
if [[ -n "$git_tag" ]]; then
  tag_version="${git_tag#v}"
  echo "git tag:                        $git_tag -> $tag_version"
  if [[ "$tag_version" != "$decoded_version" ]]; then
    echo "::error::git tag '$git_tag' does not match CONTRACT_VERSION's decoded form '$decoded_version'"
    status=1
  fi
else
  echo "git tag:                        (HEAD is not tagged, skipping tag check)"
fi

if [[ "$status" -eq 0 ]]; then
  echo "OK: CONTRACT_VERSION, Cargo.toml, and any git tag agree on $decoded_version"
fi

exit "$status"
