#![cfg(test)]

//! Regression tests for issue #375: `clone_will` and `split_will` copied
//! `source.guardians` verbatim, including each guardian's consent value. A
//! guardian who accepted the role on the source will was therefore already
//! `Accepted` on a brand-new will they were never asked about, while
//! `batch_create_wills` starts every guardian as `Pending`.
//!
//! Both functions now reset consent to `Pending` and preserve weights, so a
//! guardian must be asked about the new will before they can vote on it.

use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::StellarAssetClient,
    vec, Address, Env,
};

use crate::{
    Allocation, Beneficiary, GuardianConsent, GuardianSpec, GuardianVoteReason, WillContract,
    WillContractClient,
};

const DAY: u64 = 86_400;

/// Creates a funded source will with two beneficiaries and two guardians, the
/// first of which has already accepted. Returns the env, the client, the owner,
/// the token address, the two beneficiaries and the source will id.
fn setup() -> (
    Env,
    WillContractClient<'static>,
    Address,
    Address,
    Address,
    Address,
    u64,
) {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_700_000_000);

    let owner = Address::generate(&env);
    let token_address = env
        .register_stellar_asset_contract_v2(owner.clone())
        .address();
    StellarAssetClient::new(&env, &token_address).mint(&owner, &10_000_000);

    let contract_id = env.register(WillContract, ());
    let client = WillContractClient::new(&env, &contract_id);

    let beneficiary_a = Address::generate(&env);
    let beneficiary_b = Address::generate(&env);
    let source_id = client.create_will(
        &owner,
        &vec![&env, (token_address.clone(), 1_000_000_i128)],
        &vec![
            &env,
            Beneficiary {
                address: beneficiary_a.clone(),
                allocation: Allocation::Percentage(6_000),
            },
            Beneficiary {
                address: beneficiary_b.clone(),
                allocation: Allocation::Percentage(4_000),
            },
        ],
        &90,
        &14,
        &vec![&env, Address::generate(&env), Address::generate(&env)],
        &2,
        &None,
        &0,
    );

    // The first guardian accepts on the source; the second one declines.
    let first = client
        .get_will(&source_id)
        .guardians
        .get(0)
        .unwrap()
        .address;
    let second = client
        .get_will(&source_id)
        .guardians
        .get(1)
        .unwrap()
        .address;
    client.accept_guardian_role(&source_id, &first);
    client.reject_guardian_role(&source_id, &second);

    (
        env,
        client,
        owner,
        token_address,
        beneficiary_a,
        beneficiary_b,
        source_id,
    )
}

/// Returns the consent recorded for `guardian` on `will_id`.
fn consent_of(client: &WillContractClient, will_id: u64, guardian: &Address) -> GuardianConsent {
    client
        .get_will(&will_id)
        .guardians
        .iter()
        .find(|g| g.address == *guardian)
        .expect("guardian is on the will")
        .consent
}

#[test]
fn clone_resets_guardian_consent_to_pending_and_preserves_weights() {
    let (env, client, owner, token_address, _a, _b, source_id) = setup();
    let first = client
        .get_will(&source_id)
        .guardians
        .get(0)
        .unwrap()
        .address;
    let second = client
        .get_will(&source_id)
        .guardians
        .get(1)
        .unwrap()
        .address;

    // Give the source a non-uniform weighting so the preservation of weights
    // is actually asserted rather than trivially satisfied by all-1s.
    client.update_guardians_weighted(
        &source_id,
        &owner,
        &vec![
            &env,
            GuardianSpec {
                address: first.clone(),
                weight: 2,
            },
            GuardianSpec {
                address: second.clone(),
                weight: 5,
            },
        ],
        &Some(3),
    );
    client.accept_guardian_role(&source_id, &first);
    client.reject_guardian_role(&source_id, &second);

    let clone_id = client.clone_will(
        &source_id,
        &owner,
        &vec![&env, (token_address, 500_000_i128)],
    );

    let cloned = client.get_will(&clone_id);
    assert_eq!(cloned.guardians.len(), 2);
    // Addresses and weights are preserved ...
    assert_eq!(cloned.guardians.get(0).unwrap().address, first);
    assert_eq!(cloned.guardians.get(0).unwrap().weight, 2);
    assert_eq!(cloned.guardians.get(1).unwrap().address, second);
    assert_eq!(cloned.guardians.get(1).unwrap().weight, 5);
    // ... but neither consent carries over.
    assert_eq!(
        consent_of(&client, clone_id, &first),
        GuardianConsent::Pending
    );
    assert_eq!(
        consent_of(&client, clone_id, &second),
        GuardianConsent::Pending
    );
    assert_eq!(cloned.guardian_threshold, 3);
}

#[test]
#[should_panic(expected = "Error(Contract, #38)")]
fn a_source_guardian_cannot_vote_on_the_clone_before_accepting() {
    let (env, client, owner, token_address, _a, _b, source_id) = setup();
    let first = client
        .get_will(&source_id)
        .guardians
        .get(0)
        .unwrap()
        .address;
    let second = client
        .get_will(&source_id)
        .guardians
        .get(1)
        .unwrap()
        .address;

    // Both guardians accept on the source: both could vote there.
    client.accept_guardian_role(&source_id, &first);
    client.accept_guardian_role(&source_id, &second);

    let clone_id = client.clone_will(
        &source_id,
        &owner,
        &vec![&env, (token_address, 500_000_i128)],
    );

    // The clone's guardian-list cooldown has to elapse before a vote is allowed.
    env.ledger().with_mut(|l| l.timestamp += 8 * DAY);
    // `GuardianNotConsented`: the source's acceptance does not carry over.
    client.guardian_trigger(&clone_id, &first, &GuardianVoteReason::Deceased);
}

#[test]
fn split_resets_guardian_consent_to_pending_and_preserves_weights() {
    let (env, client, owner, token_address, beneficiary_a, _b, source_id) = setup();
    let first = client
        .get_will(&source_id)
        .guardians
        .get(0)
        .unwrap()
        .address;
    let second = client
        .get_will(&source_id)
        .guardians
        .get(1)
        .unwrap()
        .address;

    client.update_guardians_weighted(
        &source_id,
        &owner,
        &vec![
            &env,
            GuardianSpec {
                address: first.clone(),
                weight: 3,
            },
            GuardianSpec {
                address: second.clone(),
                weight: 4,
            },
        ],
        &Some(3),
    );
    client.accept_guardian_role(&source_id, &first);
    client.reject_guardian_role(&source_id, &second);

    let child_id = client.split_will(
        &source_id,
        &owner,
        &vec![
            &env,
            Beneficiary {
                address: beneficiary_a,
                allocation: Allocation::Percentage(6_000),
            },
        ],
        &vec![&env, (token_address, 250_000_i128)],
    );

    let child = client.get_will(&child_id);
    assert_eq!(child.guardians.get(0).unwrap().address, first);
    assert_eq!(child.guardians.get(0).unwrap().weight, 3);
    assert_eq!(child.guardians.get(1).unwrap().address, second);
    assert_eq!(child.guardians.get(1).unwrap().weight, 4);
    assert_eq!(
        consent_of(&client, child_id, &first),
        GuardianConsent::Pending
    );
    assert_eq!(
        consent_of(&client, child_id, &second),
        GuardianConsent::Pending
    );
    assert_eq!(child.guardian_threshold, 3);
}
