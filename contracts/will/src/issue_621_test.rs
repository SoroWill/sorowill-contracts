#![cfg(test)]

//! Regression tests for issue #621: percentage shares that round down must not
//! leave dust behind. Every token balance is paid out in full, one unit at most
//! apart from the exact split, because the last percentage beneficiary absorbs
//! the rounding remainder for each token.

use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::{Client as TokenClient, StellarAssetClient},
    vec, Address, Env, Vec as SorobanVec,
};

use crate::{Allocation, Beneficiary, WillContract, WillContractClient};

const DAY: u64 = 86_400;

#[test]
fn rounding_leaves_no_dust_across_two_tokens() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_700_000_000);

    let owner = Address::generate(&env);
    let token_a = env.register_stellar_asset_contract_v2(owner.clone()).address();
    let token_b = env.register_stellar_asset_contract_v2(owner.clone()).address();
    // Amounts that three-way percentage splits cannot divide evenly.
    StellarAssetClient::new(&env, &token_a).mint(&owner, &1_000_007);
    StellarAssetClient::new(&env, &token_b).mint(&owner, &13);

    let contract_id = env.register(WillContract, ());
    let client = WillContractClient::new(&env, &contract_id);

    let b1 = Address::generate(&env);
    let b2 = Address::generate(&env);
    let b3 = Address::generate(&env);
    let beneficiaries: SorobanVec<Beneficiary> = vec![
        &env,
        Beneficiary { address: b1.clone(), allocation: Allocation::Percentage(3_333) },
        Beneficiary { address: b2.clone(), allocation: Allocation::Percentage(3_333) },
        Beneficiary { address: b3.clone(), allocation: Allocation::Percentage(3_334) },
    ];
    let tokens = vec![
        &env,
        (token_a.clone(), 1_000_007_i128),
        (token_b.clone(), 13_i128),
    ];
    let will_id = client.create_will(
        &owner,
        &tokens,
        &beneficiaries,
        &90,
        &7,
        &vec![&env],
        &0,
        &None,
        &0,
    );

    env.ledger().with_mut(|l| l.timestamp += 91 * DAY);
    client.trigger_will(&will_id);
    env.ledger().with_mut(|l| l.timestamp += 8 * DAY);
    client.release_inheritance(&will_id, &None);

    let a = TokenClient::new(&env, &token_a);
    let b = TokenClient::new(&env, &token_b);
    assert_eq!(
        a.balance(&b1) + a.balance(&b2) + a.balance(&b3),
        1_000_007,
        "token A must be paid out in full"
    );
    assert_eq!(
        b.balance(&b1) + b.balance(&b2) + b.balance(&b3),
        13,
        "token B must be paid out in full"
    );
    assert_eq!(a.balance(&contract_id), 0, "no token A dust may stay in the contract");
    assert_eq!(b.balance(&contract_id), 0, "no token B dust may stay in the contract");
}
