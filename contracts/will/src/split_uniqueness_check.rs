//! Duplicate-address validation for `split_will` (#422).

use soroban_sdk::{panic_with_error, Env, Vec};

use crate::{Beneficiary, WillError};

/// Panics with [`WillError::InvalidSplit`] if any address appears more than
/// once in the list of beneficiaries being split out.
pub(crate) fn assert_split_addresses_unique(env: &Env, list: &Vec<Beneficiary>) {
    for i in 0..list.len() {
        let a = list.get(i).unwrap();
        for j in (i + 1)..list.len() {
            if list.get(j).unwrap().address == a.address {
                panic_with_error!(env, WillError::InvalidSplit);
            }
        }
    }
}
