#!/usr/bin/env bash
# Builds the release wasm and exports the contract's embedded spec
# (function signatures, Will/Beneficiary/Guardian/WillStatus/WillError
# types) as a versioned JSON file under spec/.
#
# Usage:
#   scripts/export-spec.sh           write spec/will-v<version>.json
#   scripts/export-spec.sh --check   verify the committed snapshot is current
#
# --check exports to a temporary file and compares it against the committed
# snapshot. It exits non-zero when they differ, or when no snapshot exists for
# the crate version currently declared — the case where a public signature
# changed and the version was bumped but the export step was skipped.
#
# Requires: cargo, the wasm32v1-none target, stellar-cli (`cargo install
# --locked stellar-cli --features opt`), and jq.
#
# STELLAR_BIN overrides the exporter binary (used by the tests).
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

check_mode=0
if [[ "${1:-}" == "--check" ]]; then
  check_mode=1
elif [[ -n "${1:-}" ]]; then
  echo "Unknown argument: $1" >&2
  echo "Usage: scripts/export-spec.sh [--check]" >&2
  exit 2
fi

version="$(cargo metadata --no-deps --format-version 1 \
  | jq -r '.packages[] | select(.name=="will") | .version')"

wasm_path="target/wasm32v1-none/release/will.wasm"
out_path="spec/will-v${version}.json"
stellar_bin="${STELLAR_BIN:-stellar}"

echo "Building will contract (release, wasm32v1-none)..."
cargo build -p will --release --target wasm32v1-none

if [[ "${check_mode}" -eq 0 ]]; then
  echo "Exporting spec to ${out_path}..."
  "${stellar_bin}" contract bindings json \
    --wasm "${wasm_path}" \
    --output "${out_path}"
  echo "Done. Diff ${out_path} against the previous version to review drift."
  exit 0
fi

# ── --check ──────────────────────────────────────────────────────────────────

if [[ ! -f "${out_path}" ]]; then
  echo "ERROR: ${out_path} does not exist." >&2
  echo "contracts/will/Cargo.toml declares version ${version}, so a spec snapshot" >&2
  echo "must be committed for it. Run scripts/export-spec.sh and commit the result." >&2
  exit 1
fi

tmp_dir="$(mktemp -d)"
trap 'rm -rf "${tmp_dir}"' EXIT
fresh_path="${tmp_dir}/will-v${version}.json"

echo "Exporting current spec for comparison..."
"${stellar_bin}" contract bindings json \
  --wasm "${wasm_path}" \
  --output "${fresh_path}"

# Compare parsed JSON rather than bytes, so formatting or key ordering from a
# different stellar-cli build is not reported as a contract change.
jq -S . "${out_path}" > "${tmp_dir}/committed.norm"
jq -S . "${fresh_path}" > "${tmp_dir}/fresh.norm"

if diff -q "${tmp_dir}/committed.norm" "${tmp_dir}/fresh.norm" >/dev/null; then
  echo "OK: ${out_path} matches the compiled contract."
  exit 0
fi

echo "ERROR: ${out_path} does not match the compiled contract." >&2
echo "The public interface changed without a matching spec export." >&2
echo >&2
diff -u "${tmp_dir}/committed.norm" "${tmp_dir}/fresh.norm" >&2 || true
echo >&2
echo "Fix: if this is a public interface change, bump the version in" >&2
echo "contracts/will/Cargo.toml, run scripts/export-spec.sh, and commit the" >&2
echo "new file. Previously published spec files are never edited." >&2
exit 1
