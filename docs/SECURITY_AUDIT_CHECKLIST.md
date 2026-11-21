# Security Audit Checklist

This checklist is the working list for reviewing the SoroWill contract before
mainnet deployment and before any change that alters observable behaviour (see
the staleness note in [`SECURITY-REVIEW.md`](./SECURITY-REVIEW.md)). It tracks
the audit status, the risks already known, and the mitigation for each.

## Audit status

| Item | Status |
|---|---|
| Internal manual review | Completed, against contract 1.0.0. See [`SECURITY-REVIEW.md`](./SECURITY-REVIEW.md). Best-effort, not a substitute for an external audit. |
| Automated dependency audit | Runs in CI via [`.github/workflows/audit.yml`](../.github/workflows/audit.yml). Triaged advisories are listed in [`audit.toml`](../audit.toml). |
| Property and fuzz testing | Property tests use `proptest`. Fuzz targets are described in [`FUZZING.md`](./FUZZING.md). |
| Formal external audit | **Not yet completed.** No audit report has been published. |
| Audit reports | None published. When a report exists, add its link here with the date, auditor, and contract commit it covers. |

Do not describe the contract as audited until an external report is linked
above.

## Pre-release checklist

Work through each item for the release candidate. Record the commit hash
the review covered.

### 1. Authorization

- [ ] Every owner-only entry point calls `owner.require_auth()` and checks
      ownership (`load_owned` or equivalent). Confirm no path skips the check.
- [ ] Guardian entry points require the guardian to have accepted the role
      (`GuardianConsent::Accepted`) before any vote counts.
- [ ] Permissionless entry points (for example `trigger_will`,
      `release_inheritance`, `distribute`) are listed in
      [`SECURITY-REVIEW.md`](./SECURITY-REVIEW.md) section 2.3 and reviewed
      for what an arbitrary caller can cause.
- [ ] Signed invocations are bound to a nonce and expiration ledger (Soroban
      authorization entries).

### 2. Arithmetic and balances

- [ ] Every balance and share calculation uses checked or saturating
      arithmetic. Release-profile overflow behaviour is confirmed in
      `Cargo.toml`.
- [ ] Percentages total exactly 10,000 basis points where required, and
      fixed amounts never exceed the primary-token balance.
- [ ] Rounding dust is accounted for, so the sum of payouts equals the locked
      balance, with the remainder refunded or retained as designed.
- [ ] Multi-token balances stay consistent: `balances[token]` and the legacy
      `balance` mirror are updated together.

### 3. Reentrancy and token interaction

- [ ] State is written before any token transfer, or the transfer result is
      handled so that a failure cannot leave a half-applied state.
- [ ] Failed payouts are recorded (`FailedPayout`) and can be retried with
      `retry_failed_payout`.
- [ ] The contract assumes the token is a trusted SEP-41 contract. See the
      token trust risk below.

### 4. Guardian voting

- [ ] Each guardian counts at most once per cycle (tally is recomputed from
      per-guardian records, not a running counter).
- [ ] Votes from removed, rejected, or expired guardians do not count.
- [ ] The vote-expiry window in effect (`get_guardian_vote_expiry`) is the one
      the review expects. See the window override risk below.

### 5. Time and periods

- [ ] Check-in, trigger, and grace deadlines are computed from the same
      `last_checkin` / `trigger_time` anchors described in the README.
- [ ] Periods are bounded by `MAX_PERIOD_DAYS` and at least one day.
- [ ] Timestamp arithmetic cannot underflow when the ledger clock is unusual.

### 6. Storage and rent

- [ ] Persistent entries are extended with `extend_ttl` when written. Confirm
      every long-lived key is covered, including per-will settings.
- [ ] Unbounded collections (owner and beneficiary indexes, guardian lists,
      history) are capped or paginated. See the query exhaustion risk below.

### 7. Upgrades and migration

- [ ] Any change to a persisted type is checked against existing stored
      records. Schema bumps come with a real migration step in `migrate_will`.
- [ ] `CONTRACT_VERSION` and `CURRENT_SCHEMA_VERSION` are updated together when
      behaviour changes.

### 8. Events and observability

- [ ] Each state-changing entry point emits the event that off-chain consumers
      depend on. Event names are listed in [`EVENTS.md`](./EVENTS.md).

