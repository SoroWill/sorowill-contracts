<img src="./docs/logo.svg" alt="SoroWill" width="56" height="56" />

# SoroWill Contracts

**Trustless on-chain inheritance on Stellar Soroban**

[![Rust](https://img.shields.io/badge/Rust-1.84%2B-orange?logo=rust)](https://www.rust-lang.org/)
[![Soroban SDK](https://img.shields.io/badge/Soroban%20SDK-22.0.0-7D00FF)](https://developers.stellar.org/docs/build/smart-contracts)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](./LICENSE)
[![Security Policy](https://img.shields.io/badge/Security-Policy-blue.svg)](./SECURITY.md)
[![Stellar Testnet](https://img.shields.io/badge/Stellar-Testnet-08b5e5?logo=stellar)](https://developers.stellar.org/docs/networks)

**Live app: [sorowill.vercel.app](https://sorowill.vercel.app/)**

## What is SoroWill

SoroWill is a trustless, on-chain inheritance protocol for Stellar Soroban. It lets anyone lock USDC (or any SEP-41 compliant token) into a smart contract, name beneficiaries with percentage splits, and set a check-in period. If the owner stops checking in, the contract automatically releases the funds to the beneficiaries after a grace period — no lawyer, no court, no middleman.

## How it works

1. **Create a will.** The owner calls `create_will`, locking a token balance and specifying beneficiaries (with percentage shares), a check-in period (e.g. 90 days), and a grace period (e.g. 7 days).
2. **Check in.** The owner calls `check_in` periodically, before the deadline, to reset the countdown and prove they are still active.
3. **Trigger.** If the owner misses a check-in deadline, anyone can call `trigger_will`, which starts the grace period.
4. **Prove you're alive.** During the grace period, the owner can call `emergency_checkin` to cancel the trigger and reset the countdown.
5. **Release.** If the grace period expires without an emergency check-in, anyone can call `release_inheritance`, which distributes the locked balance to every beneficiary proportionally, in one transaction.
6. **Cancel anytime.** While the will is active, the owner can call `cancel_will` to withdraw the full balance.
7. **Update beneficiaries.** While active, the owner can call `update_beneficiaries` to change who inherits and in what proportions.
8. **Guardian override.** A will can name up to 3 guardians. Any 2 of them calling `guardian_trigger` force an immediate release — useful if the owner is known to be incapacitated rather than simply inactive. See [docs/adr/0001-guardian-threshold.md](./docs/adr/0001-guardian-threshold.md) for the rationale behind the 2-of-3 default, its known limitations, and how it relates to the proposed configurable M-of-N guardian feature.

## Tech Stack

- **Rust** 1.84+
- **soroban-sdk** 22.0.0
- **stellar-cli** for building and deploying to Soroban networks

## Local Setup

```bash
# Install Rust (if you don't already have it)
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# Add the Soroban wasm target
rustup target add wasm32v1-none

# Install the Stellar CLI
cargo install --locked stellar-cli --features opt

# Clone and test
git clone https://github.com/SoroWill/sorowill-contracts.git
cd sorowill-contracts
cargo test
cargo clippy --all-targets -- -D warnings
```

### Task runner (recommended)

This repo ships a [`justfile`](./justfile) with the exact commands CI runs, so you don't have to remember or copy them from this README. Install [`just`](https://github.com/casey/just#installation), then:

```bash
just --list    # see every available recipe
just test      # cargo test --workspace
just lint      # cargo clippy --all-targets -- -D warnings (CI's exact flags)
just build     # cargo build --workspace --release --target wasm32v1-none
just fmt       # cargo fmt --all
just ci        # run everything CI runs, in order
```

If you don't have `just` installed, the raw `cargo` commands above work identically.

## Resource costs

Every entry point is profiled for the resources Soroban bills — CPU
instructions, ledger entries read and written, and storage rent:

```bash
cargo test -p will --lib profile -- --nocapture
```

See [docs/RESOURCE_COSTS.md](./docs/RESOURCE_COSTS.md) for the current numbers,
what drives each entry point's cost, and the storage layout trade-offs behind
them. The release build profile is separately tuned for `.wasm` binary size
(deploy cost) — see [docs/WASM_SIZE.md](./docs/WASM_SIZE.md).

## Testing and fuzzing

`cargo test` runs the hand-written suite in `contracts/will/src/test.rs`
alongside a property-based fuzzing suite that drives `create_will` and
`update_beneficiaries` with malformed and edge-case input, checking that the
contract never aborts and that an accepted will always satisfies its
documented invariants.

For deeper, coverage-guided fuzzing there are `cargo-fuzz` targets under
[`fuzz/`](./fuzz):

```bash
cargo install cargo-fuzz
cd fuzz && cargo +nightly fuzz run create_will
```

See [docs/FUZZING.md](./docs/FUZZING.md) for the invariants that are checked,
how to reproduce and minimise a crash, and how to add a target.

## Contract Constants

The following limits are defined as `pub const` in `lib.rs` and re-exported from the crate root. They are the canonical source of truth — off-chain tooling, test harnesses, and SDK integrations should import them directly rather than hardcoding duplicates that can silently drift out of sync.

| Constant | Value | Meaning |
|---|---|---|
| `MAX_BENEFICIARIES` | `10` | Maximum number of beneficiaries per will |
| `MAX_GUARDIANS` | `3` | Maximum number of guardians per will |
| `GUARDIAN_THRESHOLD` | `2` | Default number of guardian votes required to force an early release |

```rust
use will::{MAX_BENEFICIARIES, MAX_GUARDIANS, GUARDIAN_THRESHOLD};
```

## Contract Functions

| Function | Description | Parameters | Returns |
|---|---|---|---|
| `create_will` | Locks a token balance and creates a new will | `owner`, `token`, `amount`, `beneficiaries`, `checkin_period_days`, `grace_period_days`, `guardians` | `u64` (will id) |
| `check_in` | Resets the check-in countdown | `will_id`, `owner` | — |
| `trigger_will` | Starts the grace period after a missed check-in | `will_id` | — |
| `emergency_checkin` | Cancels an in-progress trigger during the grace period | `will_id`, `owner` | — |
| `release_inheritance` | Distributes the balance to beneficiaries after the grace period expires | `will_id` | — |
| `cancel_will` | Withdraws the full balance and closes the will | `will_id`, `owner` | — |
| `update_beneficiaries` | Replaces the beneficiary list before the will is triggered | `will_id`, `owner`, `beneficiaries` | — |
| `top_up` | Adds more of the token to an existing will | `will_id`, `owner`, `amount` | — |
| `get_will` | Reads the full state of a will | `will_id` | `Will` |
| `get_will_status` | Reads only a will's lifecycle status, without loading the rest of the struct | `will_id` | `WillStatus` |
| `get_time_until_deadline` | Seconds until the will's next relevant deadline (check-in or grace period); negative if past due, `None` if not applicable to the current status | `will_id` | `Option<i64>` |
| `get_wills_by_owner` | Lists every will owned by an address | `owner` | `Vec<Will>` |
| `get_wills_by_beneficiary` | Lists every will an address is named in | `beneficiary` | `Vec<Will>` |
| `guardian_trigger` | Casts a guardian vote; 2 of 3 forces an early release | `will_id`, `guardian` | — |

`checkin_period_days` and `grace_period_days` passed to `create_will` must each be at least `1` day (and at most `MAX_PERIOD_DAYS`); a value of `0` panics with `WillError::InvalidPeriod`.

### Reading wills and Soroban's archival model (issue #166)

`get_will` and `get_wills_by_owner` / `get_wills_by_beneficiary` read a will's
persistent entry via `storage::load_will`, which returns
`WillError::WillNotFound` whenever the key is absent.

Soroban's persistent-storage API does **not** expose *why* a key is absent, so
`WillNotFound` intentionally conflates three situations that are
indistinguishable on-chain with soroban-sdk 22:

1. **Never created** — the will id was never allocated.
2. **Explicitly archived** — the will reached a terminal state and was moved
   to the `ArchivedWill` namespace by `archive_will`.
3. **TTL-archived by the network** — a terminal will's entry stopped renewing
   its TTL (see `storage::save_will`) and was archived by Soroban once its TTL
   hit zero. On the live network, an invocation that touches an archived entry
   fails at the host level before contract code runs; in the test host it
   surfaces as a plain `None`.

**Consumers should therefore treat `WillNotFound` as "no readable will at this
id"** — it cannot distinguish a will that exists but needs to be restored (or
was explicitly archived) from one that never existed. This is documented on
`storage::load_will` and the `get_will` entry point; a dedicated
`WillArchived` error code is deferred until the SDK exposes an archived-entry
probe. See [issue #166](https://github.com/SoroWill/sorowill-contracts/issues/166)
for the full context.

## Error codes

Every failure mode is a `#[contracterror]` variant of `WillError`
(defined in [`contracts/will/src/errors.rs`](./contracts/will/src/errors.rs)),
surfaced to callers as a stable numeric code so SDK and app consumers can
match on the code without parsing panic messages. Note that a few codes are
intentionally shared by more than one variant below — check the error's
context (which entry point raised it, and the will's current state) to
disambiguate.

| Code | Variant | Meaning |
|---|---|---|
| 1 | `WillNotFound` | No will exists for the given identifier. |
| 2 | `NotOwner` | The caller is not the owner of the will. |
| 3 | `WillNotActive` | The requested action requires the will to be `Active`. |
| 4 | `WillNotTriggered` | The requested action requires the will to be `Triggered`. |
| 5 | `GracePeriodNotExpired` | `release_inheritance` was called before the grace period elapsed. |
| 6 | `GracePeriodExpired` | `emergency_checkin` was called after the grace period already elapsed. |
| 7 | `InvalidPercentages` | Beneficiary percentages did not sum to exactly 10,000 basis points. |
| 8 | `AlreadyVoted` | The guardian has already voted to trigger this will. |
| 9 | `NotGuardian` | The caller is not a designated guardian of this will. |
| 10 | `CheckinNotDue` | `trigger_will` was called before the check-in deadline passed. |
| 11 | `ZeroAmount` | An amount of zero (or less) was supplied where a positive amount is required. |
| 12 | `TooManyBeneficiaries` | Too many beneficiaries (or guardians) were supplied. |
| 13 | `WillNotSettled` | The requested action requires the will to be `Released` or `Cancelled`. |
| 14 | `WillNotBothActive` | Both wills in a merge must be `Active`. |
| 15 | `SameWillId` | The same will id was supplied for both sides of a merge. |
| 16 | `MergeWouldExceedLimits` | Merging would exceed the maximum beneficiaries or guardians. |
| 17 | `OwnerCannotBeGuardian` | The owner cannot designate themselves as a guardian of their own will. |
| 18 | `BeneficiaryNotFound` | A beneficiary is not found in the will's beneficiary list. |
| 19 | `KeeperBountyExceedsMax` | Keeper bounty basis points exceed the maximum allowed (100 bps / 1%). |
| 20 | `InvalidGuardianThreshold` | Guardian threshold is out of range (must be between 1 and `guardians.len()`). |
| 21 | `FixedAmountExceedsBalance` | The sum of every `Allocation::FixedAmount` beneficiary exceeds the will's balance, or (for a will with no percentage-based beneficiaries) does not exactly account for the whole balance. |
| 22 | `InvalidPercentage` | A beneficiary percentage is not in the valid range (1..=10000 basis points). |
| 23 | `WillNotReleased` | The requested action requires the will to be `Released`. |
| 24 | `NotSameOwner` | Cannot merge: both wills must be owned by the same address. |
| 25 | `InvalidPeriod` | A check-in or grace period was zero, or long enough that the resulting deadline could not be represented as a ledger timestamp. |
| 26 | `DuplicateGuardian` | The same address was supplied more than once in a guardian list. |
| 27 | `GuardianCooldownActive` | The guardian-list cooldown has not yet elapsed; `guardian_trigger` is blocked until the cooldown period passes after the last guardian-list change. |
| 28 | `InvalidToken` | A supplied token address does not respond to a read-only `decimals()` probe, indicating it is not a valid SEP-41 token. |
| 29 | `DuplicateBeneficiary` | The same beneficiary address was supplied more than once. |
| 30 | `WillNotConfirmed` | `confirm_will` was called on a will that is not `PendingConfirmation`. |
| 31 | `ConfirmationWindowExpired` | `confirm_will` was called after the confirmation deadline elapsed. |
| 32 | `TooManyIds` | `get_wills` was called with more ids than `MAX_GET_WILLS_IDS`. |
| 33 | `InsufficientBalance` | `split_will` was asked to move more of a token than the will currently holds of it. |
| 34 | `InvalidSplit` | `split_will` was called with an empty beneficiary-to-split list, or a split that would leave the source or new will with an invalid state. |
| 35 | `InvalidPreimage` | `reveal_and_claim` was called with a pre-image that does not match any stored `HashedBeneficiary` commitment on the will. |
| 36 | `AlreadyClaimed` | `reveal_and_claim` was called for a hashed beneficiary slot that has already been claimed. |
| 37 | `TooManyWills` | An owner or beneficiary index list is already at `MAX_WILLS_PER_INDEX` and cannot accept another will id. |

## Contract spec artifact

A versioned, machine-readable export of the contract's public interface —
every entry-point signature plus the `Will`, `Beneficiary`, `Guardian`,
`WillStatus`, and `WillError` types — is committed under
[`spec/`](./spec) as `will-v<crate-version>.json`. It's how
[`sorowill-sdk`](https://github.com/SoroWill/sorowill-sdk) detects when its
TypeScript types and XDR encoders have drifted from the deployed contract.

Regenerate it after any change to a public entry point or shared type:

```bash
./scripts/export-spec.sh
```

A new file is committed per crate version rather than overwriting the
previous one, so SDK maintainers can diff any two versions. The `Spec
Export` GitHub Actions workflow (`.github/workflows/spec-export.yml`) also
runs this on pushes to `main` that touch the contract, and attaches the
resulting JSON to tagged GitHub Releases. See [`spec/README.md`](./spec/README.md)
for the full update process.

## Testnet Deployment

The deployed contract ID for Stellar Testnet is recorded in [`deployments/testnet.json`](./deployments/testnet.json):

```json
{
  "WillContract": "<contract-id>",
  "network": "testnet",
  "deployedAt": "<ISO-8601 timestamp>"
}
```

Redeploying is automated via [`scripts/deploy-testnet.sh`](./scripts/deploy-testnet.sh), which builds the release wasm, deploys it with `stellar contract deploy`, and rewrites this file with the new contract id and timestamp:

```bash
# One-time: create and fund a testnet identity
stellar keys generate deployer --network testnet --fund

# Build, deploy, and record the result in deployments/testnet.json
DEPLOY_IDENTITY=deployer ./scripts/deploy-testnet.sh
```

Requires `stellar-cli` (same version as [Local Setup](#local-setup)) and a funded testnet identity passed via `DEPLOY_IDENTITY`. `NETWORK` and `RPC_URL` are optional overrides — see the script header for details.

After running it, review and commit the updated `deployments/testnet.json` on its own — see [CONTRIBUTING.md](./CONTRIBUTING.md#updating-deploymentstestnetjson-after-a-redeploy) for the full checklist. A scheduled CI job also checks daily that this file's contract id still matches the on-chain wasm, so a forgotten update won't drift silently — see [Testnet Deployment Drift Check](.github/workflows/testnet-drift-check.yml).

## Security Policy

Security reports and responsible disclosure guidelines are documented in [`SECURITY.md`](./SECURITY.md). Please do not open public GitHub issues for security vulnerabilities.

## Contributing via Drips Wave

This repo participates in the **Stellar Wave Program** on [Drips](https://drips.network/wave). Maintainer-tagged issues carry Point values, and contributors who resolve them during an active Wave earn a proportional share of that Wave's reward pool. See [CONTRIBUTING.md](./CONTRIBUTING.md) for the contribution workflow, and <https://drips.network/wave> for how Wave itself works.
