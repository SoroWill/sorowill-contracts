# Contract Troubleshooting Guide

This guide covers common issues when interacting with the will contract, how to diagnose them, and where to get help.

## Common Errors

The contract returns structured errors. Most failures surface as a numeric error code from the contract, or as a client-side validation error before the transaction is submitted.

| Code | Name | Meaning | Typical Cause |
|------|------|---------|---------------|
| 1 | `NotInitialized` | Contract has not been initialized | Calling methods before `initialize` |
| 2 | `AlreadyInitialized` | Contract was already initialized | Calling `initialize` twice |
| 3 | `Unauthorized` | Caller is not permitted | Wrong signer / missing auth |
| 4 | `NotFound` | Requested will or record does not exist | Wrong ID, or will was never created |
| 5 | `InvalidState` | Operation not allowed in current state | e.g. claiming a will that is not claimable |
| 6 | `InvalidAmount` | Amount is zero or out of range | Passing `0` or a negative/overflowing value |
| 7 | `NotYetClaimable` | Check-in window has not elapsed | Claiming before the grace period ends |
| 8 | `AlreadyClaimed` | Will has already been claimed | Duplicate claim attempt |
| 9 | `InsufficientBalance` | Not enough funds for the operation | Balance lower than the requested amount |

> Error codes are stable identifiers. Match on the code, not on human-readable messages, which may change.

## Troubleshooting Steps

### 1. Transaction fails with `Unauthorized`

1. Confirm the wallet signing the transaction is the expected owner or beneficiary.
2. Verify the correct network (testnet vs. mainnet) is selected in your client.
3. Re-check that the signer address matches the address stored on the will.
4. If using a multisig or delegated signer, ensure all required signatures are collected.

### 2. `NotFound` when reading a will

1. Confirm the will ID is correct and was returned by `create_will`.
2. Verify you are querying the same contract instance you created the will on.
3. Allow a moment for the transaction to be confirmed before reading.

### 3. Cannot claim a will

1. Check the will state — it must be in a claimable state.
2. Confirm the check-in grace period has fully elapsed (`NotYetClaimable`).
3. Ensure the will has not already been claimed (`AlreadyClaimed`).
4. Verify the claiming account is the designated beneficiary.

### 4. `InvalidAmount` on create or deposit

1. Ensure the amount is greater than zero.
2. Confirm the amount does not exceed the maximum supported value.
3. Check that the token/asset matches what the contract expects.

### 5. `InsufficientBalance`

1. Check the account balance covers the amount plus fees.
2. Confirm funds are on the correct network and asset.
3. Reduce the amount or top up the account and retry.

### 6. `NotInitialized` / `AlreadyInitialized`

1. `NotInitialized`: call `initialize` before any other method.
2. `AlreadyInitialized`: the contract is ready — remove the redundant `initialize` call.

## FAQ

**Q: Why do I get a different error than the one in this table?**
A: Newer contract versions may add codes. Check the contract's error enum in the source and the release notes for your deployed version.

**Q: My transaction succeeded but the will does not appear.**
A: Reads may lag behind confirmation. Wait for finality, then re-query using the will ID returned at creation.

**Q: Can I undo a claim?**
A: No. Claims are final. Verify the beneficiary and state before claiming.

**Q: I lost access to the owner key — can I still check in?**
A: Check-ins require the owner signer. Without it, the will follows its configured claim path once the grace period elapses.

**Q: How do I know the grace period has elapsed?**
A: Compare the current ledger time against the will's last check-in plus the configured grace period. Claiming early returns `NotYetClaimable`.

**Q: Are error codes the same across networks?**
A: Yes for a given contract version. Confirm both deployments run the same version.

## Support

If this guide does not resolve your issue:

- Open an issue in the repository with the error code, the method called, the network, and relevant transaction hashes (never include private keys or secrets).
- Include the contract version and client/SDK version you are using.
- For security-sensitive reports, follow the process in `SECURITY.md` instead of filing a public issue.
