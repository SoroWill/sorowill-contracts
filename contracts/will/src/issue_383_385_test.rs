#![cfg(test)]

//! Regression tests for issues #383, #384 and #385.
//!
//! * #385 — the `distribute` doc block used to be attached to
//!   `proportional_share`, leaving `distribute` undocumented and
//!   `proportional_share`'s rustdoc describing something else. Fixed by
//!   splitting the docs; the behaviour asserted here is the one the split
//!   documents.
//! * #384 — `distribute` used to run the fixed-amount payout loop *inside*
//!   the per-token loop, so a `FixedAmount(100)` beneficiary received 100
//!   units of *every* token the will held while validation compared the
//!   fixed total against the sum of all balances. Fixed amounts are now
//!   denominated in the will's primary token only, and validated against
//!   that token's balance.
//! * #383 — a `FixedAmount`-only will with headroom released without
//!   transferring the remainder anywhere, stranding it in the contract. The
//!   remainder is now refunded to the owner.

use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::{Client as TokenClient, StellarAssetClient},
    vec, Address, Env, Vec as SorobanVec,
};

use crate::{Allocation, Beneficiary, WillContract, WillContractClient, WillError, WillStatus};

const DAY: u64 = 86_400;

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

fn advance(env: &Env, days: u64) {
    env.ledger().with_mut(|l| l.timestamp += days * DAY);
}

fn release(env: &Env, client: &WillContractClient, will_id: u64) {
    advance(env, 91);
    client.trigger_will(&will_id);
    advance(env, 8);
    client.release_inheritance(&will_id, &None);
}

/// Issue #384: a `FixedAmount` beneficiary on a two-token will must be paid
/// the fixed amount **once**, in the will's primary token — not once per
/// token. Before the fix this failed with the beneficiary holding
/// 100_000 + 100_000 units instead of 100_000.
#[test]
fn fixed_amount_is_paid_once_in_primary_token() {
    let env = &mut Env::default();
    let (client, owner, primary, primary_address, secondary, secondary_address) =
        setup_two_tokens(env);

    let beneficiary = Address::generate(env);
    let beneficiaries: SorobanVec<Beneficiary> = vec![
        env,
        Beneficiary {
            address: beneficiary.clone(),
            allocation: Allocation::FixedAmount(100_000),
        },
    ];
    // The primary (first) token is the one fixed amounts are denominated in.
    let tokens: SorobanVec<(Address, i128)> = vec![
        env,
        (primary_address.clone(), 1_000_000),
        (secondary_address.clone(), 1_000_000),
    ];

    let will_id = client.create_will(
        &owner,
        &tokens,
        &beneficiaries,
        &90,
        &7,
        &vec![env],
        &2,
        &None,
        &0,
    );
    release(env, &client, will_id);

    assert_eq!(
        primary.balance(&beneficiary),
        100_000,
        "the fixed amount is paid once, in the primary token"
    );
    assert_eq!(
        secondary.balance(&beneficiary),
        0,
        "a fixed amount must not be paid out of a secondary token"
    );
    // The unallocated remainder of both tokens is refunded to the owner
    // rather than stranded in the contract (#383).
    assert_eq!(client.get_will(&will_id).status, WillStatus::Released);
    assert_eq!(primary.balance(&client.address), 0);
    assert_eq!(secondary.balance(&client.address), 0);
}

/// Issue #384: validation must follow the same rule as payout. A
/// `FixedAmount` larger than the primary token's balance must be rejected
/// even when the will holds enough *other* tokens to cover it, since
/// `distribute` can only pay fixed amounts out of the primary token.
#[test]
fn fixed_amount_above_primary_balance_is_rejected() {
    let env = &mut Env::default();
    let (client, owner, _primary, primary_address, _secondary, secondary_address) =
        setup_two_tokens(env);

    let beneficiary = Address::generate(env);
    let beneficiaries: SorobanVec<Beneficiary> = vec![
        env,
        Beneficiary {
            address: beneficiary,
            allocation: Allocation::FixedAmount(1_500_000),
        },
    ];
    // Summed across both tokens this is 2,000,000 — comfortably above the
    // 1,500,000 commitment. The primary token alone is not.
    let tokens: SorobanVec<(Address, i128)> = vec![
        env,
        (primary_address, 1_000_000),
        (secondary_address, 1_000_000),
    ];

    assert_eq!(
        client.try_create_will(
            &owner,
            &tokens,
            &beneficiaries,
            &90,
            &7,
            &vec![env],
            &2,
            &None,
            &0,
        ),
        Err(Ok(WillError::FixedAmountExceedsBalance.into()))
    );
}

