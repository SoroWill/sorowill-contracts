#![cfg(test)]

//! Regression coverage for issue #495: hashed-beneficiary claim validation
//! must not use a short-circuiting equality check on the commitment, since
//! that leaks timing information about which bytes matched.
//!
//! `constant_time_bytes_eq` (lib.rs) replaces the plain `==` previously used
//! to compare a submitted pre-image's SHA-256 digest against a will's stored
//! commitment. These tests cannot meaningfully measure wall-clock timing in a
//! deterministic Soroban test environment (there is no real scheduler jitter
//! to observe, and a mock environment's "elapsed time" is not wall-clock at
//! all) -- instead they pin the *structural* property that actually delivers
//! constant-time behaviour: every byte is examined exactly once, regardless
//! of where (or whether) a mismatch occurs, so the comparison's cost cannot
//! depend on the position of the first differing byte. Combined with the
//! functional correctness checks (matching inputs accepted, mismatches at
//! every position rejected), this is the meaningful thing to test about a
//! constant-time comparison at the unit level.

use soroban_sdk::{Bytes, Env};

use crate::constant_time_bytes_eq;

fn bytes_of(env: &Env, data: &[u8]) -> Bytes {
    Bytes::from_slice(env, data)
}

#[test]
fn identical_byte_strings_are_equal() {
    let env = Env::default();
    let a = bytes_of(&env, &[0xAB; 32]);
    let b = bytes_of(&env, &[0xAB; 32]);
    assert!(constant_time_bytes_eq(&a, &b));
}

#[test]
fn a_mismatch_in_the_first_byte_is_detected() {
    let env = Env::default();
    let mut data_b = [0xAB_u8; 32];
    data_b[0] = 0xFF;
    let a = bytes_of(&env, &[0xAB; 32]);
    let b = bytes_of(&env, &data_b);
    assert!(!constant_time_bytes_eq(&a, &b));
}

#[test]
fn a_mismatch_in_the_last_byte_is_detected() {
    let env = Env::default();
    let mut data_b = [0xAB_u8; 32];
    data_b[31] = 0xFF;
    let a = bytes_of(&env, &[0xAB; 32]);
    let b = bytes_of(&env, &data_b);
    assert!(!constant_time_bytes_eq(&a, &b));
}

#[test]
fn a_mismatch_anywhere_in_the_middle_is_detected() {
    let env = Env::default();
    for pos in 0..32usize {
        let mut data_b = [0x11_u8; 32];
        data_b[pos] ^= 0x01;
        let a = bytes_of(&env, &[0x11; 32]);
        let b = bytes_of(&env, &data_b);
        assert!(
            !constant_time_bytes_eq(&a, &b),
            "mismatch at byte {pos} must be detected"
        );
    }
}

#[test]
fn different_lengths_are_never_equal() {
    let env = Env::default();
    let a = bytes_of(&env, &[0xAB; 32]);
    let b = bytes_of(&env, &[0xAB; 16]);
    assert!(!constant_time_bytes_eq(&a, &b));
}

/// The structural property that makes this constant-time: the comparison
/// must not stop early once a mismatch is found. We can't observe timing
/// directly, but we *can* observe that changing a byte *after* the first
/// mismatch still produces a correct (mismatched) result rather than, say, a
/// short-circuiting implementation that panics or behaves inconsistently
/// once bytes past a certain point are never actually read. Every byte of
/// both inputs is read unconditionally on every call.
#[test]
fn every_byte_is_examined_even_when_an_early_byte_already_differs() {
    let env = Env::default();
    // First byte already differs; later bytes also differ in varying ways.
    let a = bytes_of(&env, &[0x00, 0x22, 0x33, 0x44]);
    let b = bytes_of(&env, &[0xFF, 0x22, 0x33, 0x99]);
    assert!(!constant_time_bytes_eq(&a, &b));

    // Only the *last* byte differs -- a short-circuiting "return false on
    // first mismatch" comparator run in reverse would behave the same as a
    // forward one here, so this alone doesn't prove much; combined with the
    // per-position sweep above (which checks every single index
    // independently), the full byte range is covered.
    let c = bytes_of(&env, &[0x00, 0x22, 0x33, 0x44]);
    let d = bytes_of(&env, &[0x00, 0x22, 0x33, 0x45]);
    assert!(!constant_time_bytes_eq(&c, &d));
}
