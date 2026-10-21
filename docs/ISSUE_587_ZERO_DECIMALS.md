# Issue #587: tokens with zero decimals

The contract never reads a token's decimals to compute amounts. `create_will`
calls `decimals()` only as a read-only probe to reject addresses that are not
SEP-41 tokens (`WillError::InvalidToken`); the returned value is discarded, and
`0` is accepted like any other value.

All balances, beneficiary shares and keeper bounties are integers in the
token's base unit, computed with `proportional_share` (basis points over
`i128`, rounding down) and a remainder rule in `distribute`. None of that
depends on decimal places, so a zero-decimal token pays out exactly like any
other: the shares still sum to the balance and a bounty that rounds to zero is
skipped rather than failing.

The contract also does no display formatting; amounts are returned as raw
`i128` base units and formatting is left to clients, which should treat
`decimals() == 0` as "no fractional part".

No contract change is made for this issue.