### 9. Dependencies and build

- [ ] `cargo audit` passes, or each ignored advisory in `audit.toml` has a
      written justification and a tracking issue.
- [ ] The wasm build stays `no_std` (the `fuzzing` and `testutils` features
      must not be enabled for a wasm build).
- [ ] `cargo clippy --all-targets -- -D warnings` and `cargo fmt --check` pass.

## Known risks and mitigations

Each risk lists what it is, why it is accepted or open, and the mitigation.
Items marked *Accepted* are design decisions recorded in
[`SECURITY-REVIEW.md`](./SECURITY-REVIEW.md) or [`SECURITY.md`](../SECURITY.md).

### R1. Token contract trust (Low, Accepted)

The contract trusts the token it holds to behave as a SEP-41 token. A malicious
or non-compliant token could misreport balances or fail transfers.

- **Mitigation:** Failed payouts are recorded and can be retried
  (`retry_failed_payout`). Owners choose the token at creation time.
- **Follow-up:** Consider validating the token at creation (optional, low
  priority; see the review's recommendations).

### R2. Guardian collusion (Low, By design)

A guardian quorum can release funds early, so colluding guardians can override
the owner.

- **Mitigation:** The default threshold is 2 of 3 (see
  [`docs/adr/0001-guardian-threshold.md`](./adr/0001-guardian-threshold.md)).
  Owners choose guardians, and each must accept the role. Guardians can be
  replaced with `update_guardians`, which clears stale votes.

### R3. Front-running of release and claims (Low, By design)

Release and reveal transactions are public, so they can be observed and
front-run.

- **Mitigation:** Payouts go to the address fixed in the will, so a front-runner
  cannot redirect them. `reveal_and_claim` binds the pre-image to the claimant
  (see [`SECURITY.md`](../SECURITY.md), replay section).

### R4. Duplicate beneficiary addresses (Low, Informational)

An address listed twice could receive two shares.

- **Mitigation:** `assert_valid_allocations` rejects duplicates with
  `DuplicateBeneficiary`.

### R5. Will ID enumeration (Low, Informational)

Will ids are sequential, so anyone can enumerate wills and their public
parameters.

- **Mitigation:** Sensitive beneficiary details can be hidden with hashed
  beneficiaries, which store only a commitment.

### R6. Query exhaustion (Low, Informational)

Queries over large owner or beneficiary sets could be expensive.

- **Mitigation:** Pagination on the list queries (`get_wills_by_owner`,
  `get_wills_by_beneficiary`, `get_owner_status_page`) and bounded inputs such
  as `MAX_BENEFICIARIES`.

### R7. Replay of hashed-beneficiary claims (Mitigated)

A revealed pre-image could be copied and replayed.

- **Mitigation:** One claim per slot (`AlreadyClaimed`), a pre-image bound to the
  claimant's address, and Soroban nonces and expiration ledgers. See
  [`SECURITY.md`](../SECURITY.md).

### R8. Guardian vote-expiry window override (Medium, new in #740)

Owners can set a per-will vote-expiry window with `set_guardian_vote_expiry`.
A very short window makes guardian votes lapse quickly, so a quorum is harder
to reach. A very long window is capped only at `MAX_PERIOD_DAYS`.

- **Mitigation:** Only the owner can change it, and only while the will is
  `Active`, so it cannot change during a trigger cycle. The value is bounded to
  `1..=MAX_PERIOD_DAYS`. The default is the grace period, so existing wills are
  unaffected.
- **Open point:** The override is stored under its own persistent key, with a
  TTL set when it is written. If that TTL lapses before the will ends, the
  window silently reverts to the grace period. The override is not refreshed
  when the will is saved. Before mainnet, either refresh the override TTL
  alongside the will's TTL or confirm the TTL covers the will's full lifetime.

### R9. Ledger timestamp skew (Low, Informational)

Votes and deadlines compare timestamps. The vote helpers use saturating
subtraction, so a clock that moves backwards cannot panic a transaction.

- **Mitigation:** `vote_is_live` saturates. Quorum helpers ignore records
  timestamped after the current time.

## Sign-off

| Role | Name | Date | Commit reviewed |
|---|---|---|---|
| Reviewer | | | |
| Maintainer | | | |
