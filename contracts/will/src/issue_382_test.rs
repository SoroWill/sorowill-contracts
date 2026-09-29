#![cfg(test)]

//! Regression test for issue #382: `merge_beneficiaries` derived percentage
//! shares from `will.balance` — the legacy primary-token mirror — so any other
//! token held by a multi-token will was ignored when weighting the merged
//! percentages. The multiplication was also unchecked while the rest of the
//! contract uses `proportional_share`, an `_allocation` clone was dead, and
//! both accumulator `Vec`s were rebuilt for every beneficiary.

use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::{Client as TokenClient, StellarAssetClient},
    vec, Address, Env, Vec as SorobanVec,
};

use crate::{Allocation, Beneficiary, WillContract, WillContractClient};

/// Mints `amount` of a fresh SAC to `owner` and returns its address.
fn mint(env: &Env, owner: &Address, amount: i128) -> Address {
    let sac = env.register_stellar_asset_contract_v2(owner.clone());
    let token_address = sac.address();
    StellarAssetClient::new(env, &token_address).mint(owner, &amount);
    token_address
}

/// Registers the contract and mints two independent tokens for `owner`, so
/// tests can pin down the primary-token (`tokens[0]`) semantics that #384
/// established for `Allocation::FixedAmount`.
fn setup_two_tokens<'a>(
    env: &Env,
) -> (
    WillContractClient<'a>,
    Address,
    TokenClient<'a>,
    Address,
    TokenClient<'a>,
    Address,
) {
    env.mock_all_auths();
    env.ledger().set_timestamp(1_700_000_000);

    let owner = Address::generate(env);
    let primary_address = mint(env, &owner, 10_000_000_000);
    let secondary_address = mint(env, &owner, 10_000_000_000);

    let contract_id = env.register(WillContract, ());
    let client = WillContractClient::new(env, &contract_id);

    (
        client,
        owner,
        TokenClient::new(env, &primary_address),
        primary_address,
        TokenClient::new(env, &secondary_address),
        secondary_address,
    )
}

/// Issue #382: `merge_beneficiaries` must weigh each will by its combined
/// value across *all* of its tokens, not by the legacy primary-token mirror
/// `Will::balance`.
///
/// Will A holds 1,000 of the primary token and 9,000 of a secondary one, all
/// for `a_beneficiary`; will B holds 9,000 of the primary token only, for
/// `b_beneficiary`. The true combined value is 19,000, so the merged shares
/// must be 10,000/19,000 and 9,000/19,000 of it. Before the fix A's secondary
/// holding was ignored entirely and A was weighted at just 1,000, giving it
/// 1,000 bps instead of 5,263.
#[test]
fn merge_weights_all_tokens_not_just_the_primary_mirror() {
    let env = &mut Env::default();
    let (client, owner, _primary, primary_address, _secondary, secondary_address) =
        setup_two_tokens(env);

    let a_beneficiary = Address::generate(env);
    let b_beneficiary = Address::generate(env);

    let a_beneficiaries: SorobanVec<Beneficiary> = vec![
        env,
        Beneficiary {
            address: a_beneficiary.clone(),
            allocation: Allocation::Percentage(10_000),
        },
    ];
    let a_tokens: SorobanVec<(Address, i128)> = vec![
        env,
        (primary_address.clone(), 1_000),
        (secondary_address, 9_000),
    ];
    let b_beneficiaries: SorobanVec<Beneficiary> = vec![
        env,
        Beneficiary {
            address: b_beneficiary.clone(),
            allocation: Allocation::Percentage(10_000),
        },
    ];
    let b_tokens: SorobanVec<(Address, i128)> = vec![env, (primary_address, 9_000)];

    let will_a = client.create_will(
        &owner,
        &a_tokens,
        &a_beneficiaries,
        &90,
        &7,
        &vec![env],
        &2,
        &None,
        &0,
    );
    let will_b = client.create_will(
        &owner,
        &b_tokens,
        &b_beneficiaries,
        &90,
        &7,
        &vec![env],
        &2,
        &None,
        &0,
    );
    client.merge_wills(&owner, &will_a, &will_b);

    let merged = client.get_will(&will_a);
    let bp = |addr: &Address| -> u32 {
        match merged
            .beneficiaries
            .iter()
            .find(|b| &b.address == addr)
            .expect("beneficiary present after merge")
            .allocation
        {
            Allocation::Percentage(bp) => bp,
            other => panic!("expected a Percentage allocation, got {other:?}"),
        }
    };

    // floor(10_000 * 10_000 / 19_000) = 5,263, and the 1 bp rounding
    // remainder lands on the last percentage entry.
    assert_eq!(bp(&a_beneficiary), 5_263, "A weighs 10,000 of 19,000");
    assert_eq!(bp(&b_beneficiary), 4_737, "B weighs 9,000 of 19,000");
}

/// Issue #382 (dead variable): a beneficiary present in both wills with a
/// `FixedAmount` in each must still be summed after the accumulator rewrite,
/// and must appear exactly once.
#[test]
fn merge_still_sums_fixed_amounts_without_duplicating_entries() {
    let env = &mut Env::default();
    let (client, owner, _primary, primary_address, _secondary, secondary_address) =
        setup_two_tokens(env);

    let fixed = Address::generate(env);
    let beneficiaries_a: SorobanVec<Beneficiary> = vec![
        env,
        Beneficiary {
            address: fixed.clone(),
            allocation: Allocation::FixedAmount(300_000),
        },
    ];
    let beneficiaries_b: SorobanVec<Beneficiary> = vec![
        env,
        Beneficiary {
            address: fixed.clone(),
            allocation: Allocation::FixedAmount(200_000),
        },
    ];
    let tokens_a: SorobanVec<(Address, i128)> = vec![
        env,
        (primary_address.clone(), 400_000),
        (secondary_address, 100_000),
    ];
    let tokens_b: SorobanVec<(Address, i128)> = vec![env, (primary_address, 400_000)];

    let will_a = client.create_will(
        &owner,
        &tokens_a,
        &beneficiaries_a,
        &90,
        &7,
        &vec![env],
        &2,
        &None,
        &0,
    );
    let will_b = client.create_will(
        &owner,
        &tokens_b,
        &beneficiaries_b,
        &90,
        &7,
        &vec![env],
        &2,
        &None,
        &0,
    );
    client.merge_wills(&owner, &will_a, &will_b);

    let merged = client.get_will(&will_a);
    assert_eq!(merged.beneficiaries.len(), 1, "one entry per address");
    match merged.beneficiaries.get_unchecked(0).allocation {
        Allocation::FixedAmount(amt) => assert_eq!(amt, 500_000, "300_000 + 200_000"),
        other => panic!("expected a summed FixedAmount, got {other:?}"),
    }
}
