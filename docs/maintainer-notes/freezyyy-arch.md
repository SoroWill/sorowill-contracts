# Maintainer notes (Freezyyy-arch)

## #411 reveal_and_claim uses will.balance
Already fixed on `main`: `reveal_and_claim` in `contracts/will/src/lib.rs`
iterates `will.balances` (all locked tokens) and pays the hashed beneficiary's
share of every token, then re-syncs the `will.balance` mirror. No change made.

## #410 record_transition action identifiers
Already fixed on `main`: every `record_transition` call in
`contracts/will/src/lib.rs` (create, trigger, emergency_checkin, release,
cancel, guardian paths) passes `symbol_short!(..)`, and the `action`
parameter is typed `soroban_sdk::Symbol`, so the compiler enforces it.
No change made.
