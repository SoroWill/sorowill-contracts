# Beneficiary percentage precision (#729)

This document sets out how beneficiary percentages are represented, rounded,
and paid out. The rules are enforced in `contracts/will/src/lib.rs`
(`proportional_share`, `distribute`, `assert_valid_percentages`).

## Representation

- A percentage is stored as **basis points** (`u32`). 10,000 bp = 100%.
- Beneficiary percentages must sum to **exactly 10,000 bp**. Any other total
  fails with `InvalidPercentages`. No rounding happens at input time, so there
  is no precision loss when the split is set.
- Resolution is 0.01%. A split cannot express a finer share than 1 bp.

## Rounding rule

For a balance `total` and a share of `bp` basis points:

```
share = floor(total * bp / 10_000)
```

`proportional_share` computes this without forming `total * bp`, which could
overflow. It splits `total` into whole ten-thousands and a remainder, so the
result is exact floor division.

Each non-final percentage beneficiary gets its floored share. The share is
never rounded below one unit while the balance can still cover every later
beneficiary (`min_one_unit_share`, #599).

## Remainder handling

Floor rounding leaves a remainder of less than one unit per beneficiary. The
**last percentage beneficiary** receives everything left after the earlier
shares, so the distributed total equals the balance exactly. No dust is left in
contract storage.

Remainder is handled **per token**. Each token's balance is split and its
remainder is absorbed on its own.

## Ordering of payouts within one release

1. Keeper bounty: `floor(balance * keeper_bounty_bps / 10_000)` from one token.
2. Hashed-beneficiary reserve: `floor(available * unclaimed_bps / 10_000)`.
3. `FixedAmount` beneficiaries, paid in the primary token only, capped at what
   remains.
4. `Percentage` beneficiaries split what remains by basis point. The last one
   absorbs the remainder.
5. Any balance no beneficiary can receive is refunded to the owner (#383).

## Worked examples

| Balance | Shares (bp) | Floored shares | Last beneficiary gets | Total |
| --- | --- | --- | --- | --- |
| 10,007 | 3,333 / 3,333 / 3,334 | 3,335 / 3,335 / 3,336 | 3,337 | 10,007 |
| 9 | ten beneficiaries at 1,000 bp each | 0 each (9 × 1,000 / 10,000 = 0.9) | 9 (the whole remainder) | 9 |

A balance too small for each share to reach one unit gives most recipients zero.
Creation does not enforce a minimum amount (see the note in `distribute`).

## Tests

`contracts/will/src/issue_729_test.rs` checks the rounding arithmetic directly,
including remainder cases and a sweep showing the dust left after flooring is
always less than the number of beneficiaries.
