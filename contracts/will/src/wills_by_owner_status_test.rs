#![cfg(test)]

//! Regression coverage for `get_wills_by_owner_and_status` (#213): asserts
//! the query filters correctly by status.

use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::StellarAssetClient,
    vec, Address, Env,
};

use crate::{Allocation, Beneficiary, WillContract, WillContractClient, WillStatus};

const DAY: u64 = 86_400;

#[test]
fn filters_wills_by_owner_and_status() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_700_000_000);

    let owner = Address::generate(&env);
    let sac = env.register_stellar_asset_contract_v2(owner.clone());
    let token_address = sac.address();
    StellarAssetClient::new(&env, &token_address).mint(&owner, &10_000_000);

    let contract_id = env.register(WillContract, ());
    let client = WillContractClient::new(&env, &contract_id);
    let beneficiary = Address::generate(&env);

    let make_will = || {
        client.create_will(
            &owner,
            &vec![&env, (token_address.clone(), 1_000_000_i128)],
            &vec![
                &env,
                Beneficiary {
                    address: beneficiary.clone(),
                    allocation: Allocation::Percentage(10_000),
                },
            ],
            &90,
            &7,
            &vec![&env],
            &2,
            &None,
            &0,
        )
    };

    // Note: `cancel_will` and `release_inheritance` both prune the will from
    // the owner index entirely (#70/#71), so `Cancelled`/`Released` wills are
    // never returned by this owner-scoped query regardless of status filter.
    // This test exercises the statuses that do remain in the index.
    let active_will = make_will();
    let another_active_will = make_will();
    let triggered_will = make_will();

    env.ledger().with_mut(|l| l.timestamp += 91 * DAY);
    client.trigger_will(&triggered_will);

    let active = client.get_wills_by_owner_and_status(&owner, &WillStatus::Active, &None, &100);
    assert_eq!(active.len(), 2);
    let active_ids: std::collections::HashSet<u64> =
        active.iter().map(|w| w.id).collect();
    assert!(active_ids.contains(&active_will));
    assert!(active_ids.contains(&another_active_will));

    let triggered = client.get_wills_by_owner_and_status(&owner, &WillStatus::Triggered, &None, &100);
    assert_eq!(triggered.len(), 1);
    assert_eq!(triggered.get(0).unwrap().id, triggered_will);

    let released = client.get_wills_by_owner_and_status(&owner, &WillStatus::Released, &None, &100);
    assert!(released.is_empty());

    // Issue #296: Incremental pagination test
    let page1 = client.get_wills_by_owner_and_status(&owner, &WillStatus::Active, &None, &1);
    assert_eq!(page1.len(), 1);
    let p1_id = page1.get(0).unwrap().id;
    let page2 = client.get_wills_by_owner_and_status(&owner, &WillStatus::Active, &Some(p1_id), &1);
    assert_eq!(page2.len(), 1);
    assert_ne!(page2.get(0).unwrap().id, p1_id);
}
