#![cfg(test)]

//! Compiled version of the `create_will` rustdoc example (#366).
//!
//! The example in `create_will`'s rustdoc is tagged ```` ```ignore ````, so
//! `cargo test` never compiled it and it silently drifted away from the real
//! API — it still built a `Beneficiary` with the removed `basis_points` field
//! long after the struct moved to `{ address, allocation }`.
//!
//! This module is that same example as a real test. It is deliberately a
//! near-verbatim copy of the rustdoc snippet: when the `create_will` signature
//! or the `Beneficiary` / `Allocation` types change, the rustdoc drifts and this
//! file stops compiling, which is exactly the signal the `ignore` tag removed.
//!
//! If you change the rustdoc example, change it here too, and vice versa.

use soroban_sdk::{testutils::Address as _, token::StellarAssetClient, vec, Address, Env};

use crate::{Allocation, Beneficiary, WillContract, WillContractClient, WillStatus};

/// The compiled twin of `create_will`'s `# Examples` block.
#[test]
fn doc_example_creates_a_single_beneficiary_will() {
    // Set up the environment and register the contract (test harness only).
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register(WillContract, ());
    let client = WillContractClient::new(&env, &contract_id);

    // Mint some USDC to the owner via a Stellar Asset Contract.
    let owner = Address::generate(&env);
    let usdc_id = env
        .register_stellar_asset_contract_v2(owner.clone())
        .address();
    StellarAssetClient::new(&env, &usdc_id).mint(&owner, &1_000_000);

    let beneficiary = Address::generate(&env);

    // Create a will: lock 1 USDC, a single percentage beneficiary taking the
    // whole balance, 90-day check-in, 7-day grace period, no guardians.
    let will_id = client.create_will(
        &owner,
        &vec![&env, (usdc_id.clone(), 1_000_000_i128)],
        &vec![
            &env,
            Beneficiary {
                address: beneficiary.clone(),
                // A percentage allocation is in basis points, and all
                // percentage allocations together must sum to 10_000.
                allocation: Allocation::Percentage(10_000),
            },
        ],
        &90,         // checkin_period_days
        &7,          // grace_period_days
        &vec![&env], // no guardians
        &1,          // guardian_threshold (ignored when no guardians)
        &None,       // no keeper bounty
        &0,          // confirmation_delay_seconds (0 = starts Active immediately)
    );

    let will = client.get_will(&will_id);
    assert_eq!(will.owner, owner);
    assert_eq!(will.status, WillStatus::Active);

    // And the allocation really is stored as a percentage, in basis points —
    // the shape the rustdoc example now claims.
    let stored = will.beneficiaries.get(0).unwrap();
    assert_eq!(stored.address, beneficiary);
    assert_eq!(stored.allocation, Allocation::Percentage(10_000));
}
