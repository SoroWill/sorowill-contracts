#!/usr/bin/env bash
#
# deploy-testnet.sh — build and deploy the `will` contract to Stellar
# Testnet, then record the result in deployments/testnet.json.
#
# Prerequisites:
#   - stellar-cli >= 22.0.0 (`cargo install --locked stellar-cli`)
#   - rustup target wasm32v1-none (`rustup target add wasm32v1-none`)
#   - A funded testnet identity already configured in stellar-cli, e.g.:
#       stellar keys generate deployer --network testnet --fund
#     or, to import an existing key:
#       stellar keys add deployer --secret-key
#
# Configuration (environment variables):
#   DEPLOY_IDENTITY   Required. stellar-cli identity name used to sign and
#                      pay for the deployment. Must already exist and be
#                      funded on testnet.
#   NETWORK            Optional. Soroban network alias. Default: testnet
#   RPC_URL             Optional. Soroban RPC endpoint.
#                        Default: https://soroban-testnet.stellar.org
#
# Usage:
#   DEPLOY_IDENTITY=deployer ./scripts/deploy-testnet.sh
#
# After it finishes, review and commit the updated deployments/testnet.json
# — see CONTRIBUTING.md#updating-deploymentstestnetjson-after-a-redeploy.
#
# Version consistency (#501): CONTRACT_VERSION being correct in source is not
# the same thing as the *deployed* contract actually running that source. A
# stale local build, a wrong --wasm path, or deploying from the wrong git
# worktree would all still "succeed" as far as `stellar contract deploy` is
# concerned. So after deploying, this script calls the freshly deployed
# contract's own `get_contract_version` over RPC and fails loudly, before
# writing deployments/testnet.json, if it disagrees with the source
# CONTRACT_VERSION that was supposedly just built and deployed.

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

: "${DEPLOY_IDENTITY:?Set DEPLOY_IDENTITY to a funded stellar-cli identity name, e.g. DEPLOY_IDENTITY=deployer ./scripts/deploy-testnet.sh}"
NETWORK="${NETWORK:-testnet}"
RPC_URL="${RPC_URL:-https://soroban-testnet.stellar.org}"
WASM_PATH="target/wasm32v1-none/release/will.wasm"
OUTPUT_FILE="deployments/testnet.json"
LIB_RS="contracts/will/src/lib.rs"

command -v stellar >/dev/null 2>&1 || {
  echo "error: stellar-cli not found on PATH. Install it with:" >&2
  echo "  cargo install --locked stellar-cli" >&2
  exit 1
}

echo "==> Checking CONTRACT_VERSION is consistent with Cargo.toml before building"
"$ROOT_DIR/.github/scripts/check-contract-version.sh"

echo "==> Building contract for wasm32v1-none (release)"
cargo build --package will --release --target wasm32v1-none

if [[ ! -f "$WASM_PATH" ]]; then
  echo "error: expected wasm artifact at $WASM_PATH, but it was not produced" >&2
  exit 1
fi

echo "==> Deploying to '$NETWORK' as identity '$DEPLOY_IDENTITY'"
CONTRACT_ID="$(stellar contract deploy \
  --wasm "$WASM_PATH" \
  --source "$DEPLOY_IDENTITY" \
  --network "$NETWORK" \
  --rpc-url "$RPC_URL")"

if [[ -z "$CONTRACT_ID" ]]; then
  echo "error: stellar contract deploy did not return a contract id" >&2
  exit 1
fi

DEPLOYED_AT="$(date -u +"%Y-%m-%dT%H:%M:%SZ")"

echo "==> Deployed WillContract: $CONTRACT_ID at $DEPLOYED_AT"

echo "==> Verifying deployed contract's get_contract_version matches source CONTRACT_VERSION"
EXPECTED_VERSION_RAW="$(grep -E '^pub const CONTRACT_VERSION: u32 = ' "$LIB_RS" | sed -E 's/^pub const CONTRACT_VERSION: u32 = ([0-9_]+);.*/\1/')"
EXPECTED_VERSION="${EXPECTED_VERSION_RAW//_/}"

ONCHAIN_VERSION_RAW="$(stellar contract invoke \
  --id "$CONTRACT_ID" \
  --source "$DEPLOY_IDENTITY" \
  --network "$NETWORK" \
  --rpc-url "$RPC_URL" \
  -- get_contract_version)"
# stellar-cli prints scalar results as bare JSON (a plain number here); strip
# any surrounding quotes/whitespace defensively before comparing.
ONCHAIN_VERSION="$(echo "$ONCHAIN_VERSION_RAW" | tr -d '"[:space:]')"

if [[ "$ONCHAIN_VERSION" != "$EXPECTED_VERSION" ]]; then
  echo "error: deployed contract $CONTRACT_ID reports get_contract_version() = $ONCHAIN_VERSION, but source CONTRACT_VERSION is $EXPECTED_VERSION" >&2
  echo "error: the deployment is inconsistent with the source tree -- do NOT record this in $OUTPUT_FILE. Check for a stale build or wrong --wasm artifact." >&2
  exit 1
fi
echo "==> OK: on-chain version $ONCHAIN_VERSION matches source CONTRACT_VERSION"

cat > "$OUTPUT_FILE" <<JSON
{
  "WillContract": "$CONTRACT_ID",
  "network": "$NETWORK",
  "deployedAt": "$DEPLOYED_AT"
}
JSON

echo "==> Wrote $OUTPUT_FILE"
echo
echo "Next steps:"
echo "  1. Review the diff: git diff -- $OUTPUT_FILE"
echo "  2. Commit it on its own: git add $OUTPUT_FILE && git commit -m 'chore: record testnet deployment $CONTRACT_ID'"
echo "  3. See CONTRIBUTING.md#updating-deploymentstestnetjson-after-a-redeploy for the full checklist."
