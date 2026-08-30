# ADR 0002: Legacy Single-Token `token`/`balance` Mirror Alongside `balances`

## Status

Accepted (current implementation), with known desync risk documented below.
No migration is scheduled; this ADR records the tradeoff so future
contributors can tell "intentional legacy shim" from "bug."

## Context

`Will` stores locked funds two ways at once:

- `balances: Map<Address, i128>` — the authoritative multi-token source of
  truth, populated from the `tokens: Vec<(Address, i128)>` list passed to
  `create_will`.
- `token: Address` and `balance: i128` — a single-token mirror that always
  reflects the **first** entry of `tokens` (see `create_will`, where
  `let (primary_token, primary_balance) = tokens.get_unchecked(0);`).

Multi-token support (`balances`) was added after the contract's original
single-token design (`token`/`balance`). Rather than migrating every entry
point to the map in one pass, `token`/`balance` were kept as a "primary
token" mirror so that single-token helpers didn't need to be rewritten
immediately.

## Why keep the mirror instead of migrating everything at once

- **Several entry points were never generalized to multi-token and still
  read only the mirror**: `cancel_will` refunds via `will.balance` /
  `will.token` (see the `storage::adjust_locked_value(&env, &will.token,
  -refund)` call), and `reveal_and_claim` computes a claimant's share as
  `will.balance * percentage / 100` and transfers via `will.token` alone,
  never touching `balances`. Removing the mirror without first rewriting
  these to iterate `balances` would silently break every will with more than
  one locked token at these entry points.
- **`merge_wills` and `split_will` also key off the mirror** for their
  headline balance math (`combined_balance = will_a.balance +
  will_b.balance` in `merge_wills`), while separately combining the
  `balances` map. Both need to stay populated for these entry points to
  produce a will whose `balance` field means anything at all.
- **The alternative — migrating `cancel_will`, `reveal_and_claim`,
  `merge_wills`, and `split_will` to be fully multi-token-aware in the same
  change that introduced `balances`** — was judged too large and risky to
  land atomically; the mirror let multi-token support ship for `create_will`
  and `top_up` first, with the remaining entry points migrated
  incrementally (or left on the mirror, for single-token wills, which are
  still the common case).

## Known consequences (the desync risk)

Because `token`/`balance` are a snapshot of *only the first token*, not a
derived total, they can and do drift from `balances` in ways that are easy
to misread as "the will's balance":

1. **`reveal_and_claim` never updates `balances`.** It debits `will.balance`
   only. For a will with more than one locked token, `balances` still shows
   the pre-claim amount for the primary token after a claim — `will.balance`
   and `balances.get(will.token)` disagree.
2. **`merge_wills`' `combined_balance` is not "total value across all
   tokens."** It is `will_a.balance + will_b.balance`, i.e. primary-token
   amounts only, summed. For wills holding multiple tokens, this sum has no
   coherent meaning as a single `i128` — it is not the total locked value,
   just an arithmetic combination of two single-token snapshots.
3. **Any future entry point that reads `will.balance` as "the will's total
   locked value"** (a natural-sounding but incorrect assumption for a
   multi-token will) will silently under- or over-count. `get_will` returns
   both fields, so nothing in the type system flags this.

These are latent risks rather than exploitable bugs in the currently shipped
entry points (each function has consistently used the same field for both
its read and write side so far), but they are exactly the kind of "quirk vs.
oversight" ambiguity this ADR exists to resolve: **the mirror is only
guaranteed accurate for the primary token; any code path that needs a
will's true total locked value, across every token, must sum `balances`,
never read `will.balance`.**

## Migration plan

No migration is scheduled. If/when `cancel_will`, `reveal_and_claim`,
`merge_wills`, and `split_will` are generalized to operate over every entry
in `balances` rather than the primary-token mirror, `token`/`balance` can be
deprecated and eventually removed from `Will` in a version bump that updates
`spec/` and the SDK. Until then:

- Treat `token`/`balance` as **read-only, primary-token-only** convenience
  fields for single-token wills and off-chain display purposes.
- Any new entry point that needs a will's true total value must sum
  `balances`, not read `will.balance`.
