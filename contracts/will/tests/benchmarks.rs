//! Run with `cargo test -p will --test benchmarks -- --nocapture`.
use soroban_sdk::{
    testutils::{Address as _, IssuerFlags, Ledger},
    token::StellarAssetClient,
    vec,
    xdr::ToXdr,
    Address, Bytes, Env, Vec,
};
use will::{
    ActionLogScope, Allocation, Beneficiary, GuardianSpec, GuardianVoteReason, WillContract,
    WillContractClient, WillStatus, MAX_BENEFICIARIES,
};

const DAY: u64 = 86_400;

#[test]
fn profile_public_entry_points() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(DAY);
    let owner = Address::generate(&env);
    let first = Address::generate(&env);
    let second = Address::generate(&env);
    let asset = env.register_stellar_asset_contract_v2(owner.clone());
    asset.issuer().set_flag(IssuerFlags::RevocableFlag);
    let token = asset.address();
    let asset_client = StellarAssetClient::new(&env, &token);
    let id = env.register(WillContract, ());
    let client = WillContractClient::new(&env, &id);
    asset_client.mint(&owner, &100_000_000);
    let funds = vec![&env, (token.clone(), 1_000_000_i128)];
    let names = vec![&env, first.clone(), second.clone()];
    let people = beneficiaries(&env);
    let mut measured = std::collections::BTreeSet::new();

    println!("| Function | Instructions | Memory reads | Disk reads | Writes | Write bytes | Fee (stroops) |");
    println!("| --- | ---: | ---: | ---: | ---: | ---: | ---: |");
    macro_rules! profile {
        ($function:ident($($arg:expr),* $(,)?)) => {{
            let result = client.$function($($arg),*);
            let cost = env.cost_estimate();
            let resources = cost.resources();
            let name = stringify!($function);
            measured.insert(name.strip_prefix("try_").unwrap_or(name));
            println!(
                "| {name} | {} | {} | {} | {} | {} | {} |",
                resources.instructions,
                resources.memory_read_entries,
                resources.disk_read_entries,
                resources.write_entries,
                resources.write_bytes,
                cost.fee().total,
            );
            result
        }};
    }

    let main = profile!(create_will(
        &owner, &funds, &people, &90, &7, &names, &2, &None, &600
    ));
    profile!(confirm_will(&main, &owner));
    profile!(check_in(&main, &owner));
    profile!(set_delegate(&main, &owner, &Some(Address::generate(&env))));
    profile!(top_up(&main, &owner, &token, &50));
    profile!(update_beneficiaries(&main, &owner, &beneficiaries(&env)));
    profile!(update_guardians(&main, &owner, &names));
    let weighted = vec![
        &env,
        GuardianSpec {
            address: first.clone(),
            weight: 1,
        },
        GuardianSpec {
            address: second.clone(),
            weight: 2,
        },
    ];
    profile!(update_guardians_weighted(
        &main,
        &owner,
        &weighted,
        &Some(2)
    ));
    profile!(set_guardian_cooldown(&main, &owner, &8));
    profile!(set_guardian_vote_expiry(&main, &owner, &Some(5)));
    profile!(update_periods(&main, &owner, &Some(91), &None));
    profile!(update_will_settings(
        &main,
        &owner,
        &None,
        &None,
        &Some(90),
        &None
    ));
    profile!(reject_guardian_role(&main, &second));
    profile!(accept_guardian_role(&main, &first));

    let batch = profile!(batch_create_wills(
        &owner,
        &vec![
            &env,
            (funds.clone(), people.clone(), 90, 7, Vec::new(&env), 0)
        ],
    ));
    profile!(batch_check_in(&batch, &owner));
    profile!(batch_check_in_each(&batch, &owner));
    let copy = profile!(clone_will(&main, &owner, &funds));
    profile!(merge_wills(&owner, &main, &copy));
    let child = profile!(split_will(
        &main,
        &owner,
        &vec![&env, client.get_will(&main).beneficiaries.first().unwrap()],
        &funds,
    ));
    profile!(cancel_will(&child, &owner));
    profile!(archive_will(&child));

    profile!(get_contract_version());
    profile!(get_will(&main));
    profile!(get_will_status(&main));
    profile!(get_time_until_deadline(&main));
    profile!(get_guardian_vote_expiry(&main));
    profile!(get_protocol_stats());
    profile!(audit_protocol_stats());
    profile!(repair_protocol_stats());
    profile!(get_wills_by_owner(&owner, &None, &10));
    profile!(get_owner_stats(&owner));
    profile!(get_wills_by_owner_and_status(
        &owner,
        &WillStatus::Active,
        &None,
        &10
    ));
    profile!(get_owner_status_page(
        &owner,
        &WillStatus::Active,
        &None,
        &10
    ));
    profile!(get_wills_by_beneficiary(
        &people.last().unwrap().address,
        &None,
        &10
    ));
    profile!(get_wills(&batch));
    profile!(get_will_history(&main));
    profile!(get_will_history_page(&main, &None, &10));
    profile!(migrate_will(&main, &owner));

    env.ledger().set_timestamp(94 * DAY);
    profile!(guardian_trigger(
        &main,
        &first,
        &GuardianVoteReason::Incapacitated
    ));
    profile!(get_guardian_vote_status(&main, &first));
    profile!(trigger_will(&main, &None));
    profile!(get_triggered_wills(&None, &10));
    profile!(guardian_cancel_trigger(&main, &first));
    profile!(emergency_checkin(&main, &owner));
    profile!(get_emergency_actions(
        &ActionLogScope::Will(main),
        &None,
        &10
    ));
    env.ledger().set_timestamp(200 * DAY);
    client.trigger_will(&main, &None);
    env.ledger().set_timestamp(208 * DAY);
    let delayed_recipient = client
        .get_will(&main)
        .beneficiaries
        .first()
        .unwrap()
        .address;
    asset_client.mint(&delayed_recipient, &1);
    asset_client.set_authorized(&delayed_recipient, &false);
    profile!(release_inheritance(&main, &None));
    asset_client.set_authorized(&delayed_recipient, &true);
    profile!(retry_failed_payout(&main, &token, &delayed_recipient));
    profile!(close_will(&main, &owner));

    let alternate = vec![
        &env,
        Beneficiary {
            address: Address::generate(&env),
            allocation: Allocation::FixedAmount(100),
        },
        Beneficiary {
            address: Address::generate(&env),
            allocation: Allocation::FixedAmount(100),
        },
    ];
    let another = client.create_will(
        &owner,
        &funds,
        &alternate,
        &90,
        &7,
        &Vec::new(&env),
        &0,
        &None,
        &0,
    );
    profile!(renounce_beneficiary(
        &another,
        &alternate.first().unwrap().address
    ));
    let claimant = Address::generate(&env);
    let fingerprint = env.crypto().sha256(&claimant.clone().to_xdr(&env));
    let mut preimage = Bytes::from_array(&env, &fingerprint.to_array());
    preimage.append(&Bytes::from_array(&env, &[7; 32]));
    let commitment = Bytes::from_array(&env, &env.crypto().sha256(&preimage).to_array());
    profile!(add_hashed_beneficiary(
        &another,
        &owner,
        &commitment,
        &2_000
    ));
    env.ledger().set_timestamp(300 * DAY);
    client.trigger_will(&another, &None);
    env.ledger().set_timestamp(308 * DAY);
    client.release_inheritance(&another, &None);
    profile!(reveal_and_claim(&another, &claimant, &preimage));

    let source = include_str!("../src/lib.rs");
    let impl_source = source.split_once("impl WillContract {").unwrap().1;
    // The next unindented brace ends the contract's public implementation.
    for line in impl_source.split_once("\n}").unwrap().0.lines() {
        if let Some(signature) = line.trim_start().strip_prefix("pub fn ") {
            let name = signature.split_once('(').unwrap().0;
            assert!(measured.contains(name), "missing profile for {name}");
        }
    }
}

fn beneficiaries(env: &Env) -> Vec<Beneficiary> {
    let mut people = Vec::new(env);
    for _ in 0..MAX_BENEFICIARIES {
        people.push_back(Beneficiary {
            address: Address::generate(env),
            allocation: Allocation::Percentage(10_000 / MAX_BENEFICIARIES),
        });
    }
    people
}
