# `distribute` resource budget

`distribute` (reached from `release_inheritance` and from the guardian-quorum
branch of `guardian_trigger`) pays out every token a will holds. All of it runs
in one Soroban transaction, so the work must stay inside the network's CPU,
memory and ledger-IO limits.

## Work per payout

Each (beneficiary, token) pair with a non-zero share costs:

- one cross-contract `try_transfer` call into the token contract, which itself
  reads and writes two persistent balance entries (sender and recipient);
- on failure, one extra persistent write (`set_failed_payout`) and one event.

Per token there is also one balance-map entry read, plus one optional refund
transfer to the owner and one optional keeper-bounty transfer.

Per release there is a fixed cost: the will entry write, index removals (owner
and one per beneficiary), the protocol-stats update and the release events.

## Bound

A release performs at most

    MAX_BENEFICIARIES * MAX_TOKENS + MAX_TOKENS + 1  =  10 * 10 + 10 + 1  =  111

token calls. These are estimates derived from the call structure, not measured
instruction counts; re-measure with the Soroban cost estimator before raising
either cap.

## Pre-flight check

`distribute_limits::assert_payout_budget` rejects a release whose
beneficiaries x tokens product exceeds `MAX_RELEASE_PAYOUTS` (100) with
`WillError::DistributionTooLarge` before any state is changed. The list caps
are enforced when a will is created or edited; this check also protects the
guardian-quorum path and wills written under an older layout.

## Batching token transfers (#735)

Question: can `distribute` replace its N per-beneficiary transfers with fewer,
larger calls?

Findings:

- **No batch primitive in the token interface.** SEP-41 exposes a single
  `transfer(from, to, amount)` per call and no multi-recipient form. Any
  batching would depend on a non-standard token function, which this contract
  cannot assume for an arbitrary token address.
- **Batching would undo failure isolation.** Each payout is a `try_transfer`
  so that one refusing recipient is recorded via `set_failed_payout` instead
  of rolling back the whole release (#459). Folding transfers into one call
  makes them succeed or fail together, which reintroduces the permanent
  blockage that #459 removed.
- **Zero-amount transfers are already skipped.** `distribute` only calls
  `pay_or_record_failure` for `share > 0`, so the N in the cost above counts
  only real payouts.
- **Measured savings: none.** No batching is implemented, so no gas numbers
  are claimed. The per-transfer cost is already bounded by
  `MAX_RELEASE_PAYOUTS`.

Decision: keep one `try_transfer` per (beneficiary, token) pair. Revisit only if
the token standard gains a batch transfer and the failure-isolation requirement
is changed deliberately.
