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
8. **Guardian override.** A will can name up to 3 guardians. Once the will's guardian threshold is reached (2 by default, configurable per will), their `guardian_trigger` calls force an immediate release — useful if the owner is known to be incapacitated rather than simply inactive. See [docs/adr/0001-guardian-threshold.md](./docs/adr/0001-guardian-threshold.md) for the rationale behind the 2-of-3 default, its known limitations, and how it relates to the proposed configurable M-of-N guardian feature.

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

# Install the Stellar CLI (>= 22.0.0)
cargo install --locked stellar-cli

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
| `MAX_BENEFICIARIES` | `10` | Maximum number of beneficiaries per will — see [the FAQ](#faq-why-is-there-a-beneficiary-limit) |
| `MAX_GUARDIANS` | `3` | Maximum number of guardians per will |
| `GUARDIAN_THRESHOLD` | `2` | Default number of guardian votes required to force an early release |
| `MAX_BENEFICIARIES` | `10` | Maximum number of beneficiaries per will |
| `MAX_GUARDIAN_WEIGHT` | `1000000` | Maximum vote weight a single guardian may be given via `update_guardians_weighted`. A weight of `0` is normalised to `1` |
| `MAX_GUARDIANS` | `3` | Maximum number of guardians per will (private to the crate, not exported) |
| `GUARDIAN_THRESHOLD` | `2` | Default number of guardian votes required to force an early release (private to the crate, not exported) |

```rust
use will::MAX_BENEFICIARIES;
```

> **Beneficiary limit:** a will can name at most **10 beneficiaries**.
> `create_will`, `batch_create_wills`, `update_beneficiaries` and
> `update_will_settings` reject an 11th with `TooManyBeneficiaries` (code `12`);
> `merge_wills` rejects a merge that would exceed it with
> `MergeWouldExceedLimits` (code `16`). The diagnostic log records the
> supplied count and the limit, so it shows up when you simulate the
> transaction. Validate the list length client-side before submitting.

### FAQ: why is there a beneficiary limit?

`release_inheritance` pays every beneficiary in **one transaction**, with one
token transfer per beneficiary and token. Soroban caps the CPU instructions,
memory and ledger entries a single transaction may use. If the list had no
upper bound, a will could grow until its release no longer fits in those
limits. At that point nobody could release the funds, and they would stay
locked for good. Ten beneficiaries, combined with `MAX_TOKENS` tokens, stays
comfortably inside the budget measured in
[docs/RESOURCE_COSTS.md](./docs/RESOURCE_COSTS.md).

If you need more than 10 heirs, you can:

- **Split across several wills.** Create more than one will, each with at most
  10 beneficiaries, or use `split_will` on an existing one.
- **Nest the distribution.** Name an intermediary address, such as a
  multisig or another contract, as a single beneficiary and distribute
  further from there.

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
| `get_wills_by_owner` | Lists a page of wills owned by an address | `owner`, `cursor`, `limit` | `Vec<Will>` |
| `get_owner_stats` | Aggregate stats for an owner: total wills, non-terminal wills, and locked value per token across all their wills | `owner` | `OwnerStats` |
| `get_wills_by_beneficiary` | Lists every will an address is named in | `beneficiary` | `Vec<Will>` |
| `get_will_history` | Reads a will's on-chain audit trail (capped at the newest `MAX_HISTORY_ENTRIES` transitions) | `will_id` | `Vec<WillStatusTransition>` |
| `get_will_history_page` | Reads a bounded, cursor-paged slice of a will's audit trail | `will_id`, `cursor`, `limit` | `Vec<WillStatusTransition>` |
| `guardian_trigger` | Casts a guardian vote; 2 of 3 forces an early release | `will_id`, `guardian` | — |

`checkin_period_days` and `grace_period_days` passed to `create_will` must each be at least `1` day (and at most `MAX_PERIOD_DAYS`); a value of `0` panics with `WillError::InvalidPeriod`.

## Contract Events

Every state-mutating entry point publishes exactly one event so that off-chain indexers and SDK consumers can reconstruct will history without re-simulating transactions. The topic is a tuple of `(symbol, will_id)` unless noted otherwise. Payload fields are listed in order.

| Entry point | Topic symbol | Payload |
|---|---|---|
| `create_will` | `"created"` | `(owner: Address, token_count: u32, beneficiaries: Vec<Beneficiary>, checkin_deadline: u64)` |
| `confirm_will` | `"confirmed"` | `owner: Address` |
| `check_in` | `"checkin"` | `(owner: Address, next_deadline: u64)` |
| `trigger_will` | `"triggered"` | `grace_period_ends: u64` |
| `emergency_checkin` | `"emerg"` | `(owner: Address, next_deadline: u64)` |
| `release_inheritance` | `"released"` | `(token_count: u32, beneficiaries_count: u32)` |
| `cancel_will` | `"cancelled"` | `(owner: Address, token_count: u32)` |
| `update_beneficiaries` | `"benefup"` | `(owner: Address, beneficiary_count: u32, beneficiaries: Vec<Beneficiary>)` |
| `update_guardians` | `"guardup"` | `(owner: Address, guardians: Vec<Guardian>)` |
| `update_will_settings` | `"setupd"` | `(owner: Address, update_fields: Vec<Symbol>)` — also emits `"guardup"` when guardians change |
| `close_will` | `"closed"` | `owner: Address` |
| `top_up` | `"topup"` | `(owner: Address, token: Address, amount: i128, new_balance: i128)` |
| `guardian_trigger` | `"gvote"` | `(guardian: Address, weight: u32, total_weight: u32)` |
| `accept_guardian_role` | `"gaccept"` | `guardian: Address` |
| `reject_guardian_role` | `"greject"` | `guardian: Address` |
| `guardian_cancel` (cancel vote) | `"gcvote"` | `(guardian: Address, weight: u32, total_weight: u32)` |
| `guardian_cancel` (quorum reached) | `"gcancel"` | `(guardian: Address, next_deadline: u64)` |
| `merge_wills` | `"merged"` | `(owner: Address, consumed_will_id: u64, new_balance: i128, beneficiaries: Vec<Beneficiary>)` — topic uses surviving will id |
| `migrate_will` | `"migrated"` | `(owner: Address, from_version: u32, to_version: u32)` |
| `clone_will` | `"cloned"` | `(source_id: u64, owner: Address)` — topic uses new will id |
| `batch_create_wills` | `"batch"` | `will_ids: Vec<u64>` — topic is `(symbol, owner)` instead of `(symbol, will_id)` |
| `archive_will` | `"archived"` | `owner: Address` |
| `update_will_settings` (periods) | `"periodu"` | `(owner: Address, new_checkin_period_days: u64, new_grace_period_days: u64, next_deadline: u64)` |
| `renounce_inheritance` | `"renounce"` | `(beneficiary: Address, owner: Address, beneficiaries: Vec<Beneficiary>)` |
| `keeper_bounty` | `"bounty"` | `(keeper: Address, amount: i128)` |
| `split_will` | `"split"` | `(new_id: u64, owner: Address, split_amount: i128)` — topic uses original will id |
| `reveal_and_claim` | `"hclaim"` | `(claimant: Address, amount: i128)` |
| `set_delegate` | `"delegset"` | `(owner: Address, delegate: Address)` |
| `clear_delegate` | `"delegclr"` | `owner: Address` |
| `batch_checkin` | `"batchchk"` | `(will_ids: Vec<u64>, count: u32)` — topic is `(symbol, owner)` instead of `(symbol, will_id)` |

The canonical source of truth for each event's exact topic and payload is [`contracts/will/src/events.rs`](./contracts/will/src/events.rs).

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

### What `archive_will` removes

`archive_will` is permissionless: once a will is `Released` or `Cancelled`, any
account may call it to reclaim storage. Beyond the will entry and the
owner/beneficiary/Triggered indexes, it also drops the will's on-chain
`WillHistory` entry and every `GuardianVote` / `GuardianCancelVote` entry
belonging to its guardians.

**History does not survive archival.** Those keys are only ever read to describe
a *live* will, so retaining them would strand ledger state — paid for out of the
protocol's rent — for entries no query can resolve. Consumers that need the
audit trail after a will is archived must use the **off-chain event log**,
which is append-only and never trimmed; the archived `Will` itself keeps the
final status, balances, and parties until Soroban's state archival collects it.
In particular, `get_will_history` returns an empty trail for an archived will
and must not be used as a post-archival recovery path. See
[issue #393](https://github.com/SoroWill/sorowill-contracts/issues/393).

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
| 6 | `GracePeriodExpired` | `emergency_checkin` (or `guardian_cancel_trigger`) was called after the grace period already elapsed. A `Triggered` will can no longer be returned to `Active` once the grace period is over. |
| 7 | `InvalidPercentages` | Beneficiary percentages did not sum to exactly 10,000 basis points. |
| 8 | `AlreadyVoted` | The guardian has already voted to trigger this will. |
| 9 | `NotGuardian` | The caller is not a designated guardian of this will. |
| 10 | `CheckinNotDue` | `trigger_will` was called before the check-in deadline passed. |
| 11 | `ZeroAmount` | An amount of zero (or less) was supplied where a positive amount is required. |
| 12 | `TooManyBeneficiaries` | The beneficiary list was empty or exceeded `MAX_BENEFICIARIES` (10), or the guardian/token list exceeded its cap. See [the FAQ](#faq-why-is-there-a-beneficiary-limit). |
| 12 | `TooManyBeneficiaries` | A list-length cap was exceeded: a `beneficiaries` list that is empty or longer than `MAX_BENEFICIARIES`, a `guardians` list longer than `MAX_GUARDIANS`, or a `batch_create_wills` spec list that is empty or longer than `BATCH_MAX`. Token-list bounds are **not** reported here — those raise `InvalidTokenCount`. |
| 13 | `WillNotSettled` | The requested action requires the will to be `Released` or `Cancelled`. |
| 14 | `WillNotBothActive` | Both wills in a merge must be `Active`. |
| 15 | `SameWillId` | The same will id was supplied for both sides of a merge. |
| 16 | `MergeWouldExceedLimits` | Merging would exceed the maximum beneficiaries or guardians. |
| 17 | `OwnerCannotBeGuardian` | The owner cannot designate themselves as a guardian of their own will. |
| 18 | `BeneficiaryNotFound` | A beneficiary is not found in the will's beneficiary list. |
| 19 | `KeeperBountyExceedsMax` | Keeper bounty basis points exceed the maximum allowed (100 bps / 1%). |
| 20 | `InvalidGuardianThreshold` | `guardian_threshold` is outside the range the guardian list can reach. Quorum is compared against accumulated guardian **weight**, so the range is `1..=guardians.len()` for unweighted lists and `1..=sum(weights)` for lists installed via `update_guardians_weighted`. Also raised when shrinking a non-empty guardian list would leave the stored threshold unreachable. |
| 21 | `FixedAmountExceedsBalance` | The sum of every `Allocation::FixedAmount` entry exceeds the will's **primary-token** balance. A `FixedAmount`-only will is allowed to leave headroom unaccounted for (reserved for a later `add_hashed_beneficiary`, otherwise refunded to the owner at release) — only an over-commitment is an error. |
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
| 35 | `InvalidPreimage` | `reveal_and_claim` was called with a 64-byte pre-image whose SHA-256 does not match any stored `HashedBeneficiary` commitment on the will. |
| 36 | `AlreadyClaimed` | `reveal_and_claim` was called for a hashed beneficiary slot that has already been claimed. |
| 37 | `TooManyWills` | An owner or beneficiary index list is already at `MAX_WILLS_PER_INDEX` and cannot accept another will id. |
| 38 | `GuardianNotConsented` | A guardian has not accepted their role and cannot vote. |
| 39 | `UnsupportedSchemaVersion` | A stored will matches neither the current `Will` layout nor any known legacy layout. |
| 39 | `PrimaryTokenMismatch` | Cannot merge: the two wills' primary tokens differ. |
| 40 | `DuplicateToken` | The same token address was supplied more than once in a `tokens` list. |
| 41 | `BatchTooLarge` | A `batch_check_in` call supplied more than `MAX_BATCH_CHECK_IN` (50) will ids. |
| 42 | `InvalidTokenCount` | The token list supplied to `create_will`, `clone_will`, `split_will`, or `batch_create_wills` was empty, or contained more than `MAX_TOKENS` entries. |
| 43 | `InvalidPreimageLength` | `reveal_and_claim` was called with a pre-image that is not exactly 32 bytes, so its SHA-256 could never match a stored commitment. |
| 44 | `InvalidCommitmentLength` | A hashed-beneficiary commitment was not exactly 32 bytes (a SHA-256 digest) and could never be matched by a pre-image. |
| 45 | `DuplicateCommitment` | The same commitment hash is already registered on the will, making the second slot unreachable. |
| 46 | `PreimageAddressMismatch` | A `reveal_and_claim` pre-image was not bound to the `claimant` address, so a third party could replay it and take the reserved share. |
| 47 | `MergeWithHashedBeneficiaries` | `merge_wills` was called while either will still had an unrevealed hashed beneficiary, whose committed percentage a merge cannot carry across. |
| 48 | `DuplicateWillId` | A `batch_check_in` `will_ids` list named the same will more than once. |
| 49 | `InvalidConsentTransition` | `accept_guardian_role` was called by a guardian who had already `Rejected` the role, which is terminal. |

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

## Documentation

All supplementary docs live under [`docs/`](./docs):

| Document | What it covers |
|---|---|
| [docs/FUZZING.md](./docs/FUZZING.md) | Fuzz-testing setup, invariants checked, how to run `cargo-fuzz` targets, and how to add a new target |
| [docs/RESOURCE_COSTS.md](./docs/RESOURCE_COSTS.md) | Per-entry-point resource profiles (CPU, ledger reads/writes, storage rent) and the storage layout trade-offs behind them |
| [docs/WASM_SIZE.md](./docs/WASM_SIZE.md) | Release build profile tuning for `.wasm` binary size (deploy cost) |
| [docs/SECURITY-REVIEW.md](./docs/SECURITY-REVIEW.md) | Internal security review findings, threat model, and resolved/open items |
| [docs/adr/0001-guardian-threshold.md](./docs/adr/0001-guardian-threshold.md) | ADR: rationale behind the 2-of-3 default guardian threshold, known limitations, and the proposed configurable M-of-N feature |
| [docs/adr/0002-legacy-token-balance-mirror.md](./docs/adr/0002-legacy-token-balance-mirror.md) | ADR: legacy token balance mirror design |
| [docs/adr/0002-total-locked-by-token-scalability.md](./docs/adr/0002-total-locked-by-token-scalability.md) | ADR: scalability trade-offs in the total-locked-by-token protocol stats entry |

## Security Policy

Security reports and responsible disclosure guidelines are documented in [`SECURITY.md`](./SECURITY.md). Please do not open public GitHub issues for security vulnerabilities.

## Contributing via Drips Wave

This repo participates in the **Stellar Wave Program** on [Drips](https://drips.network/wave). Maintainer-tagged issues carry Point values, and contributors who resolve them during an active Wave earn a proportional share of that Wave's reward pool. See [CONTRIBUTING.md](./CONTRIBUTING.md) for the contribution workflow, and <https://drips.network/wave> for how Wave itself works.
