#![cfg(test)]

//! Regression coverage for issue #501: `CONTRACT_VERSION` is defined in
//! source but nothing actually validated it against anything else.
//!
//! `CONTRACT_VERSION` being correct in isolation is not the guarantee an SDK
//! actually needs -- three independent representations of "the contract's
//! version" exist in this repo, each a literal a human can forget to update:
//! - `CONTRACT_VERSION` itself (`contracts/will/src/lib.rs`), the `u32`
//!   `get_contract_version()` returns on-chain.
//! - The crate `version` in `contracts/will/Cargo.toml` (this test).
//! - The `contractmeta!(key = "Version", ...)` string literal embedded in the
//!   Wasm binary at build time, covered separately by `issue_272_test.rs`.
//!
//! This test closes the Cargo.toml gap using `env!("CARGO_PKG_VERSION")`,
//! which Cargo populates from the crate's own `Cargo.toml` at compile time --
//! no file parsing, so it can never itself go stale relative to what actually
//! got built. Before this fix, `.github/scripts/check-contract-version.sh`
//! (the shell-level counterpart, invoked by `test.yml` in CI) did not exist
//! at all despite being referenced by both CI and CONTRIBUTING.md, and
//! Cargo.toml had already drifted to `version = "0.1.0"` while
//! `CONTRACT_VERSION` decoded to `1.2.0` -- undetected because nothing ran
//! the check.
//!
//! What this test deliberately does *not* attempt: having
//! `get_contract_version()` read the value back out of the deployed Wasm
//! binary's `contractmeta!` custom section at call time. Soroban's contract
//! execution environment has no host function for a contract to introspect
//! its own deployed binary's custom sections, so `CONTRACT_VERSION` compiled
//! directly into the function body *is* the on-chain source of truth, not a
//! copy of it. The deploy-time/on-chain half of consistency validation lives
//! in `scripts/deploy-testnet.sh`, which calls a freshly deployed contract's
//! `get_contract_version` and compares it against source immediately after
//! deployment.

use crate::CONTRACT_VERSION;

#[test]
fn cargo_toml_version_matches_contract_version_constant() {
    let major = CONTRACT_VERSION / 1_000_000;
    let minor = (CONTRACT_VERSION / 1_000) % 1_000;
    let patch = CONTRACT_VERSION % 1_000;
    let decoded = std::format!("{major}.{minor}.{patch}");

    assert_eq!(
        env!("CARGO_PKG_VERSION"),
        decoded,
        "contracts/will/Cargo.toml's `version` must match CONTRACT_VERSION's \
         semver-decoded form; bump both together (see \
         CONTRIBUTING.md#contract-versioning)"
    );
}
