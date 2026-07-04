# SoroWill Contracts

**Trustless on-chain inheritance on Stellar Soroban**

[![Rust](https://img.shields.io/badge/Rust-1.84%2B-orange?logo=rust)](https://www.rust-lang.org/)
[![Soroban SDK](https://img.shields.io/badge/Soroban%20SDK-22.0.0-7D00FF)](https://developers.stellar.org/docs/build/smart-contracts)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](./LICENSE)
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
8. **Guardian override.** A will can name up to 3 guardians. Any 2 of them calling `guardian_trigger` force an immediate release — useful if the owner is known to be incapacitated rather than simply inactive.

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
cargo clippy --all-targets
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
| `get_wills_by_owner` | Lists every will owned by an address | `owner` | `Vec<Will>` |
| `get_wills_by_beneficiary` | Lists every will an address is named in | `beneficiary` | `Vec<Will>` |
| `guardian_trigger` | Casts a guardian vote; 2 of 3 forces an early release | `will_id`, `guardian` | — |

## Testnet Deployment

The deployed contract ID for Stellar Testnet is recorded in [`deployments/testnet.json`](./deployments/testnet.json), updated manually whenever a new version is deployed:

```json
{
  "WillContract": "<contract-id>",
  "network": "testnet",
  "deployedAt": "<ISO-8601 timestamp>"
}
```

## Contributing via Drips Wave

This repo participates in the **Stellar Wave Program** on [Drips](https://drips.network/wave). Maintainer-tagged issues carry Point values, and contributors who resolve them during an active Wave earn a proportional share of that Wave's reward pool. See [CONTRIBUTING.md](./CONTRIBUTING.md) for the contribution workflow, and <https://drips.network/wave> for how Wave itself works.
