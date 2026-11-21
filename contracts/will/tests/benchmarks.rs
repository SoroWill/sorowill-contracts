//! Gas and resource benchmarks per operation (#707).
//!
//! Measures Soroban CPU instructions, ledger read/write entries, byte sizes,
//! and estimated fee (in stroops) for core `WillContract` operations using
//! `Env::cost_estimate()`.
//!
//! Run with:
//! ```text
//! cargo test -p will --test benchmarks -- --nocapture
//! ```

use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::StellarAssetClient,
    vec, Address, Env, Vec,
};

use will::{Allocation, Beneficiary, WillContract, WillContractClient};

const DAY: u64 = 86_400;

struct GasMeasurement {
    operation: &'static str,
    instructions: i64,
    read_entries: u32,
    write_entries: u32,
    read_bytes: u32,
    write_bytes: u32,
    fee_stroops: i64,
}

fn measure(env: &Env, operation: &'static str) -> GasMeasurement {
    let resources = env.cost_estimate().resources();
    let fee = env.cost_estimate().fee();
    GasMeasurement {
        operation,
        instructions: resources.instructions,
        read_entries: resources.read_entries,
        write_entries: resources.write_entries,
        read_bytes: resources.read_bytes,
        write_bytes: resources.write_bytes,
        fee_stroops: fee.total,
    }
}

fn setup<'a>() -> (Env, WillContractClient<'a>, Address, Address) {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_700_000_000);

    let owner = Address::generate(&env);
    let sac = env.register_stellar_asset_contract_v2(owner.clone());
    let token = sac.address();
    StellarAssetClient::new(&env, &token).mint(&owner, &1_000_000_000_000);

    let contract_id = env.register(WillContract, ());
    let client = WillContractClient::new(&env, &contract_id);
    (env, client, owner, token)
}

fn make_beneficiaries(env: &Env, count: u32) -> Vec<Beneficiary> {
    let mut list = Vec::new(env);
    let even = 10_000 / count;
    let mut allocated = 0;
    for i in 0..count {
        let bps = if i == count - 1 {
            10_000 - allocated
        } else {
            even
        };
        allocated += bps;
        list.push_back(Beneficiary {
            address: Address::generate(env),
            allocation: Allocation::Percentage(bps),
        });
    }
    list
}

#[test]
fn benchmark_gas_costs_per_operation() {
    let (env, client, owner, token) = setup();
    let mut report: std::vec::Vec<GasMeasurement> = std::vec::Vec::new();

    let beneficiaries = make_beneficiaries(&env, 3);
    let guardians = vec![&env, Address::generate(&env), Address::generate(&env)];
    let tokens = vec![&env, (token.clone(), 1_000_000_i128)];

    let will_id = client.create_will(
        &owner,
        &tokens,
        &beneficiaries,
        &90,
        &7,
        &guardians,
        &2,
        &None,
        &0,
    );
    report.push(measure(&env, "create_will"));

    client.check_in(&will_id, &owner);
    report.push(measure(&env, "check_in"));

    client.top_up(&will_id, &owner, &token, &500_000_i128);
    report.push(measure(&env, "top_up"));

    let updated = make_beneficiaries(&env, 3);
    client.update_beneficiaries(&will_id, &owner, &updated);
    report.push(measure(&env, "update_beneficiaries"));

    let new_guardians = vec![&env, Address::generate(&env), Address::generate(&env)];
    client.update_guardians(&will_id, &owner, &new_guardians);
    report.push(measure(&env, "update_guardians"));

    env.ledger().with_mut(|l| l.timestamp += 91 * DAY);
    client.trigger_will(&will_id);
    report.push(measure(&env, "trigger_will"));

    env.ledger().with_mut(|l| l.timestamp += 8 * DAY);
    client.release_inheritance(&will_id, &None);
    report.push(measure(&env, "release_inheritance"));

    let cancel_id = client.create_will(
        &owner,
        &tokens,
        &beneficiaries,
        &90,
        &7,
        &vec![&env],
        &0,
        &None,
        &0,
    );
    client.cancel_will(&cancel_id, &owner);
    report.push(measure(&env, "cancel_will"));

    let batch_ids = client.batch_create_wills(
        &owner,
        &vec![
            &env,
            (tokens.clone(), beneficiaries.clone(), 90, 7, vec![&env], 0),
            (tokens.clone(), beneficiaries.clone(), 90, 7, vec![&env], 0),
        ],
    );
    report.push(measure(&env, "batch_create_wills(2)"));

    client.batch_check_in(&batch_ids, &owner);
    report.push(measure(&env, "batch_check_in(2)"));

    std::println!("\n| operation | instructions | read_entries | write_entries | read_bytes | write_bytes | fee_stroops |");
    std::println!("| --- | ---: | ---: | ---: | ---: | ---: | ---: |");
    for m in &report {
        std::println!(
            "| {} | {} | {} | {} | {} | {} | {} |",
            m.operation,
            m.instructions,
            m.read_entries,
            m.write_entries,
            m.read_bytes,
            m.write_bytes,
            m.fee_stroops,
        );
        assert!(
            m.instructions > 0,
            "{} should meter non-zero CPU instructions",
            m.operation
        );
        assert!(
            m.fee_stroops > 0,
            "{} should meter non-zero fee in stroops",
            m.operation
        );
    }
}
