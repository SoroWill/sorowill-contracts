#![cfg(test)]

use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::{Client as TokenClient, StellarAssetClient},
    vec, Address, Env,
};

use crate::{Beneficiary, GuardianVoteReason, WillContract, WillContractClient, WillStatus};

const DAY: u64 = 86_400;

fn setup<'a>() -> (
    Env,
    WillContractClient<'a>,
    Address,
    TokenClient<'a>,
    Address,
) {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_700_000_000);

    let owner = Address::generate(&env);
    let sac = env.register_stellar_asset_contract_v2(owner.clone());
    let token_address = sac.address();
    let token_client = TokenClient::new(&env, &token_address);
    StellarAssetClient::new(&env, &token_address).mint(&owner, &1_000_000_000);

    let contract_id = env.register(WillContract, ());
    let client = WillContractClient::new(&env, &contract_id);

    (env.clone(), client, owner, token_client, token_address)
}

fn advance_time(env: &Env, seconds: u64) {
    env.ledger().with_mut(|l| { l.timestamp += seconds; });
}

// ── Issue #298: guardian_trigger does not pay keeper bounty ───────────────────

#[test]
fn test_guardian_trigger_does_not_pay_keeper_bounty() {
    let (env, client, owner, token, token_address) = setup();
    let beneficiary = Address::generate(&env);
    let g1 = Address::generate(&env);
    let g2 = Address::generate(&env);

    let will_id = client.create_will(
        &owner,
        &vec![&env, (token_address.clone(), 1_000_000_i128)],
        &vec![&env, Beneficiary { address: beneficiary.clone(), allocation: crate::Allocation::Percentage(10_000) }],
        &90,
        &7,
        &vec![&env, g1.clone(), g2.clone()],
        &2,
        &Some(50),
        &0,
    );

    advance_time(&env, 8 * DAY);
    client.accept_guardian_role(&will_id, &g1);
    client.accept_guardian_role(&will_id, &g2);
    client.guardian_trigger(&will_id, &g1, &GuardianVoteReason::Incapacitated);
    client.guardian_trigger(&will_id, &g2, &GuardianVoteReason::Incapacitated);

    let will = client.get_will(&will_id);
    assert_eq!(will.status, WillStatus::Released);
    assert_eq!(token.balance(&beneficiary), 1_000_000);
}

// ── Issue #283: close_will rejects already-settled will ───────────────────────

#[test]
fn test_close_will_rejects_already_settled() {
    let (env, client, owner, _token, token_address) = setup();
    let beneficiary = Address::generate(&env);

    let will_id = client.create_will(
        &owner,
        &vec![&env, (token_address.clone(), 1_000_000_i128)],
        &vec![&env, Beneficiary { address: beneficiary.clone(), allocation: crate::Allocation::Percentage(10_000) }],
        &90,
        &7,
        &vec![&env],
        &2,
        &None,
        &0,
    );

    advance_time(&env, 91 * DAY);
    client.trigger_will(&will_id);
    advance_time(&env, 8 * DAY);
    client.release_inheritance(&will_id, &None);
    client.close_will(&will_id, &owner);

    let result = client.try_close_will(&will_id, &owner);
    assert!(result.is_err());
}

// ── Issue #294: batch_check_in is atomic on invalid id ───────────────────────

#[test]
fn test_batch_check_in_atomicity_on_invalid_id() {
    let (env, client, owner, _token, token_address) = setup();
    let beneficiary = Address::generate(&env);

    let will_id_1 = client.create_will(
        &owner,
        &vec![&env, (token_address.clone(), 1_000_000)],
        &vec![&env, Beneficiary { address: beneficiary.clone(), allocation: crate::Allocation::Percentage(10_000) }],
        &90,
        &7,
        &vec![&env],
        &2,
        &None,
        &0,
    );

    let will_id_2 = client.create_will(
        &owner,
        &vec![&env, (token_address, 500_000)],
        &vec![&env, Beneficiary { address: beneficiary, allocation: crate::Allocation::Percentage(10_000) }],
        &60,
        &5,
        &vec![&env],
        &2,
        &None,
        &0,
    );

    advance_time(&env, 5 * DAY);

    let invalid_id = 999999u64;
    let result = client.try_batch_check_in(&vec![&env, will_id_1, invalid_id, will_id_2], &owner);
    assert!(result.is_err());

    let will_1 = client.get_will(&will_id_1);
    assert_eq!(will_1.last_checkin, 1_700_000_000);

    let will_2 = client.get_will(&will_id_2);
    assert_eq!(will_2.last_checkin, 1_700_000_000);
}
