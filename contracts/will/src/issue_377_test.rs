#![cfg(test)]

//! Regression tests for issue #377: the code comment in `split_will` said it
//! built a set to verify the split beneficiaries exist in the source, but it
//! only filtered the source list. Any address and allocation the owner passed
//! in became a beneficiary of the child even when it was never on the source,
//! the child list was not checked against `MAX_BENEFICIARIES`, and the child's
//! allocations came from the caller rather than from the source entries.
//!
//! `split_will` now rejects a non-member address with
//! `WillError::BeneficiaryNotFound`, takes each allocation from the source
//! entry, bounds the child list by `MAX_BENEFICIARIES`, and rejects a repeated
//! address with `WillError::DuplicateBeneficiary`.

use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::StellarAssetClient,
    vec, Address, Env, Vec,
};

use crate::{Allocation, Beneficiary, WillContract, WillContractClient, MAX_BENEFICIARIES};

const FUNDING: i128 = 10_000_000;

/// Creates a funded source will whose beneficiaries hold `shares` basis points
/// each, in order. Returns the env, the client, the owner, the token address,
/// the beneficiary addresses and the will id.
fn setup(
    shares: &[u32],
) -> (
    Env,
    WillContractClient<'static>,
    Address,
    Address,
    Vec<Address>,
    u64,
) {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_700_000_000);

    let owner = Address::generate(&env);
    let token_address = env
        .register_stellar_asset_contract_v2(owner.clone())
        .address();
    StellarAssetClient::new(&env, &token_address).mint(&owner, &FUNDING);

    let contract_id = env.register(WillContract, ());
    let client = WillContractClient::new(&env, &contract_id);

    let mut addresses: Vec<Address> = Vec::new(&env);
    let mut beneficiaries: Vec<Beneficiary> = Vec::new(&env);
    for share in shares {
        let address = Address::generate(&env);
        addresses.push_back(address.clone());
        beneficiaries.push_back(Beneficiary {
            address,
            allocation: Allocation::Percentage(*share),
        });
    }

    let will_id = client.create_will(
        &owner,
        &vec![&env, (token_address.clone(), FUNDING)],
        &beneficiaries,
        &90,
        &7,
        &vec![&env],
        &2,
        &None,
        &0,
    );

    (env, client, owner, token_address, addresses, will_id)
}

#[test]
fn split_rejects_an_address_that_is_not_a_beneficiary_of_the_source() {
    let (env, client, owner, token_address, beneficiaries, will_id) = setup(&[6_000, 4_000]);
    let stranger = Address::generate(&env);

    assert_eq!(
        client.try_split_will(
            &will_id,
            &owner,
            &vec![
                &env,
                Beneficiary {
                    address: stranger.clone(),
                    allocation: Allocation::Percentage(10_000),
                },
            ],
            &vec![&env, (token_address, 100_000_i128)],
        ),
        Err(Ok(crate::WillError::BeneficiaryNotFound.into())),
        "an address that was never a beneficiary of the source must be rejected"
    );

    // Nothing changed: the stranger is not a beneficiary of either will, and
    // the source keeps both its beneficiaries and its balance.
    let source = client.get_will(&will_id);
    assert_eq!(source.beneficiaries.len(), 2);
    assert_eq!(source.balance, FUNDING);
    assert!(client
        .get_wills_by_beneficiary(&stranger, &None, &10)
        .is_empty());
    assert_eq!(beneficiaries.len(), 2);
}

#[test]
fn split_rejects_a_partly_foreign_list_without_side_effects() {
    let (env, client, owner, token_address, beneficiaries, will_id) = setup(&[6_000, 4_000]);

    // The first address is a real member, the second is not: the whole request
    // is refused, so no partial child will is created.
    assert_eq!(
        client.try_split_will(
            &will_id,
            &owner,
            &vec![
                &env,
                Beneficiary {
                    address: beneficiaries.get(0).unwrap(),
                    allocation: Allocation::Percentage(6_000),
                },
                Beneficiary {
                    address: Address::generate(&env),
                    allocation: Allocation::Percentage(4_000),
                },
            ],
            &vec![&env, (token_address, 100_000_i128)],
        ),
        Err(Ok(crate::WillError::BeneficiaryNotFound.into())),
    );

    let source = client.get_will(&will_id);
    assert_eq!(source.beneficiaries.len(), 2);
    assert_eq!(source.balance, FUNDING);
    assert_eq!(client.get_wills_by_owner(&owner, &None, &10).len(), 1);
}

