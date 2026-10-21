# Issue #580: "Missing validation in function scenario_64"

The issue names `function_64` / `scenario_64` in `contracts/will/src/lib.rs`.
No function with that name exists in the contract. A search of the whole
repository (`contracts/`, `fuzz/`, `docs/`, `spec/`, `scripts/`, the README
and the CHANGELOG) and of the history of `main` found no occurrence of
`scenario_64` or `function_64`, and none of the entry points takes a
"parameter 64". The title and description are generic and do not identify a
concrete entry point or input.

Every real entry point that accepts caller-supplied bounds already validates
them and returns a typed `WillError` (for example `ZeroAmount`,
`InvalidAllocation`, `InvalidTokenCount`, `TooManyBeneficiaries`,
`BatchTooLarge`, `InvalidGuardianThreshold`). Boundary behavior is covered by
the existing regression tests in `contracts/will/src/`.

No contract change is made for this issue. If a specific entry point and input
are identified, it can be closed or re-filed with the actual function name.
