#![cfg(test)]

//! Regression test for issue #362: renouncing the **only** `Percentage`
//! beneficiary of a will that also has `FixedAmount` beneficiaries.
//!
//! `renounce_beneficiary` redistributes a renounced `Percentage` share across
//! the remaining percentage-based beneficiaries. When there are none left --
//! `remaining_basis_points` is 0, so the `remaining_basis_points > 0` branch is
//! skipped and the shortened list is assigned as-is -- the renounced basis
//! points are no longer claimed by any visible beneficiary.
//!
//! The decided behaviour (documented on `renounce_beneficiary` and at the
//! branch itself) is that the renunciation is **accepted** and the share is
//! **returned to the owner** at release, via `distribute`'s existing refund
//! path for a `FixedAmount`-only list with headroom (#383). Nothing is stranded
//! in the contract and no remaining beneficiary is over-paid.
//!
//! These tests pin the part `renounce_beneficiary` itself decides: the
//! renunciation is accepted (including while `Triggered`), the surviving
//! `FixedAmount` claim is left exactly as it was rather than absorbing the
//! renounced share, and the corresponding balance stays in the will -- which is
//! precisely the unclaimed headroom that later routes to the owner.

use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::{Client as TokenClient, StellarAssetClient},
    vec, Address, Env,
};

use crate::{Allocation, Beneficiary, WillContract, WillContractClient, WillStatus};

const DAY: u64 = 86_400;

/// A will with one `FixedAmount` and one `Percentage` beneficiary, triggered
/// and ready to be released. Returns `(env, client, owner, token_client,
/// token_address, fixed_beneficiary, percentage_beneficiary, will_id)`.
#[allow(clippy::type_complexity)]
fn setup_mixed_will<'a>() -> (
    Env,
    WillContractClient<'a>,
    Address,
    TokenClient<'a>,
    Address,
    Address,
    Address,
    u64,
) {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_700_000_000);

    let owner = Address::generate(&env);
    let sac = env
        .clone()
        .register_stellar_asset_contract_v2(owner.clone());
    let token_address = sac.address();
    StellarAssetClient::new(&env, &token_address).mint(&owner, &1_000_000);

    let contract_id = env.clone().register(WillContract, ());
    let client = WillContractClient::new(&env, &contract_id);

    let fixed_beneficiary = Address::generate(&env);
    let percentage_beneficiary = Address::generate(&env);

    let will_id = client.create_will(
        &owner,
        &vec![&env, (token_address.clone(), 1_000_000_i128)],
        &vec![
            &env,
            Beneficiary {
                address: fixed_beneficiary.clone(),
                allocation: Allocation::FixedAmount(300_000),
            },
            // The only percentage entry: 100% of whatever the fixed amount
            // leaves behind.
            Beneficiary {
                address: percentage_beneficiary.clone(),
                allocation: Allocation::Percentage(10_000),
            },
        ],
        &90,
        &7,
        &vec![&env],
        &0,
        &None,
        &0,
    );

    // Miss the check-in deadline, then let the grace period elapse so the will
    // is `Triggered` and releasable.
    env.ledger().with_mut(|l| l.timestamp += 91 * DAY);
    client.trigger_will(&will_id);
    env.ledger().with_mut(|l| l.timestamp += 8 * DAY);

    let token_client = TokenClient::new(&env, &token_address);
    (
        env,
        client,
        owner,
        token_client,
        token_address,
        fixed_beneficiary,
        percentage_beneficiary,
        will_id,
    )
}

/// The shortened list keeps the remaining `FixedAmount` beneficiary's claim
/// exactly as it was: the renounced basis points are not folded into it.
#[test]
fn renouncing_the_last_percentage_beneficiary_leaves_the_fixed_claim_untouched() {
    let (env, client, _owner, _token, _token_address, _fixed, percentage, will_id) =
        setup_mixed_will();

    client.renounce_beneficiary(&will_id, &percentage);

    let will = client.get_will(&will_id);
    assert_eq!(will.beneficiaries.len(), 1);
    let remaining = will.beneficiaries.get(0).unwrap();
    assert!(
        matches!(remaining.allocation, Allocation::FixedAmount(300_000)),
        "the fixed claim must be unchanged, not inflated by the renounced share"
    );
}

/// The renunciation is accepted while the will is `Triggered` too: the will has
/// not been released yet, so the shortened list is exactly what the eventual
/// distribution will read.
#[test]
fn renouncing_the_last_percentage_beneficiary_works_while_triggered() {
    let (env, client, _owner, _token, _token_address, _fixed, percentage, will_id) =
        setup_mixed_will();

    assert_eq!(client.get_will(&will_id).status, WillStatus::Triggered);

    client.renounce_beneficiary(&will_id, &percentage);

    let will = client.get_will(&will_id);
    assert_eq!(
        will.status,
        WillStatus::Triggered,
        "renouncing is not a status change"
    );
    assert_eq!(will.beneficiaries.len(), 1);
    assert!(matches!(
        will.beneficiaries.get(0).unwrap().allocation,
        Allocation::FixedAmount(300_000)
    ));
}

/// The renounced basis points are not reassigned to the surviving beneficiary,
/// and the will's locked balance is untouched by the renunciation itself: the
/// share stays in the will and is what `distribute` later returns to the owner
/// once no percentage beneficiary is left to absorb it (#383).
#[test]
fn the_renounced_share_is_left_unallocated_in_the_will() {
    let (env, client, _owner, _token, token_address, fixed, percentage, will_id) =
        setup_mixed_will();

    let balance_before = client
        .get_will(&will_id)
        .balances
        .get(token_address.clone())
        .unwrap();

    client.renounce_beneficiary(&will_id, &percentage);

    let will = client.get_will(&will_id);
    assert_eq!(
        will.balances.get(token_address).unwrap(),
        balance_before,
        "renouncing must not move any funds; the unclaimed share stays in the will"
    );
    let survivor = will.beneficiaries.get(0).unwrap();
    assert_eq!(survivor.address, fixed);
    assert!(
        matches!(survivor.allocation, Allocation::FixedAmount(300_000)),
        "the surviving fixed claim absorbs none of the renounced share"
    );
    // No percentage entry is left, which is what routes the unclaimed headroom
    // to the owner refund in `distribute` rather than to a beneficiary.
    assert!(will
        .beneficiaries
        .iter()
        .all(|b| matches!(b.allocation, Allocation::FixedAmount(_))));
}
