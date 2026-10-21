# Event atomicity in `guardian_trigger` / `distribute` (#594)

Events published by an entry point are part of the same Soroban invocation as
its state changes. If the invocation fails (any panic or error), the host
discards both the ledger writes and the events it emitted, so a consumer can
never observe `gvote` or the release audit entry for a release that did not
complete.

Within `guardian_trigger`, when the vote reaches quorum:

1. the vote and tally are persisted, then `gvote` is published;
2. the `Active -> Released` transition is recorded;
3. `distribute` commits balances and status *before* any transfer
   (checks-effects-interactions), runs the transfers, and publishes
   `inheritance_released` last, after every payout was attempted.

A beneficiary transfer that fails does not abort the release: it is recorded
via `payout_failed` and can be retried with `retry_failed_payout` (#459).
`inheritance_released` therefore always means "the release is final", and
`payout_failed` events identify any payouts still outstanding.
