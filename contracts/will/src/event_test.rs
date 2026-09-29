#![cfg(test)]

//! Unit test for the extended `will_created` event payload.
//!
//! Previously `will_created` published only `beneficiaries_count`, so an
//! off-chain indexer building an activity feed purely from events (rather
//! than always falling back to `get_will`) could not reconstruct who a
//! will's beneficiaries actually are. The event now publishes the full
//! beneficiary list (address + allocation pairs), bounded by
//! `MAX_BENEFICIARIES` — see the doc comment on `events::will_created` for
//! the payload-size tradeoff.
//!
//! Self-contained module (its own `setup`) so it does not depend on the
//! large pre-existing `test.rs` suite, which still constructs `Beneficiary`
//! via the old `basis_points` field.

use soroban_sdk::{
    symbol_short,
    testutils::{Address as _, Events, Ledger},
    token::StellarAssetClient,
    vec, Address, Env, TryIntoVal,
};

use crate::{Allocation, Beneficiary, WillContract, WillContractClient};

#[test]
fn will_created_event_includes_full_beneficiary_list() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_700_000_000);

    let owner = Address::generate(&env);
    let sac = env.register_stellar_asset_contract_v2(owner.clone());
    let token_address = sac.address();
    StellarAssetClient::new(&env, &token_address).mint(&owner, &1_000_000_000);

    let contract_id = env.register(WillContract, ());
    let client = WillContractClient::new(&env, &contract_id);

    let beneficiary_a = Address::generate(&env);
    let beneficiary_b = Address::generate(&env);
    let beneficiaries = vec![
        &env,
        Beneficiary {
            address: beneficiary_a.clone(),
            allocation: Allocation::Percentage(7_000),
        },
        Beneficiary {
            address: beneficiary_b.clone(),
            allocation: Allocation::Percentage(3_000),
        },
    ];
    let tokens = vec![&env, (token_address, 1_000_000_i128)];

    let will_id = client.create_will(
        &owner,
        &tokens,
        &beneficiaries,
        &90,
        &7,
        &vec![&env],
        &2,
        &None,
        &0,
    );

    let events = env.events().all();
    let mut found = false;
    for event in events.iter() {
        if event.1.is_empty() {
            continue;
        }
        let topic0: Result<soroban_sdk::Symbol, _> = event.1.get(0).unwrap().try_into_val(&env);
        if topic0 != Ok(symbol_short!("created")) {
            continue;
        }
        let topic1: u64 = event.1.get(1).unwrap().try_into_val(&env).unwrap();
        if topic1 != will_id {
            continue;
        }

        found = true;
        let data: (Address, u32, soroban_sdk::Vec<Beneficiary>, u64) =
            event.2.try_into_val(&env).unwrap();
        assert_eq!(data.0, owner, "event owner does not match the creator");
        assert_eq!(
            data.1, 1,
            "event token_count does not match the locked token count"
        );
        assert_eq!(
            data.2, beneficiaries,
            "event beneficiary list does not match what was supplied to create_will"
        );
        assert_eq!(data.2.len(), 2);
    }

    assert!(found, "will_created event not found");
}

/// Verify that `will_created` event is correctly emitted when guardians are
/// specified at will creation time, and the event includes the full beneficiary list.
#[test]
fn will_created_event_with_guardians_included() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_700_000_000);

    let owner = Address::generate(&env);
    let sac = env.register_stellar_asset_contract_v2(owner.clone());
    let token_address = sac.address();
    StellarAssetClient::new(&env, &token_address).mint(&owner, &1_000_000_000);

    let contract_id = env.register(WillContract, ());
    let client = WillContractClient::new(&env, &contract_id);

    let beneficiary_a = Address::generate(&env);
    let beneficiary_b = Address::generate(&env);
    let guardian_1 = Address::generate(&env);
    let guardian_2 = Address::generate(&env);

    let beneficiaries = vec![
        &env,
        Beneficiary {
            address: beneficiary_a.clone(),
            allocation: Allocation::Percentage(6_000),
        },
        Beneficiary {
            address: beneficiary_b.clone(),
            allocation: Allocation::Percentage(4_000),
        },
    ];
    let tokens = vec![&env, (token_address, 1_000_000_i128)];
    let guardians = vec![&env, guardian_1.clone(), guardian_2.clone()];

    let will_id = client.create_will(
        &owner,
        &tokens,
        &beneficiaries,
        &90,
        &7,
        &guardians,
        &2,
        &None,
        &0,
    );

    let events = env.events().all();
    let mut found = false;
    for event in events.iter() {
        if event.1.is_empty() {
            continue;
        }
        let topic0: Result<soroban_sdk::Symbol, _> = event.1.get(0).unwrap().try_into_val(&env);
        if topic0 != Ok(symbol_short!("created")) {
            continue;
        }
        let topic1: u64 = event.1.get(1).unwrap().try_into_val(&env).unwrap();
        if topic1 != will_id {
            continue;
        }

        found = true;
        let data: (Address, u32, soroban_sdk::Vec<Beneficiary>, u64) =
            event.2.try_into_val(&env).unwrap();
        assert_eq!(data.0, owner, "event owner must match creator");
        assert_eq!(data.1, 1, "event token count must be one");
        assert_eq!(
            data.2, beneficiaries,
            "event beneficiary list must match supplied list"
        );
        assert_eq!(data.2.len(), 2, "beneficiary count must be two");
    }

    assert!(found, "will_created event must be emitted with guardians");
}
