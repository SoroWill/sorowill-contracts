#![cfg(test)]

//! Regression test for validation in `renounce_beneficiary`.
//! Verifies that beneficiary allocation percentages are validated after
//! redistribution, catching rounding errors or invalid states.

use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::{Client as TokenClient, StellarAssetClient},
    vec, Address, Env,
};

use crate::{Allocation, Beneficiary, WillContract, WillContractClient};


fn setup<'a>() -> (Env, WillContractClient<'a>, Address, TokenClient<'a>, Address) {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_700_000_000);

    let owner = Address::generate(&env);
    let sac = env.register_stellar_asset_contract_v2(owner.clone());
    let token_address = sac.address();
    StellarAssetClient::new(&env, &token_address).mint(&owner, &1_000_000_000);

    let contract_id = env.register(WillContract, ());
    let client = WillContractClient::new(&env, &contract_id);

    (env.clone(), client, owner, TokenClient::new(&env, &token_address), token_address)
}

#[test]
fn renounce_beneficiary_redistributes_percentages_validly() {
    let (env, client, owner, _token, token_address) = setup();
    let a = Address::generate(&env);
    let b = Address::generate(&env);
    let c = Address::generate(&env);

    // Create will with three percentage beneficiaries
    let will_id = client.create_will(
        &owner,
        &vec![&env, (token_address.clone(), 1_000_000_i128)],
        &vec![
            &env,
            Beneficiary {
                address: a.clone(),
                allocation: Allocation::Percentage(3_333),
            },
            Beneficiary {
                address: b.clone(),
                allocation: Allocation::Percentage(3_333),
            },
            Beneficiary {
                address: c.clone(),
                allocation: Allocation::Percentage(3_334),
            },
        ],
        &90,
        &7,
        &vec![&env],
        &2,
        &None,
        &0,
    );

    // One beneficiary renounces
    client.renounce_beneficiary(&will_id, &b);

    // The resulting beneficiary list should still be valid
    let will = client.get_will(&will_id);
    let mut total_bp = 0_u32;
    for beneficiary in will.beneficiaries.iter() {
        if let Allocation::Percentage(bp) = beneficiary.allocation {
            total_bp = total_bp.saturating_add(bp);
        }
    }
    assert_eq!(total_bp, 10_000, "Percentages must sum to 10,000 after redistribution");
}

#[test]
fn renounce_beneficiary_with_rounding_edge_case() {
    let (env, client, owner, _token, token_address) = setup();
    let a = Address::generate(&env);
    let b = Address::generate(&env);
    let c = Address::generate(&env);

    // Create will with percentages that may have rounding implications
    let will_id = client.create_will(
        &owner,
        &vec![&env, (token_address.clone(), 1_000_000_i128)],
        &vec![
            &env,
            Beneficiary {
                address: a.clone(),
                allocation: Allocation::Percentage(3_000),
            },
            Beneficiary {
                address: b.clone(),
                allocation: Allocation::Percentage(3_500),
            },
            Beneficiary {
                address: c.clone(),
                allocation: Allocation::Percentage(3_500),
            },
        ],
        &90,
        &7,
        &vec![&env],
        &2,
        &None,
        &0,
    );

    // Middle beneficiary renounces
    client.renounce_beneficiary(&will_id, &b);

    // Verify the result is valid
    let will = client.get_will(&will_id);
    let mut total_bp = 0_u32;
    for beneficiary in will.beneficiaries.iter() {
        if let Allocation::Percentage(bp) = beneficiary.allocation {
            total_bp = total_bp.saturating_add(bp);
        }
    }
    assert_eq!(total_bp, 10_000, "Percentages must sum to 10,000 after rounding redistribution");
}

#[test]
fn renounce_beneficiary_single_percentage_beneficiary() {
    let (env, client, owner, _token, token_address) = setup();
    let a = Address::generate(&env);
    let b = Address::generate(&env);

    // Create will with fixed and percentage beneficiary
    let will_id = client.create_will(
        &owner,
        &vec![&env, (token_address.clone(), 1_000_000_i128)],
        &vec![
            &env,
            Beneficiary {
                address: a.clone(),
                allocation: Allocation::FixedAmount(300_000),
            },
            Beneficiary {
                address: b.clone(),
                allocation: Allocation::Percentage(10_000),
            },
        ],
        &90,
        &7,
        &vec![&env],
        &2,
        &None,
        &0,
    );

    // The percentage beneficiary renounces. With no percentage beneficiary
    // left to absorb their share, the remaining FixedAmount-only list only
    // covers 300_000 of the will's 1_000_000 balance -- that headroom is
    // legitimately left unaccounted for a will can still later gain a
    // hashed beneficiary (#181/#186) to claim it, so this is allowed rather
    // than rejected.
    client.renounce_beneficiary(&will_id, &b);

    let will = client.get_will(&will_id);
    assert_eq!(will.beneficiaries.len(), 1);
    let remaining = &will.beneficiaries.get(0).unwrap();
    assert!(matches!(remaining.allocation, Allocation::FixedAmount(300_000)));
}
