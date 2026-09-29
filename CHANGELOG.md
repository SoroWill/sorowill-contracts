# Changelog

All notable changes to the `will` contract are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project uses the `version` field in
[`contracts/will/Cargo.toml`](./contracts/will/Cargo.toml) as its version
identifier. Each entry below the "in Cargo.toml (change grew here)" line
gets its own [contract spec artifact](./spec) once exported.

## [Unreleased]

### Fixed

- `confirm_will` now re-validates the beneficiary list before moving a will
  from `PendingConfirmation` to `Active`. It rechecks the count, duplicates,
  basis-point totals and fixed amounts against the current balances, so a
  list that became invalid during the confirmation delay cannot be activated
  (#448).
- `guardian_trigger` and `guardian_cancel_trigger` now recount live votes from
  the current guardian list on every vote instead of adding to a running
  total. Votes from removed, rejected or expired guardians no longer count
  toward the threshold, and the tally can never exceed the number of guardians
  (#453). `update_will_settings` now also clears cancel votes when it replaces
  the guardian list, as `update_guardians` already did.
- `reveal_and_claim` now requires the pre-image to start with the claimant's
  XDR-encoded address, followed by a non-empty salt. A leaked or front-run
  pre-image can no longer be used to claim a hashed beneficiary's share to a
  different address. `add_hashed_beneficiary` rejects commitments that are not
  32-byte SHA-256 digests (#452).

  **Breaking:** commitments must now be computed as
  `sha256(beneficiary.to_xdr() || salt)`.

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
