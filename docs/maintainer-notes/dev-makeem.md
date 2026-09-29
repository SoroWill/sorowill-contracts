# Maintainer notes (Dev-makeem)

## #412 get_will_history unbounded response

Not duplicated here. Open PR #407 ("Bound WillHistory and add a paged history read") bounds the stored history in storage.rs and adds a paged read in lib.rs, with tests in get_will_history_test.rs and documentation of the maximum response size. Main still returns the raw vector until #407 merges.

## #413 distribute and the balances snapshot

No code change. `distribute` takes no snapshot parameter: it pays out from `will.balances`, the on-chain map stored in the will. `top_up` requires `WillStatus::Active`, so balances cannot change after `trigger_will` moves the will out of Active, and the payout cannot include tokens added after the trigger. A caller cannot supply a modified snapshot, so there is nothing to compare or hash.

## #415 update_guardians threshold reachability

No code change. `update_guardians` already rejects `guardian_threshold > new_guardians.len()` with `WillError::InvalidGuardianThreshold` for any non-empty list, and `update_guardians_threshold_test.rs` covers the rejection. `update_guardians_weighted` validates against total weight.