/// Issue #383: releasing a `FixedAmount`-only will with headroom must pay the
/// fixed amount to the beneficiary and refund the leftover to the owner,
/// leaving nothing behind in the contract.
#[test]
fn fixed_amount_only_will_refunds_leftover_to_owner() {
    let env = &mut Env::default();
    let (client, owner, primary, primary_address, secondary, secondary_address) =
        setup_two_tokens(env);

    let beneficiary = Address::generate(env);
    let beneficiaries: SorobanVec<Beneficiary> = vec![
        env,
        Beneficiary {
            address: beneficiary.clone(),
            allocation: Allocation::FixedAmount(400_000),
        },
    ];
    // 1,000,000 locked, 400,000 promised: 600,000 of headroom, which the
    // release must refund rather than strand.
    let tokens: SorobanVec<(Address, i128)> = vec![
        env,
        (primary_address.clone(), 1_000_000),
        (secondary_address.clone(), 500_000),
    ];

    let will_id = client.create_will(
        &owner,
        &tokens,
        &beneficiaries,
        &90,
        &7,
        &vec![env],
        &2,
        &None,
        &0,
    );
    let owner_primary_before = primary.balance(&owner);
    let owner_secondary_before = secondary.balance(&owner);
    release(env, &client, will_id);

    assert_eq!(primary.balance(&beneficiary), 400_000);
    assert_eq!(
        primary.balance(&owner),
        owner_primary_before + 600_000,
        "the unallocated primary-token remainder is refunded to the owner"
    );
    assert_eq!(
        secondary.balance(&owner),
        owner_secondary_before + 500_000,
        "a secondary token is never claimed by a fixed amount, so all of it is refunded"
    );
    assert_eq!(
        primary.balance(&client.address) + secondary.balance(&client.address),
        0,
        "nothing may be left stranded in the contract"
    );
    assert_eq!(client.get_will(&will_id).status, WillStatus::Released);
}

/// Issue #383: a will that *does* have percentage beneficiaries must not pay
/// the owner a refund — the last percentage beneficiary already absorbs the
/// whole remainder, so the owner balance is unchanged.
#[test]
fn percentage_will_does_not_refund_to_owner() {
    let env = &mut Env::default();
    let (client, owner, primary, primary_address, secondary, secondary_address) =
        setup_two_tokens(env);

    let fixed = Address::generate(env);
    let percentage = Address::generate(env);
    let beneficiaries: SorobanVec<Beneficiary> = vec![
        env,
        Beneficiary {
            address: fixed.clone(),
            allocation: Allocation::FixedAmount(400_000),
        },
        Beneficiary {
            address: percentage.clone(),
            allocation: Allocation::Percentage(10_000),
        },
    ];
    let tokens: SorobanVec<(Address, i128)> = vec![
        env,
        (primary_address.clone(), 1_000_000),
        (secondary_address, 500_000),
    ];

    let will_id = client.create_will(
        &owner,
        &tokens,
        &beneficiaries,
        &90,
        &7,
        &vec![env],
        &2,
        &None,
        &0,
    );
    let owner_before = primary.balance(&owner);
    release(env, &client, will_id);

    assert_eq!(primary.balance(&fixed), 400_000);
    assert_eq!(primary.balance(&percentage), 600_000);
    assert_eq!(
        secondary.balance(&percentage),
        500_000,
        "the whole secondary balance goes to the percentage split"
    );
    assert_eq!(primary.balance(&owner), owner_before, "no refund is due");
}

/// Issue #384: the doc split is only meaningful if the documented
/// `Allocation` model is the one enforced. A `FixedAmount` of zero is still
/// rejected, and the error surface is unchanged.
#[test]
fn zero_fixed_amount_is_still_rejected() {
    let env = &mut Env::default();
    let (client, owner, _primary, primary_address, _secondary, _secondary_address) =
        setup_two_tokens(env);

    let beneficiary = Address::generate(env);
    let beneficiaries: SorobanVec<Beneficiary> = vec![
        env,
        Beneficiary {
            address: beneficiary,
            allocation: Allocation::FixedAmount(0),
        },
    ];
    let tokens: SorobanVec<(Address, i128)> = vec![env, (primary_address, 1_000_000)];

    assert_eq!(
        client.try_create_will(
            &owner,
            &tokens,
            &beneficiaries,
            &90,
            &7,
            &vec![env],
            &2,
            &None,
            &0,
        ),
        Err(Ok(WillError::InvalidPercentages.into()))
    );
}
