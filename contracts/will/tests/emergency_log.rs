//! Integration coverage for the append-only emergency action log.

use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::StellarAssetClient,
    vec, Address, Env, Vec,
};
use will::{Allocation, Beneficiary, WillContract, WillContractClient};

const DAY: u64 = 86_400;

#[test]
fn emergency_log_is_append_only_and_survives_will_archival() {
    use will::{ActionLogScope, EmergencyAction};

    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(DAY);
    let owner = Address::generate(&env);
    let token = env
        .register_stellar_asset_contract_v2(owner.clone())
        .address();
    let id = env.register(WillContract, ());
    let client = WillContractClient::new(&env, &id);
    StellarAssetClient::new(&env, &token).mint(&owner, &1);
    let will_id = client.create_will(
        &owner,
        &vec![&env, (token, 1_i128)],
        &vec![
            &env,
            Beneficiary {
                address: Address::generate(&env),
                allocation: Allocation::Percentage(10_000),
            },
        ],
        &2,
        &1,
        &Vec::new(&env),
        &0,
        &None,
        &0,
    );
    let scope = ActionLogScope::Will(will_id);
    assert!(client.try_emergency_checkin(&will_id, &owner).is_err());
    assert!(client.get_emergency_actions(&scope, &None, &50).is_empty());

    env.ledger().set_timestamp(4 * DAY);
    client.trigger_will(&will_id, &None);
    client.emergency_checkin(&will_id, &owner);
    let first = client
        .get_emergency_actions(&scope, &None, &1)
        .first()
        .unwrap();
    assert_eq!(first.action, EmergencyAction::OwnerCheckIn);
    assert_eq!(first.actor, Some(owner.clone()));
    assert_eq!(first.timestamp, 4 * DAY);
    assert_eq!(first.sequence, 1);
    client.cancel_will(&will_id, &owner);
    client.archive_will(&will_id);
    assert_eq!(
        client.get_emergency_actions(&scope, &None, &1).first(),
        Some(first)
    );
    let tail = client.get_emergency_actions(&scope, &Some(1), &50);
    assert_eq!(tail.len(), 1);
    assert_eq!(tail.first().unwrap().action, EmergencyAction::ArchiveWill);
    assert!(client
        .get_emergency_actions(&scope, &Some(u64::MAX), &50)
        .is_empty());

    client.repair_protocol_stats();
    let maintenance = client.get_emergency_actions(&ActionLogScope::Protocol, &None, &1);
    assert_eq!(
        maintenance.first().unwrap().action,
        EmergencyAction::RepairProtocolStats
    );
    assert_eq!(maintenance.first().unwrap().actor, None);
}
