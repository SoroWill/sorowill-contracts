#![cfg(test)]

//! Regression test for issue #265: the rustdoc on `events::will_created`
//! claims the `MAX_BENEFICIARIES` (10) cap "comfortably" fits inside
//! Soroban's per-event payload limit, but nothing actually constructed a
//! will with the maximum beneficiary count and asserted `create_will`
//! succeeds without hitting a host-level event/resource error.

use soroban_sdk::{testutils::Address as _, token::StellarAssetClient, vec, Address, Env, Vec};

use crate::{Allocation, Beneficiary, WillContract, WillContractClient, MAX_BENEFICIARIES};

#[test]
fn create_will_succeeds_with_max_beneficiaries_and_multiple_tokens() {
    let env = Env::default();
    env.mock_all_auths();

    let owner = Address::generate(&env);

    // Multiple locked tokens, not just one, per the acceptance criteria.
    const TOKEN_COUNT: u32 = 3;
    let mut tokens: Vec<(Address, i128)> = Vec::new(&env);
    for _ in 0..TOKEN_COUNT {
        let token_address = env
            .register_stellar_asset_contract_v2(owner.clone())
            .address();
        StellarAssetClient::new(&env, &token_address).mint(&owner, &1_000_000);
        tokens.push_back((token_address, 1_000_000_i128));
    }

    // Exactly MAX_BENEFICIARIES beneficiaries, each a full Address plus a
    // full Allocation, sharing the balance evenly (remainder to the last).
    let mut beneficiaries: Vec<Beneficiary> = Vec::new(&env);
    let even_share = 10_000 / MAX_BENEFICIARIES;
    let mut allocated = 0u32;
    for i in 0..MAX_BENEFICIARIES {
        let share = if i == MAX_BENEFICIARIES - 1 {
            10_000 - allocated
        } else {
            even_share
        };
        allocated += share;
        beneficiaries.push_back(Beneficiary {
            address: Address::generate(&env),
            allocation: Allocation::Percentage(share),
        });
    }
    assert_eq!(beneficiaries.len(), MAX_BENEFICIARIES);

    let contract_id = env.register(WillContract, ());
    let client = WillContractClient::new(&env, &contract_id);

    // If the will_created event payload didn't fit Soroban's per-event
    // limit, this call would abort at the host level rather than return
    // normally.
    let will_id = client.create_will(
        &owner,
        &tokens,
        &beneficiaries,
        &90,
        &7,
        &vec![&env],
        &1,
        &None,
        &0,
    );

    let will = client.get_will(&will_id);
    assert_eq!(will.beneficiaries.len(), MAX_BENEFICIARIES);
    assert_eq!(will.balances.len(), TOKEN_COUNT);
}
