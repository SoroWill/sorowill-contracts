#![cfg(test)]

//! Regression test for issue #279: `cancel_will` correctly excludes
//! `Triggered` wills (the owner must call `emergency_checkin` first per the
//! rustdoc), but no test explicitly exercised the negative case — attempting
//! to cancel a `Triggered` will and asserting the `WillNotActive` rejection.

use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::StellarAssetClient,
    vec, Address, Env,
};

use crate::{Allocation, Beneficiary, WillContract, WillContractClient, WillError};

const DAY: u64 = 86_400;

#[test]
fn cancel_will_is_rejected_for_a_triggered_will() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_700_000_000);

    let owner = Address::generate(&env);
    let beneficiary = Address::generate(&env);
    let token_address = env
        .register_stellar_asset_contract_v2(owner.clone())
        .address();
    StellarAssetClient::new(&env, &token_address).mint(&owner, &1_000_000);

    let contract_id = env.register(WillContract, ());
    let client = WillContractClient::new(&env, &contract_id);

    let will_id = client.create_will(
        &owner,
        &vec![&env, (token_address, 1_000_000_i128)],
        &vec![
            &env,
            Beneficiary {
                address: beneficiary,
                allocation: Allocation::Percentage(10_000),
            },
        ],
        &90,
        &7,
        &vec![&env],
        &1,
        &None,
        &0,
    );

    // Miss the check-in deadline and trigger the will.
    env.ledger().with_mut(|l| l.timestamp += 91 * DAY);
    client.trigger_will(&will_id);

    // Cancelling a Triggered will must be rejected: the owner has to prove
    // they're alive via `emergency_checkin` first, not cancel directly.
    assert_eq!(
        client.try_cancel_will(&will_id, &owner),
        Err(Ok(WillError::WillNotActive.into()))
    );
}
