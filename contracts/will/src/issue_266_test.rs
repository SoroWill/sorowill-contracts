#![cfg(test)]

//! Regression test for issue #266: `split_will` renormalises the caller-
//! supplied `beneficiaries_to_split` allocations (used verbatim for the new
//! child will, independent of whatever is actually stored for those
//! addresses in the source will) via `renormalize_percentages`, but never
//! re-validated the result through `assert_valid_allocations` before saving
//! -- so a skewed input ratio that renormalises a beneficiary down to
//! exactly `Allocation::Percentage(0)` used to be saved silently instead of
//! rejected.

use soroban_sdk::{testutils::Address as _, token::StellarAssetClient, vec, Address, Env};

use crate::{Allocation, Beneficiary, WillContract, WillContractClient, WillError};

#[test]
fn split_will_rejects_a_renormalised_share_that_rounds_to_zero() {
    let env = Env::default();
    env.mock_all_auths();

    let owner = Address::generate(&env);
    let a = Address::generate(&env);
    let b = Address::generate(&env);
    let c = Address::generate(&env);
    let token_address = env
        .register_stellar_asset_contract_v2(owner.clone())
        .address();
    StellarAssetClient::new(&env, &token_address).mint(&owner, &1_000_000);

    let contract_id = env.register(WillContract, ());
    let client = WillContractClient::new(&env, &contract_id);

    // Three beneficiaries, real shares irrelevant to the exploit below --
    // split_will renormalises whatever allocations the caller supplies for
    // beneficiaries_to_split, not the ones actually stored here.
    let will_id = client.create_will(
        &owner,
        &vec![&env, (token_address, 1_000_000_i128)],
        &vec![
            &env,
            Beneficiary { address: a.clone(), allocation: Allocation::Percentage(100) },
            Beneficiary { address: b.clone(), allocation: Allocation::Percentage(100) },
            Beneficiary { address: c.clone(), allocation: Allocation::Percentage(9_800) },
        ],
        &90,
        &7,
        &vec![&env],
        &1,
        &None,
        &0,
    );

    // Split off [a, b] with a deliberately skewed ratio: renormalising
    // 1 : 999_999 rescales `a`'s share to floor(1 * 10_000 / 1_000_000) = 0.
    let skewed_split = vec![
        &env,
        Beneficiary { address: a, allocation: Allocation::Percentage(1) },
        Beneficiary { address: b, allocation: Allocation::Percentage(999_999) },
    ];

    assert_eq!(
        client.try_split_will(&will_id, &owner, &skewed_split, &100_000),
        Err(Ok(WillError::InvalidPercentages.into())),
        "a renormalised share of exactly 0 bp must be rejected, not silently saved"
    );

    // The source will must be untouched by the rejected attempt.
    let source = client.get_will(&will_id);
    assert_eq!(source.beneficiaries.len(), 3);
    assert_eq!(source.balance, 1_000_000);
}
