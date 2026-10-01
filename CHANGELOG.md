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
- `archive_will` now has dedicated integration-test coverage confirming it
  publishes the `archived` event through the real entry point, not just a
  direct call to the events module (#494). The event itself (owner,
  timestamp, terminal-status reason) was already implemented.
- **CONTRACT_VERSION bumped to 1.3.0** (`1_003_000`) for the behavioral
  changes below.

### Fixed
- `reveal_and_claim` (pull-mode hashed-beneficiary claims) now pays every
  token the will holds proportionally, not just the primary token -- a
  multi-token will is no longer partially liquidated when a hashed
  beneficiary claims (#497).
- `reveal_and_claim`'s commitment check now uses a constant-time byte
  comparison (`constant_time_bytes_eq`) instead of `==`, which short-circuits
  on the first mismatching byte (#495).
- Fixed three instances of the same bad-merge corruption pattern (a missing
  closing brace immediately followed by a stale duplicate of
  already-superseded logic) found while implementing the above, all in
  `contracts/will/src/lib.rs`: `emergency_checkin` and `release_inheritance`
  each had a shadowed, non-callable local `grace_deadline` where the shared
  `grace_deadline()` helper should have been used directly (a hard compile
  error); `release_inheritance`'s was additionally nested so the *entire*
  function body only ran when the grace period had **not** yet expired, the
  opposite of its purpose. `distribute()` had a duplicate, pre-#378/#384
  keeper-bounty/fixed-amount computation left dangling inside an unclosed
  `if`, which (had it compiled) would have let a `FixedAmount` allocation be
  deducted from every token's balance instead of the primary token only, and
  paid the primary token's share twice. (#751)
- `WillError::UnsupportedSchemaVersion` and `WillError::PrimaryTokenMismatch`
  both declared discriminant `39`, colliding with `DistributionMismatch` -- a
  hard compile error (duplicate enum discriminant). Reassigned to `51` and
  `52`; the README error-code table is updated to match. (#751)
- `test.rs::test_get_contract_version`'s hardcoded `1_000_000` literal was
  already stale relative to `CONTRACT_VERSION` before this PR; updated to
  track the current value.
- Added regression tests confirming percentage-sum validation
  (`assert_valid_allocations`/`assert_valid_percentages`) is already
  order-agnostic for interleaved `Percentage`/`FixedAmount` beneficiaries;
  the reported bug was not reproducible against current code (#496).
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
### Added

- `get_owner_stats(owner)` entry point returning `OwnerStats` (total wills,
  non-terminal wills, locked value per token) so clients don't have to walk
  every `get_wills_by_owner` page to compute totals (#447).
- Storage schema versioning: `migration.rs` with a legacy-aware
  `decode_will` used by every will load, stepwise `upgrade` used by
  `migrate_will`, and a new `UnsupportedSchemaVersion` error (code 39) (#446).
- Diagnostic log messages stating the supplied count and `MAX_BENEFICIARIES`
  when a beneficiary list is rejected; README FAQ on the limit (#444).

### Changed

- `merge_wills` now explicitly rejects wills with different owners with
  `NotSameOwner` (code 24) before checking that the caller owns them (#445).
- `WillError::MergeWithHashedBeneficiaries` (code 47): `merge_wills` is now
  rejected while either will still carries a hashed beneficiary that has not
  revealed and claimed. `merge_beneficiaries` only merges *visible*
  beneficiaries, so the consumed will's commitments and their committed
  percentages were silently dropped while its balance moved to the survivor,
  and the survivor's existing percentages then applied to the larger combined
  balance. Claimed hashed entries are inert and do not block a merge (#380).
- `merge_wills` now records a `merge` transition for **both** wills. The
  consumed will moves `Active` → `Cancelled` and previously recorded nothing,
  so `get_will_history` showed only its creation; the survivor records an
  `Active` → `Active` entry describing the merge that rewrote its balance,
  beneficiaries, guardians and periods (#381).
- `WillError::DuplicateWillId` (code 45): `batch_check_in` now rejects a
  `will_ids` list that names the same will twice, instead of processing the
  repeat and emitting a redundant `check_in` event for it (#355). `README.md`
  and `spec/will-v0.1.0.json` are updated for the new code.
- `WillError::DuplicateToken` (code 40): `create_will` and `batch_create_wills`
  now reject a `tokens` list that names the same token address twice (#350).
- `get_will_history` now records the `confirm_will` (`PendingConfirmation` to
  `Active`) and `close_will` (`Released` to `Settled`) transitions (#352).
- Restored five `WillError` variants that entry points and tests already
  referenced but which were missing from the enum, so the crate compiles:
  `BatchTooLarge` (41), `InvalidTokenCount` (42), `InvalidPreimageLength` (43),
  `InvalidCommitmentLength` (44) and `DuplicateCommitment` (45).
- Guardian consent is now observable from events: `accept_guardian_role`
  publishes `"gaccept"` and `reject_guardian_role` publishes `"greject"`, each
  with the guardian address as the payload. Both entry points already mutated a
  guardian's consent field — the gate on whether they may vote in
  `guardian_trigger` — but published nothing, so an indexer rebuilding will
  state from events alone could not tell when a guardian became eligible to vote
  or was rejected (#386).

### Changed

- `merge_wills` now combines the two guardian lists by **address** instead of
  comparing whole `Guardian` structs. The same address recorded with a
  different weight or consent on each will compared unequal and was appended
  twice, breaking the no-duplicate-guardian rule `assert_valid_guardians`
  enforces on every creation path and double-counting that guardian's weight
  toward quorum. A shared address now yields one entry whose weight is the
  greater of the two — keeping the surviving will's validated
  `guardian_threshold` reachable while counting the weight exactly once — and
  whose consent is the more advanced of the two (`Accepted` > `Pending` >
  `Rejected`) (#379).
- The keeper bounty in `distribute` is now computed from and paid out of the
  **same** token, chosen up front as the first entry of `will.balances` whose
  bounty share rounds above zero. Previously the amount was computed against
  whichever token first produced a non-zero share while the payout used the
  first entry of `transfer_plan`; on a multi-token will whose first token's
  share rounded to zero, the keeper was paid with the wrong token's client out
  of a balance that had never been reduced — taking the funds from other
  beneficiaries, or aborting the release. Rounding: the bounty is
  `floor(balance * keeper_bounty_bps / 10_000)`, so a token too small to round
  above zero is skipped, and if no token rounds above zero no bounty is paid
  and no beneficiary share is reduced (#378).
- Corrected the `WillError` docs for `FixedAmountExceedsBalance`,
  `InvalidGuardianThreshold` and `TooManyBeneficiaries` so each states exactly
  when it is raised. The wording is generated into the SDK and client error
  references, which integrators rely on to interpret error codes (#389).
- Corrected the `Will` field docs for `beneficiaries`, `hashed_beneficiaries`,
  `guardian_threshold` and `guardian_vote_weight` in `contracts/will/src/types.rs`.
  `beneficiaries` claimed shares "always sum to 10,000" (false once
  `Allocation::FixedAmount` entries are allowed), `hashed_beneficiaries` used a
  "100-sum" against the rest of the contract's 10,000 basis points, and
  `guardian_threshold` was described as a count of distinct votes rather than a
  comparison against accumulated weight (#388).
- Rewrote the `WillStatus` lifecycle diagram. The old one showed a
  `partial_release` transition back to `Active` that no entry point implements,
  and omitted `PendingConfirmation` — the state every will created with a
  confirmation delay starts in. The new diagram shows `PendingConfirmation` with
  its `confirm_will` and `cancel_will` transitions, and every arrow maps to a
  real entry point (#387).

### Fixed

- `top_up` now enforces the same `MAX_TOKENS` distinct-token cap `create_will`
  applies, raising `InvalidTokenCount` when the token is not already in the
  will's `balances` map and the map already holds `MAX_TOKENS` entries.
  `distribute` and `cancel_will` iterate every entry of that map, so an owner
  could previously grow it one token at a time until a release or refund no
  longer fit in a Soroban transaction, leaving the will's funds unreachable.
  Topping up a token the will already holds is unchanged and still works at the
  cap (#358).
- `get_guardian_vote_status` no longer panics when the current ledger time is
  earlier than a stored vote timestamp. It computed `now - record.timestamp`
  with a raw `u64` subtraction, so a skewed clock (or a test that rewinds the
  ledger) underflowed and trapped the read-only query. The expiry rule is now
  one shared `storage::vote_is_live` helper used by this query and by
  `has_guardian_voted` / `has_guardian_cancel_voted`, so the three call sites
  cannot diverge again; a record timestamped after `now` is reported as-is by
  the query and as "not voted" by the quorum counters (#359).
- `get_wills_by_owner_and_status` now returns an empty page for `limit == 0`,
  matching `get_wills_by_owner` and `get_wills_by_beneficiary`. It pushed a
  matching will *before* comparing the page length against the page size, so a
  caller paginating with a computed limit that reached zero received one will
  while the id-based queries returned none (#360).
- `renounce_beneficiary` now rejects the last remaining beneficiary with
  `TooManyBeneficiaries` instead of leaving the will with an empty beneficiary
  list. `create_will` and `update_beneficiaries` both require between 1 and
  `MAX_BENEFICIARIES` entries, and `assert_valid_allocations` cannot catch the
  emptied list because a `Vec` with no entries passes every check it makes, so
  a sole beneficiary could renounce and leave an `Active` or `Triggered` will
  with nobody for `release_inheritance` to pay. The rationale recorded for
  `InvalidPercentages` in the function's rustdoc has been corrected (#361).
- `update_guardians_weighted` now accumulates guardian weights with
  `checked_add` and enforces a new public `MAX_GUARDIAN_WEIGHT` (1_000_000) cap
  per guardian. A weight list whose `u32` total overflows used to abort the call
  with an opaque arithmetic trap (the release profile keeps `overflow-checks` on)
  rather than a typed `WillError`; it now returns
  `InvalidGuardianThreshold` (#356).
- `update_periods` now emits a `periodu` `next_deadline` of
  `last_checkin + checkin_period_days` — the deadline `trigger_will` actually
  enforces — instead of `now + checkin_period_days`. Off-chain consumers that
  display or schedule reminders from the event are no longer told a deadline
  that is later than the true one by the age of the current check-in (#357).
- `release_inheritance` now requires `now` to be *strictly greater* than the
  grace deadline, so the deadline second belongs to the owner's
  `emergency_checkin` and exactly one of the two entry points succeeds at any
  timestamp (#354). Previously both were valid at `now == deadline`, so the
  outcome depended on which transaction the ledger ordered first. The rustdoc
  for both functions, for `guardian_cancel_trigger` and for
  `get_time_until_deadline` now states which side owns the boundary second.
- Restored five `WillError` variants (`InvalidTokenCount`, `InvalidPreimageLength`,
  `InvalidCommitmentLength`, `DuplicateCommitment`, `BatchTooLarge`) and the
  `total_balance` helper that a bad merge had dropped, which left `main` failing
  to compile. The variants are re-numbered 41-45 so the already-published
  `DuplicateToken` (40) keeps its code; `README.md` and `spec/will-v0.1.0.json`
  are updated to match.
- `create_will` / `cancel_will` now record the will's real status in the audit
  trail instead of a hardcoded `Active`, so `get_will_history` is accurate for
  wills created with a confirmation delay and cancelled while pending (#351).
- `cancel_will` now decrements `ProtocolStats.total_locked_by_token` for every
  token the will held, not just the primary token, so `get_protocol_stats` no
  longer overstates locked value after a multi-token cancellation (#353).
- `create_will` derives the legacy `token`/`balance` mirror from the
  accumulated `balances` map, so the two can no longer disagree (#350).

### Removed

- Removed unused `InvalidPercentage` (code 22) error variant from `WillError`.

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
