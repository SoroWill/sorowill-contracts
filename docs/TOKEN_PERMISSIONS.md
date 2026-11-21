# Token permission requirements

This document covers what an owner must have in place before the contract can
take their tokens (#713).

## How the contract pulls funds

Every owner-funded entry point (`create_will`, `batch_create_wills`,
`clone_will`, `top_up`) moves tokens with:

```
token.transfer(owner, contract_address, amount)
```

This call is authorized by the owner's own signature (`owner.require_auth()`),
which the entry point already requires. **No SEP-41 allowance (`approve`) is
needed** for these entry points. The contract never calls `transfer_from`.

## What is validated before a transfer

Before each owner-funded transfer the contract checks that the owner's balance
of that token is at least the amount being pulled. If it is not, the call
fails with:

- Error code `33`, `WillError::InsufficientBalance`.

Nothing is written. Because Soroban discards all storage writes from a failed
invocation, a multi-token `create_will` is all-or-nothing.

## Requirements checklist

| Requirement | Who | Checked by the contract |
| --- | --- | --- |
| Signs the transaction (`require_auth`) | Owner | Yes, by the auth framework |
| Holds at least `amount` of each token | Owner | Yes, before each transfer (`InsufficientBalance`) |
| Token implements SEP-41 | Token contract | Yes, `decimals()` probe in `create_will` (`InvalidToken`) |
| Amount is positive | Caller | Yes (`ZeroAmount`) |

Payouts (`release_inheritance`, `retry_failed_payout`, `reveal_and_claim`) move
tokens **out of** the contract and need no owner allowance.

## Not checked

The balance check does not prevent a token-level freeze, clawback, or
authorization rule from blocking the transfer. Those still fail inside the token
contract.
