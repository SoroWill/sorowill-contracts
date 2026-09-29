# Changelog

All notable changes to the `will` contract are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project uses the `version` field in
[`contracts/will/Cargo.toml`](./contracts/will/Cargo.toml) as its version
identifier. Each entry below the "in Cargo.toml (change grew here)" line
gets its own [contract spec artifact](./spec) once exported.

## [Unreleased]

### Added
- `get_wills_by_owner_and_status_page`, returning a `WillPage` with the page's
  wills, the `total_count` of all matches and an explicit `next_cursor` (#455).
- `WillError::DistributionMismatch` (39): release aborts if planned payouts for
  a token do not sum to exactly its balance (#456).

### Fixed
- Guardian release/cancel votes are tallied from per-guardian records, so a
  guardian re-voting after expiry is no longer counted twice (#458).
- The keeper bounty is deducted from the will's balance instead of being paid
  on top of a full distribution out of other wills' funds (#458).
- `reveal_and_claim` now reduces the will's token balance map, so a later
  `release_inheritance` cannot pay the claimed share a second time (#458).
- `distribute` refuses wills that are not `Active`/`Triggered` (#458).
- Grace period is anchored to the missed check-in deadline instead of the time
  `trigger_will` was called; all grace checks share one `grace_deadline`
  helper (#457).
- Fixed-amount-only wills pay any leftover balance to the final beneficiary
  instead of stranding it in the contract (#456).
- `get_wills_by_owner_and_status` filters by status before cutting the page, so
  short pages no longer hide matching wills (#455).

## [0.1.0] - Initial shipped behavior

Seeded entry summarizing the contract's behavior as of this changelog's
introduction. See the [README's Contract Functions table](./README.md#contract-functions)
and [`spec/will-v0.1.0.json`](./spec/will-v0.1.0.json) for the authoritative,
up-to-date interface.

### Added

- Core will lifecycle: `create_will`, `check_in`, `trigger_will`,
  `emergency_checkin`, `release_inheritance`, `cancel_will`, `close_will`.
- Beneficiary management: `update_beneficiaries`, `renounce_beneficiary`,
  basis-point-based percentage splits (must sum to 10,000).
- Guardian override: up to 3 named guardians, weighted quorum voting via
  `guardian_trigger`, guardian list management via `update_guardians`, and
  a cooldown period after guardian-list changes before a vote can force a
  release.
- Multi-token support: a will can hold balances across multiple SEP-41
  tokens (or native XLM) simultaneously via `top_up`.
- Batch and convenience operations: `batch_check_in`, `batch_create_wills`,
  `clone_will`, `merge_wills`, `set_delegate` (delegated check-in),
  `migrate_will`, `archive_will`.
- Query surface: `get_will`, `get_wills_by_owner`,
  `get_wills_by_owner_and_status`, `get_wills_by_beneficiary`,
  `get_triggered_wills`, `get_protocol_stats`, `get_will_history`,
  `get_contract_version`.
- On-chain audit trail via `WillStatusTransition` records, retrievable
  through `get_will_history`.
- `WillError` numeric error codes for every failure mode (see the
  [README's error code reference](./README.md#error-codes)).
- Resource-cost profiling suite (`docs/RESOURCE_COSTS.md`) and a
  coverage-guided + property-based fuzzing suite (`docs/FUZZING.md`)
  covering `create_will` and `update_beneficiaries`.

[Unreleased]: https://github.com/SoroWill/sorowill-contracts/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/SoroWill/sorowill-contracts/releases/tag/v0.1.0
