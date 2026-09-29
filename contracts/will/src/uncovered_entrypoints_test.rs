#![cfg(test)]

//! Minimal coverage for entry points that `entrypoint_coverage_test` found
//! had zero references anywhere in the test suite: `update_periods` and
//! `reject_guardian_role`.

use soroban_sdk::{
    symbol_short,
    testutils::{Address as _, Events, Ledger},
    token::StellarAssetClient,
    vec, Address, Env, TryIntoVal, Vec as SorobanVec,
};

use crate::{
    Allocation, Beneficiary, GuardianVoteReason, WillContract, WillContractClient, WillError,
};

/// Creates a one-guardian will and returns the will id plus the guardian.
fn setup_with_guardian<'a>() -> (Env, WillContractClient<'a>, Address, u64, Address) {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_700_000_000);

    let owner = Address::generate(&env);
    let sac = env.register_stellar_asset_contract_v2(owner.clone());
    let token_address = sac.address();
    StellarAssetClient::new(&env, &token_address).mint(&owner, &1_000_000_000);

    let contract_id = env.register(WillContract, ());
    let client = WillContractClient::new(&env, &contract_id);

    let beneficiary = Address::generate(&env);
    let guardian = Address::generate(&env);

    let beneficiaries: SorobanVec<Beneficiary> = vec![
        &env,
        Beneficiary {
            address: beneficiary,
            allocation: Allocation::Percentage(10_000),
        },
    ];
    let tokens: SorobanVec<(Address, i128)> = vec![&env, (token_address, 1_000_000_i128)];

    let will_id = client.create_will(
        &owner,
        &tokens,
        &beneficiaries,
        &90,
        &7,
        &vec![&env, guardian.clone()],
        &1,
        &None,
        &0,
    );

    (env, client, owner, will_id, guardian)
}

/// Returns the payload of the single event whose topic is `(symbol, will_id)`.
fn find_payload(env: &Env, symbol: soroban_sdk::Symbol, will_id: u64) -> Option<soroban_sdk::Val> {
    env.events().all().iter().find_map(|event| {
        let t0: Result<soroban_sdk::Symbol, _> = event.1.get(0)?.try_into_val(env);
        let t1: Result<u64, _> = event.1.get(1)?.try_into_val(env);
        match (t0, t1) {
            (Ok(s), Ok(id)) if s == symbol && id == will_id => Some(event.2),
            _ => None,
        }
    })
}

#[test]
fn accept_guardian_role_publishes_gaccept_with_the_guardian() {
    let (env, client, _owner, will_id, guardian) = setup_with_guardian();

    client.accept_guardian_role(&will_id, &guardian);

    let payload = find_payload(&env, symbol_short!("gaccept"), will_id)
        .expect("accept_guardian_role must publish a gaccept event");
    let address: Result<Address, _> = payload.try_into_val(&env);
    assert_eq!(
        address,
        Ok(guardian),
        "gaccept payload must be the accepting guardian"
    );
}

#[test]
fn reject_guardian_role_publishes_greject_with_the_guardian() {
    let (env, client, _owner, will_id, guardian) = setup_with_guardian();

    client.reject_guardian_role(&will_id, &guardian);

    let payload = find_payload(&env, symbol_short!("greject"), will_id)
        .expect("reject_guardian_role must publish a greject event");
    let address: Result<Address, _> = payload.try_into_val(&env);
    assert_eq!(
        address,
        Ok(guardian),
        "greject payload must be the rejecting guardian"
    );
}

#[test]
fn accept_then_reject_publishes_the_matching_reject_event() {
    // `env.events().all()` only retains events from the last top-level
    // invocation, so the accept and the reject have to be checked in separate
    // tests. Here the last call is the reject, so the greject event is what
    // must be visible — and gaccept must have been cleared with the accept.
    let (env, client, _owner, will_id, guardian) = setup_with_guardian();

    client.accept_guardian_role(&will_id, &guardian);
    client.reject_guardian_role(&will_id, &guardian);

    assert!(
        find_payload(&env, symbol_short!("greject"), will_id).is_some(),
        "the most recent guardian-consent event must be greject"
    );
}

fn setup<'a>() -> (Env, WillContractClient<'a>, Address, Address) {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_700_000_000);

    let owner = Address::generate(&env);
    let sac = env.register_stellar_asset_contract_v2(owner.clone());
    let token_address = sac.address();
    StellarAssetClient::new(&env, &token_address).mint(&owner, &1_000_000_000);

    let contract_id = env.register(WillContract, ());
    let client = WillContractClient::new(&env, &contract_id);

    (env, client, owner, token_address)
}

#[test]
fn update_periods_changes_checkin_and_grace_periods() {
    let (env, client, owner, token_address) = setup();
    let beneficiary = Address::generate(&env);

    let beneficiaries: SorobanVec<Beneficiary> = vec![
        &env,
        Beneficiary {
            address: beneficiary,
            allocation: Allocation::Percentage(10_000),
        },
    ];
    let tokens: SorobanVec<(Address, i128)> = vec![&env, (token_address, 1_000_000_i128)];

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

    client.update_periods(&will_id, &owner, &Some(30), &Some(14));

    let will = client.get_will(&will_id);
    assert_eq!(will.checkin_period_days, 30);
    assert_eq!(will.grace_period_days, 14);
}

#[test]
fn reject_guardian_role_marks_the_guardian_as_rejected_and_blocks_voting() {
    let (env, client, owner, token_address) = setup();
    let beneficiary = Address::generate(&env);
    let guardian = Address::generate(&env);

    let beneficiaries: SorobanVec<Beneficiary> = vec![
        &env,
        Beneficiary {
            address: beneficiary,
            allocation: Allocation::Percentage(10_000),
        },
    ];
    let tokens: SorobanVec<(Address, i128)> = vec![&env, (token_address, 1_000_000_i128)];

    let will_id = client.create_will(
        &owner,
        &tokens,
        &beneficiaries,
        &90,
        &7,
        &vec![&env, guardian.clone()],
        &1,
        &None,
        &0,
    );

    client.reject_guardian_role(&will_id, &guardian);

    // Clear the guardian-list cooldown so the rejection below is
    // specifically GuardianNotConsented, not GuardianCooldownActive.
    env.ledger().with_mut(|l| l.timestamp += 8 * 86_400);

    // A guardian who explicitly rejected the role has not consented, so
    // guardian_trigger must refuse their vote.
    assert_eq!(
        client.try_guardian_trigger(&will_id, &guardian, &GuardianVoteReason::Other),
        Err(Ok(WillError::GuardianNotConsented.into())),
    );
}
