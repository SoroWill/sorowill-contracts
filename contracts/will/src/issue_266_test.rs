#![cfg(test)]

//! Regression test for issue #266: `split_will` renormalises the child's
//! percentage allocations via `renormalize_percentages` but must never save a
//! beneficiary that renormalises down to exactly `Allocation::Percentage(0)`,
//! which would silently drop them from the payout.
//!
//! Issue #377 changed where those allocations come from: the child's list is
//! now built from the SOURCE will's entries and the caller-supplied
//! `Allocation` on each entry of `beneficiaries_to_split` is ignored. A skewed
//! caller-supplied ratio can therefore no longer produce a 0 bp share, and the
//! zero-share invariant is enforced where a 0 bp entry can now only enter the
//! system in the first place — `create_will` / `update_beneficiaries`. Both
//! halves are asserted below so the invariant stays pinned.

use soroban_sdk::{testutils::Address as _, token::StellarAssetClient, vec, Address, Env};

use crate::{Allocation, Beneficiary, WillContract, WillContractClient, WillError};

#[test]
fn create_will_rejects_a_zero_basis_point_beneficiary() {
    let env = Env::default();
    env.mock_all_auths();

    let owner = Address::generate(&env);
    let a = Address::generate(&env);
    let token_address = env
        .register_stellar_asset_contract_v2(owner.clone())
        .address();
    StellarAssetClient::new(&env, &token_address).mint(&owner, &1_000_000);

    let contract_id = env.register(WillContract, ());
    let client = WillContractClient::new(&env, &contract_id);

    // A 0 bp entry is the only way a renormalised share could round to 0 once
    // the child's allocations are copied from the source, so it must be
    // impossible to store one in the first place.
    assert_eq!(
        client.try_create_will(
            &owner,
            &vec![&env, (token_address.clone(), 1_000_000_i128)],
            &vec![
                &env,
                Beneficiary {
                    address: a,
                    allocation: Allocation::Percentage(0)
                },
            ],
            &90,
            &7,
            &vec![&env],
            &1,
            &None,
            &0,
        ),
        Err(Ok(WillError::InvalidPercentages.into())),
        "a 0 bp beneficiary must be rejected at creation time",
    );
}

#[test]
fn split_will_ignores_a_skewed_caller_allocation_and_uses_the_source_entry() {
    let env = Env::default();
    env.mock_all_auths();

    let owner = Address::generate(&env);
    let a = Address::generate(&env);
    let b = Address::generate(&env);
    let c = Address::generate(&env);
    let token_address = env
        .register_stellar_asset_contract_v2(owner.clone())
        .address();
    StellarAssetClient::new(&env, &token_address).mint(&owner, &1_000_000);

    let contract_id = env.register(WillContract, ());
    let client = WillContractClient::new(&env, &contract_id);

    // Three beneficiaries; the caller's split allocations are irrelevant to the
    // child since #377, which takes them from these source entries instead.
    let will_id = client.create_will(
        &owner,
        &vec![&env, (token_address.clone(), 1_000_000_i128)],
        &vec![
            &env,
            Beneficiary {
                address: a.clone(),
                allocation: Allocation::Percentage(100),
            },
            Beneficiary {
                address: b.clone(),
                allocation: Allocation::Percentage(100),
            },
            Beneficiary {
                address: c.clone(),
                allocation: Allocation::Percentage(9_800),
            },
        ],
        &90,
        &7,
        &vec![&env],
        &1,
        &None,
        &0,
    );

    // Split off [a, b] with a deliberately skewed ratio: renormalising
    // 1 : 999_999 would rescale `a`'s share to floor(1 * 10_000 / 1_000_000) = 0.
    // That ratio is ignored: the child is built from the source's 100 : 100.
    let skewed_split = vec![
        &env,
        Beneficiary {
            address: a,
            allocation: Allocation::Percentage(1),
        },
        Beneficiary {
            address: b,
            allocation: Allocation::Percentage(999_999),
        },
    ];

    let child_id = client.split_will(
        &will_id,
        &owner,
        &skewed_split,
        &vec![&env, (token_address, 100_000_i128)],
    );

    // Neither child entry rounds to zero: the source's equal 100 bp shares
    // renormalise to an even 5,000 / 5,000.
    let child = client.get_will(&child_id);
    assert_eq!(child.beneficiaries.len(), 2);
    for b in child.beneficiaries.iter() {
        match b.allocation {
            Allocation::Percentage(bp) => {
                assert!(bp > 0, "a renormalised share of 0 bp must never be saved")
            }
            Allocation::FixedAmount(_) => panic!("expected a percentage allocation"),
        }
    }
    let total_bp = child
        .beneficiaries
        .iter()
        .fold(0u32, |sum, b| match b.allocation {
            Allocation::Percentage(bp) => sum + bp,
            Allocation::FixedAmount(_) => sum,
        });
    assert_eq!(
        total_bp, 10_000,
        "renormalised child percentages must sum to 10,000 bps"
    );
}
