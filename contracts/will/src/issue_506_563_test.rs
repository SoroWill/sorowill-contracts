#![cfg(test)]

//! Regression tests for issues #506-#563:
//! Specify and verify rounding direction (floor truncation and ceiling remainder absorption).
//!
//! Verifies that proportional calculations strictly round down (floor), and that any
//! rounding dust is absorbed by the final percentage beneficiary.

use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::StellarAssetClient,
    vec, Address, Env,
};

use crate::{
    proportional_share, Allocation, Beneficiary, WillContract, WillContractClient, WillStatus,
};

const DAY: u64 = 86_400;

#[test]
fn test_proportional_share_strict_floor_rounding() {
    // 9 units distributed at 900 basis points (9%): 9 * 900 / 10_000 = 0.81 units.
    // Strictly floor rounds to 0.
    assert_eq!(proportional_share(9, 900), 0);

    // 19 units at 5000 basis points (50%): 19 * 5000 / 10_000 = 9.5 units.
    // Strictly floor rounds to 9.
    assert_eq!(proportional_share(19, 5000), 9);

    // 100 units at 3333 basis points (33.33%): 33.33 units.
    // Strictly floor rounds to 33.
    assert_eq!(proportional_share(100, 3333), 33);
}

#[test]
fn test_distribution_floor_rounding_and_remainder_absorption() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_700_000_000);

    let owner = Address::generate(&env);
    let token_address = env
        .register_stellar_asset_contract_v2(owner.clone())
        .address();
    let token_client = StellarAssetClient::new(&env, &token_address);
    token_client.mint(&owner, &100);

    let contract_id = env.register(WillContract, ());
    let client = WillContractClient::new(&env, &contract_id);

    let b1 = Address::generate(&env);
    let b2 = Address::generate(&env);
    let b3 = Address::generate(&env);

    // 3 beneficiaries with 3333, 3333, 3334 basis points
    let will_id = client.create_will(
        &owner,
        &vec![&env, (token_address.clone(), 100_i128)],
        &vec![
            &env,
            Beneficiary {
                address: b1.clone(),
                allocation: Allocation::Percentage(3333),
            },
            Beneficiary {
                address: b2.clone(),
                allocation: Allocation::Percentage(3333),
            },
            Beneficiary {
                address: b3.clone(),
                allocation: Allocation::Percentage(3334),
            },
        ],
        &90,
        &7,
        &vec![&env],
        &0,
        &None,
        &0,
    );

    // Advance past check-in and trigger deadline
    env.ledger().with_mut(|l| l.timestamp += 91 * DAY);
    client.trigger_will(&will_id);

    // Advance past grace period
    env.ledger().with_mut(|l| l.timestamp += 8 * DAY);
    client.release_inheritance(&will_id, &None);

    let will = client.get_will(&will_id);
    assert_eq!(will.status, WillStatus::Released);

    // b1: floor(100 * 3333 / 10000) = 33
    // b2: floor(100 * 3333 / 10000) = 33
    // b3 (final beneficiary absorbs remainder 100 - 33 - 33): 34
    let token_contract = soroban_sdk::token::Client::new(&env, &token_address);
    assert_eq!(token_contract.balance(&b1), 33);
    assert_eq!(token_contract.balance(&b2), 33);
    assert_eq!(token_contract.balance(&b3), 34);
}
