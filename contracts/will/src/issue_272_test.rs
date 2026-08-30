#![cfg(test)]

//! Regression test for issue #272: `CONTRACT_VERSION` and the `contractmeta!`
//! `"Version"` string are two independent representations of the same
//! version, and nothing checked they stayed consistent. They had in fact
//! already drifted: `CONTRACT_VERSION` documents a baseline of "1.0.0" while
//! `contractmeta!` still said `"0.1.0"`, left over from before
//! `CONTRACT_VERSION` was introduced. `contractmeta!` requires its `val` to
//! be a string literal (it cannot be derived from `CONTRACT_VERSION` via a
//! macro or const at compile time), so this test is the mechanism that keeps
//! the two from silently disagreeing again.

use crate::CONTRACT_VERSION;

/// Must match the literal passed to `contractmeta!(key = "Version", val =
/// ...)` in `lib.rs`. Update both together when bumping `CONTRACT_VERSION`.
const CONTRACTMETA_VERSION: &str = "1.0.0";

#[test]
fn contractmeta_version_matches_contract_version_constant() {
    let major = CONTRACT_VERSION / 1_000_000;
    let minor = (CONTRACT_VERSION / 1_000) % 1_000;
    let patch = CONTRACT_VERSION % 1_000;
    let decoded = std::format!("{major}.{minor}.{patch}");

    assert_eq!(
        CONTRACTMETA_VERSION, decoded,
        "contractmeta!'s \"Version\" literal in lib.rs must match \
         CONTRACT_VERSION's semver-decoded form; update both together"
    );
}
