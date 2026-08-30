#![cfg(test)]

//! Regression test for issue #277: the module doc states that for a will
//! created with `confirmation_delay_seconds > 0`, "the check-in clock does
//! not start until confirmation." `create_will` sets `last_checkin: now`
//! regardless of starting status, so nothing previously proved that a
//! `PendingConfirmation` will's check-in deadline can't be reached (and thus
//! `trigger_will` can't fire) while confirmation is still outstanding.

use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::StellarAssetClient,
    vec, Address, Env,
};

use crate::{Allocation, Beneficiary, WillContract, WillContractClient, WillError, WillStatus};

const DAY: u64 = 86_400;

#[test]
fn pending_confirmation_will_cannot_be_triggered_before_confirmation() {
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

    // 90-day check-in period, 60-day confirmation window: never confirmed.
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
        &(60 * DAY),
    );
    assert_eq!(
        client.get_will(&will_id).status,
        WillStatus::PendingConfirmation
    );

    // Advance well past what would be the check-in deadline if the clock had
    // started at creation. The will is still PendingConfirmation (confirm_will
    // was never called), so trigger_will must still be rejected.
    env.ledger().with_mut(|l| l.timestamp += 91 * DAY);

    assert_eq!(
        client.try_trigger_will(&will_id),
        Err(Ok(WillError::WillNotActive.into()))
    );
}
