#![cfg(test)]

//! Regression tests for issue #662: `distribute` must not pay a keeper bounty
//! to the contract's own address, which would be a circular transfer with no
//! withdrawal path.
//!
//! Before the fix, `should_pay_bounty` only excluded the will's owner from
//! receiving the bounty; the contract address itself was not excluded, so
//! passing `Some(contract_address)` as the keeper would deduct the bounty
//! from the beneficiaries' pool and attempt to transfer it to the contract,
//! where it could not be claimed.
//!
//! After the fix, a keeper address equal to `env.current_contract_address()`
//! is treated the same as `None` for bounty purposes: no bounty is deducted
//! and the full balance is distributed to beneficiaries.

use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::{Client as TokenClient, StellarAssetClient},
    vec, Address, Env, Vec as SorobanVec,
};

use crate::{Allocation, Beneficiary, WillContract, WillContractClient};

const DAY: u64 = 86_400;

fn setup(env: &Env) -> (WillContractClient, Address, Address, Address) {
    env.mock_all_auths();
    env.ledger().set_timestamp(1_700_000_000);

    let owner = Address::generate(env);
    let sac = env.register_stellar_asset_contract_v2(owner.clone());
    let token_address = sac.address();
    StellarAssetClient::new(env, &token_address).mint(&owner, &1_000_000i128);

    let contract_id = env.register(WillContract, ());
    let client = WillContractClient::new(env, &contract_id);

    (client, owner, token_address, contract_id)
}

fn advance(env: &Env, days: u64) {
    env.ledger().with_mut(|l| l.timestamp += days * DAY);
}

/// When the keeper address supplied to `release_inheritance` is the contract's
/// own address, no bounty should be deducted and the full balance must go to
/// the beneficiaries (#662).
#[test]
fn contract_address_as_keeper_receives_no_bounty() {
    let env = Env::default();
    let (client, owner, token_address, contract_id) = setup(&env);

    let beneficiary = Address::generate(&env);
    let locked_amount = 1_000_000i128;
    let bounty_bps: u32 = 50; // 0.5%

    let beneficiaries: SorobanVec<Beneficiary> = vec![
        &env,
        Beneficiary {
            address: beneficiary.clone(),
            allocation: Allocation::Percentage(10_000),
        },
    ];
    let tokens: SorobanVec<(Address, i128)> = vec![&env, (token_address.clone(), locked_amount)];

    let will_id = client.create_will(
        &owner,
        &tokens,
        &beneficiaries,
        &90,
        &7,
        &vec![&env],
        &0,
        &Some(bounty_bps),
        &0,
    );

    advance(&env, 91);
    client.trigger_will(&will_id);
    advance(&env, 8);

    // Pass the contract's own address as the keeper.
    client.release_inheritance(&will_id, &Some(contract_id.clone()));

    let token = TokenClient::new(&env, &token_address);

    // The beneficiary should receive the full balance — no bounty deducted.
    let beneficiary_balance = token.balance(&beneficiary);
    assert_eq!(
        beneficiary_balance, locked_amount,
        "beneficiary should receive full balance when keeper is the contract address"
    );

    // The contract itself should hold no bounty.
    let contract_balance = token.balance(&contract_id);
    assert_eq!(
        contract_balance, 0,
        "contract address should receive no bounty"
    );
}

/// A regular (non-owner, non-contract) keeper still receives the bounty
/// as before — this test guards against regressions in the happy path.
#[test]
fn valid_keeper_still_receives_bounty() {
    let env = Env::default();
    let (client, owner, token_address, _contract_id) = setup(&env);

    let beneficiary = Address::generate(&env);
    let keeper = Address::generate(&env);
    let locked_amount = 1_000_000i128;
    let bounty_bps: u32 = 50; // 0.5%

    let beneficiaries: SorobanVec<Beneficiary> = vec![
        &env,
        Beneficiary {
            address: beneficiary.clone(),
            allocation: Allocation::Percentage(10_000),
        },
    ];
    let tokens: SorobanVec<(Address, i128)> = vec![&env, (token_address.clone(), locked_amount)];

    let will_id = client.create_will(
        &owner,
        &tokens,
        &beneficiaries,
        &90,
        &7,
        &vec![&env],
        &0,
        &Some(bounty_bps),
        &0,
    );

    advance(&env, 91);
    client.trigger_will(&will_id);
    advance(&env, 8);
    client.release_inheritance(&will_id, &Some(keeper.clone()));

    let token = TokenClient::new(&env, &token_address);

    let expected_bounty = (locked_amount * bounty_bps as i128) / 10_000;
    let expected_beneficiary = locked_amount - expected_bounty;

    assert_eq!(token.balance(&keeper), expected_bounty);
    assert_eq!(token.balance(&beneficiary), expected_beneficiary);
    assert_eq!(
        token.balance(&keeper) + token.balance(&beneficiary),
        locked_amount
    );
}
