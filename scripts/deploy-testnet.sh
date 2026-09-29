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

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

: "${DEPLOY_IDENTITY:?Set DEPLOY_IDENTITY to a funded stellar-cli identity name, e.g. DEPLOY_IDENTITY=deployer ./scripts/deploy-testnet.sh}"
NETWORK="${NETWORK:-testnet}"
RPC_URL="${RPC_URL:-https://soroban-testnet.stellar.org}"
WASM_PATH="target/wasm32v1-none/release/will.wasm"
OUTPUT_FILE="deployments/testnet.json"

command -v stellar >/dev/null 2>&1 || {
  echo "error: stellar-cli not found on PATH. Install it with:" >&2
  echo "  cargo install --locked stellar-cli" >&2
  exit 1
}

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
