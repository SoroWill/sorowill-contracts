# Maintainer notes (Valreb001)

## #421: hashed beneficiary commitments in events

Already addressed on main. `add_hashed_beneficiary` emits only the on-chain
`commitment`, which is `SHA-256(address_bytes || salt)` over a 64-byte
pre-image (32 bytes of address plus a 32-byte random salt chosen by the
beneficiary; see `reveal_and_claim`). No user-chosen password is hashed, so the
public event cannot be used for an offline dictionary attack.

## #423: merge_wills duplicate beneficiaries

Already addressed on main. `merge_wills` builds the combined list through
`merge_beneficiaries`, which sums the shares of an address that appears in both
wills (and adds `FixedAmount` values) before recalculating percentages, so a
shared beneficiary is listed once and not paid twice.
