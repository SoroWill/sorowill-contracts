//! Native-host load tests using the production index cap.

use soroban_sdk::{
    testutils::{Address as _, EnvTestConfig, Ledger},
    token::{StellarAssetClient, TokenClient},
    vec, Address, Env, Vec,
};
use will::{Allocation, Beneficiary, WillContract, WillContractClient, WillError, WillStatus};

const DAY: u64 = 86_400;

#[test]
fn production_index_cap_and_late_pages() {
    let env = Env::new_with_config(EnvTestConfig {
        capture_snapshot_at_drop: false,
    });
    env.cost_estimate().budget().reset_unlimited();
    env.mock_all_auths();
    env.ledger().set_timestamp(DAY);
    let owner = Address::generate(&env);
    let beneficiary = Address::generate(&env);
    let token = env
        .register_stellar_asset_contract_v2(owner.clone())
        .address();
    let id = env.register(WillContract, ());
    let client = WillContractClient::new(&env, &id);
    StellarAssetClient::new(&env, &token).mint(&owner, &1_001);
    let tokens = vec![&env, (token.clone(), 1_i128)];
    let people = vec![
        &env,
        Beneficiary {
            address: beneficiary.clone(),
            allocation: Allocation::Percentage(10_000),
        },
    ];
    let spec = (tokens.clone(), people, 2, 1, Vec::new(&env), 0);
    let mut batch = Vec::new(&env);
    for _ in 0..10 {
        batch.push_back(spec.clone());
    }

    let mut expected = std::vec::Vec::new();
    for _ in 0..100 {
        expected.extend(client.batch_create_wills(&owner, &batch).iter());
    }

    let mut cursor = None;
    let mut actual = std::vec::Vec::new();
    loop {
        let page = client.get_wills_by_owner(&owner, &cursor, &u32::MAX);
        assert!(page.len() <= 50);
        let beneficiary_page = client.get_wills_by_beneficiary(&beneficiary, &cursor, &u32::MAX);
        assert_eq!(page, beneficiary_page);
        for will in page {
            assert!(cursor.is_none_or(|previous| will.id > previous));
            cursor = Some(will.id);
            actual.push(will.id);
        }
        if beneficiary_page.len() < 50 {
            break;
        }
    }
    assert_eq!(actual, expected);
    assert_eq!(actual.len(), 1_000);

    let overflow = client.try_batch_create_wills(&owner, &vec![&env, spec]);
    assert_eq!(overflow, Err(Ok(WillError::TooManyWills.into())));
    let balances = TokenClient::new(&env, &token);
    assert_eq!(balances.balance(&owner), 1);
    assert_eq!(balances.balance(&id), 1_000);

    let mut checkins = Vec::new(&env);
    for id in expected.iter().take(50) {
        checkins.push_back(*id);
    }
    env.ledger().set_timestamp(2 * DAY);
    client.batch_check_in(&checkins, &owner);
    checkins.push_back(expected[50]);
    assert!(client.try_batch_check_in(&checkins, &owner).is_err());
    assert_eq!(client.get_will(&expected[0]).last_checkin, 2 * DAY);
    assert_eq!(client.get_will(&expected[50]).last_checkin, DAY);

    let late = client.get_wills_by_owner(&owner, &Some(expected[989]), &10);
    assert_eq!(late.len(), 10);
    let resources = env.cost_estimate().resources();
    println!(
        "late owner page: {} instructions, {} in-memory reads, {} writes",
        resources.instructions, resources.memory_read_entries, resources.write_entries
    );
}

#[test]
fn parallel_workloads_serialize_competing_transactions_per_will() {
    std::thread::scope(|scope| {
        for worker in 0..4 {
            scope.spawn(move || {
                let env = Env::new_with_config(EnvTestConfig {
                    capture_snapshot_at_drop: false,
                });
                env.mock_all_auths();
                env.ledger().set_timestamp(DAY);
                let owner = Address::generate(&env);
                let recipient = Address::generate(&env);
                let token = env
                    .register_stellar_asset_contract_v2(owner.clone())
                    .address();
                let id = env.register(WillContract, ());
                let client = WillContractClient::new(&env, &id);
                StellarAssetClient::new(&env, &token).mint(&owner, &64);
                let mut ids = Vec::new(&env);
                for _ in 0..64 {
                    ids.push_back(client.create_will(
                        &owner,
                        &vec![&env, (token.clone(), 1_i128)],
                        &vec![
                            &env,
                            Beneficiary {
                                address: recipient.clone(),
                                allocation: Allocation::Percentage(10_000),
                            },
                        ],
                        &2,
                        &1,
                        &Vec::new(&env),
                        &0,
                        &None,
                        &0,
                    ));
                }

                env.ledger().set_timestamp(4 * DAY);
                for id in ids.iter() {
                    client.trigger_will(&id, &None);
                }
                if worker % 2 == 0 {
                    for id in ids.iter() {
                        client.emergency_checkin(&id, &owner);
                    }
                }
                env.ledger().set_timestamp(6 * DAY);
                for (index, id) in ids.into_iter().enumerate() {
                    if worker % 2 == 0 {
                        assert!(client.try_release_inheritance(&id, &None).is_err());
                        assert_eq!(client.get_will_status(&id), WillStatus::Active);
                    } else {
                        client.release_inheritance(&id, &None);
                        assert!(client.try_emergency_checkin(&id, &owner).is_err());
                        assert!(client.try_release_inheritance(&id, &None).is_err());
                        assert_eq!(client.get_will_status(&id), WillStatus::Released);
                        if index == 31 {
                            assert!(client.audit_protocol_stats().consistent);
                        }
                    }
                }
                let paid = if worker % 2 == 0 { 0 } else { 64 };
                let balances = TokenClient::new(&env, &token);
                assert_eq!(balances.balance(&recipient), paid);
                assert_eq!(balances.balance(&id), 64 - paid);
                assert!(client.get_triggered_wills(&None, &50).is_empty());
            });
        }
    });
}