#[test]
fn child_allocation_comes_from_the_source_entry_not_the_caller() {
    // Three beneficiaries so a two-way split leaves the source with one.
    let (env, client, owner, token_address, beneficiaries, will_id) = setup(&[5_000, 3_000, 2_000]);

    // The caller asks for a 100% / 0% split of [a, b]; the source records
    // 5_000 / 3_000, and it is the source's ratio that must be renormalised
    // (5_000 : 3_000 -> 6_250 : 3_750), not the caller's.
    let child_id = client.split_will(
        &will_id,
        &owner,
        &vec![
            &env,
            Beneficiary {
                address: beneficiaries.get(0).unwrap(),
                allocation: Allocation::Percentage(10_000),
            },
            Beneficiary {
                address: beneficiaries.get(1).unwrap(),
                allocation: Allocation::Percentage(0),
            },
        ],
        &vec![&env, (token_address, 200_000_i128)],
    );

    let child = client.get_will(&child_id);
    let allocations: std::vec::Vec<u32> = child
        .beneficiaries
        .iter()
        .map(|b| match b.allocation {
            Allocation::Percentage(bp) => bp,
            Allocation::FixedAmount(_) => panic!("expected a percentage allocation"),
        })
        .collect();
    assert_eq!(
        allocations,
        std::vec![6_250_u32, 3_750_u32],
        "the child must renormalise the source's 5_000 : 3_000, not the caller's 10_000 : 0"
    );

    // The single beneficiary left on the source takes the whole remaining pot.
    let source = client.get_will(&will_id);
    assert_eq!(source.beneficiaries.len(), 1);
    assert_eq!(
        source.beneficiaries.get(0).unwrap().address,
        beneficiaries.get(2).unwrap()
    );
    assert_eq!(
        source.beneficiaries.get(0).unwrap().allocation,
        Allocation::Percentage(10_000)
    );
}

#[test]
fn split_rejects_a_list_longer_than_max_beneficiaries() {
    // MAX_BENEFICIARIES (10) beneficiaries of 1_000 bp each, i.e. a source
    // that is exactly full.
    let shares: std::vec::Vec<u32> = (0..MAX_BENEFICIARIES).map(|_| 1_000).collect();
    let (env, client, owner, token_address, beneficiaries, will_id) = setup(&shares);

    // A source can hold at most MAX_BENEFICIARIES beneficiaries, so an oversized
    // request necessarily repeats one of them. The cap is checked before the
    // membership and duplicate checks, so this is a size failure and not a
    // duplicate one.
    let mut oversized: Vec<Beneficiary> = Vec::new(&env);
    for i in 0..MAX_BENEFICIARIES {
        oversized.push_back(Beneficiary {
            address: beneficiaries.get(i).unwrap(),
            allocation: Allocation::Percentage(1_000),
        });
    }
    oversized.push_back(Beneficiary {
        address: beneficiaries.get(0).unwrap(),
        allocation: Allocation::Percentage(1_000),
    });
    assert_eq!(oversized.len(), MAX_BENEFICIARIES + 1);

    assert_eq!(
        client.try_split_will(
            &will_id,
            &owner,
            &oversized,
            &vec![&env, (token_address, 100_000_i128)],
        ),
        Err(Ok(crate::WillError::TooManyBeneficiaries.into())),
        "a child list longer than MAX_BENEFICIARIES must be rejected"
    );

    // Nothing was created or moved.
    let source = client.get_will(&will_id);
    assert_eq!(source.beneficiaries.len(), MAX_BENEFICIARIES);
    assert_eq!(source.balance, FUNDING);
    assert_eq!(client.get_wills_by_owner(&owner, &None, &10).len(), 1);
}

#[test]
fn split_rejects_a_repeated_address() {
    let (env, client, owner, token_address, beneficiaries, will_id) = setup(&[5_000, 3_000, 2_000]);
    let repeated = beneficiaries.get(0).unwrap();

    assert_eq!(
        client.try_split_will(
            &will_id,
            &owner,
            &vec![
                &env,
                Beneficiary {
                    address: repeated.clone(),
                    allocation: Allocation::Percentage(5_000),
                },
                // The same address a second time: without a duplicate check this
                // would collapse in the filter and leave the source and the
                // child disagreeing about how many beneficiaries moved.
                Beneficiary {
                    address: repeated.clone(),
                    allocation: Allocation::Percentage(5_000),
                },
            ],
            &vec![&env, (token_address, 100_000_i128)],
        ),
        Err(Ok(crate::WillError::DuplicateBeneficiary.into())),
    );

    let source = client.get_will(&will_id);
    assert_eq!(source.beneficiaries.len(), 3);
    assert_eq!(source.balance, FUNDING);
}
