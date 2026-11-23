//! Gas / resource-usage benchmarks for the will contract.
//!
//! These tests exercise every public operation of the will contract and
//! record the CPU-instruction and memory cost reported by the Soroban host.
//! They are intentionally assertion-light: the goal is to *document* the
//! relative cost of each operation and to flag the hot paths so that future
//! optimizations can be measured against a stable baseline.
//!
//! Run with:
//! ```text
//! cargo test -p will gas_benchmark -- --nocapture
//! ```
//!
//! Hot paths (highest cost, see `report_hot_paths`):
//!   * `distribute`  – iterates over all beneficiaries and performs a token
//!                     transfer per beneficiary (storage read + write + token
//!                     call each iteration).
//!   * `create_will` – writes the full will struct plus guardian list to
//!                     persistent storage.
//!   * `confirm` / `check_in` – read-modify-write of the will struct.
//!   * `cancel`      – read-modify-write plus a single token transfer.
//!   * queries (`get_will`, `is_confirmed`, ...) – single storage read.

use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token, Address, Env, Vec,
};

use crate::{WillContract, WillContractClient, WillStatus};

/// Snapshot of the host resource usage for a single operation.
#[derive(Clone, Copy, Debug)]
struct GasReport {
    cpu: u64,
    mem: u64,
}

impl GasReport {
    fn capture(env: &Env) -> Self {
        let budget = env.budget();
        Self {
            cpu: budget.cpu_instruction_cost(),
            mem: budget.memory_bytes_cost(),
        }
    }

    fn delta(&self, before: &GasReport) -> Self {
        Self {
            cpu: self.cpu.saturating_sub(before.cpu),
            mem: self.mem.saturating_sub(before.mem),
        }
    }
}

/// Measure the resource cost of `f` and print a labelled report.
fn measure<F: FnOnce()>(env: &Env, label: &str, f: F) -> GasReport {
    let before = GasReport::capture(env);
    f();
    let after = GasReport::capture(env);
    let delta = after.delta(&before);
    std::println!(
        "[gas] {:<24} cpu={:>12} mem={:>12}",
        label, delta.cpu, delta.mem
    );
    delta
}

/// Build a funded environment with a deployed will contract and a token.
fn setup<'a>() -> (Env, WillContractClient<'a>, Address, token::Client<'a>, token::StellarAssetClient<'a>) {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let contract_id = env.register_contract(None, WillContract);
    let client = WillContractClient::new(&env, &contract_id);

    let token_admin = Address::generate(&env);
    let token_id = env.register_stellar_asset_contract(token_admin.clone());
    let token_client = token::Client::new(&env, &token_id);
    let token_admin_client = token::StellarAssetClient::new(&env, &token_id);

    client.initialize(&admin, &token_id);

    (env, client, admin, token_client, token_admin_client)
}

fn beneficiaries(env: &Env, n: u32) -> Vec<(Address, u32)> {
    let mut list = Vec::new(env);
    for i in 0..n {
        list.push_back((Address::generate(env), 100 / n + i));
    }
    list
}

#[test]
fn gas_benchmark_create_will() {
    let (env, client, owner, _token, _token_admin) = setup();
    let bens = beneficiaries(&env, 3);

    measure(&env, "create_will", || {
        client.create_will(&owner, &bens, &1_000u32, &100u32);
    });
}

#[test]
fn gas_benchmark_confirm() {
    let (env, client, owner, _token, _token_admin) = setup();
    let bens = beneficiaries(&env, 3);
    client.create_will(&owner, &bens, &1_000u32, &100u32);

    measure(&env, "confirm", || {
        client.confirm(&owner);
    });
}

#[test]
fn gas_benchmark_check_in() {
    let (env, client, owner, _token, _token_admin) = setup();
    let bens = beneficiaries(&env, 3);
    client.create_will(&owner, &bens, &1_000u32, &100u32);
    client.confirm(&owner);

    env.ledger().with_mut(|l| l.timestamp += 10);

    measure(&env, "check_in", || {
        client.check_in(&owner);
    });
}

#[test]
fn gas_benchmark_cancel() {
    let (env, client, owner, _token, _token_admin) = setup();
    let bens = beneficiaries(&env, 3);
    client.create_will(&owner, &bens, &1_000u32, &100u32);
    client.confirm(&owner);

    measure(&env, "cancel", || {
        client.cancel(&owner);
    });
}

#[test]
fn gas_benchmark_distribute() {
    let (env, client, owner, token, token_admin) = setup();
    let bens = beneficiaries(&env, 3);
    token_admin.mint(&owner, &10_000i128);
    client.create_will(&owner, &bens, &1_000u32, &100u32);
    client.confirm(&owner);

    // Advance past the inactivity window so distribution is allowed.
    env.ledger().with_mut(|l| l.timestamp += 10_000);

    measure(&env, "distribute", || {
        client.distribute(&owner);
    });

    // Sanity: funds actually moved.
    assert_eq!(token.balance(&owner), 0);
}

#[test]
fn gas_benchmark_queries() {
    let (env, client, owner, _token, _token_admin) = setup();
    let bens = beneficiaries(&env, 3);
    client.create_will(&owner, &bens, &1_000u32, &100u32);

    measure(&env, "get_will", || {
        let _ = client.get_will(&owner);
    });

    measure(&env, "is_confirmed", || {
        let _ = client.is_confirmed(&owner);
    });

    measure(&env, "status", || {
        let _ = client.status(&owner);
    });
}

/// Aggregate the per-operation costs and print the hot paths in descending
/// order so the highest consumers are easy to spot in CI logs.
#[test]
fn report_hot_paths() {
    let (env, client, owner, _token, token_admin) = setup();
    let bens = beneficiaries(&env, 3);
    token_admin.mint(&owner, &10_000i128);

    let mut reports: Vec<(soroban_sdk::String, GasReport)> = Vec::new(&env);

    let r = measure(&env, "create_will", || {
        client.create_will(&owner, &bens, &1_000u32, &100u32);
    });
    reports.push_back((soroban_sdk::String::from_str(&env, "create_will"), r));

    let r = measure(&env, "confirm", || client.confirm(&owner));
    reports.push_back((soroban_sdk::String::from_str(&env, "confirm"), r));

    env.ledger().with_mut(|l| l.timestamp += 10);
    let r = measure(&env, "check_in", || client.check_in(&owner));
    reports.push_back((soroban_sdk::String::from_str(&env, "check_in"), r));

    env.ledger().with_mut(|l| l.timestamp += 10_000);
    let r = measure(&env, "distribute", || client.distribute(&owner));
    reports.push_back((soroban_sdk::String::from_str(&env, "distribute"), r));

    // Sort descending by CPU cost (simple insertion sort; list is tiny).
    let len = reports.len();
    for i in 1..len {
        let mut j = i;
        while j > 0 {
            let a = reports.get(j - 1).unwrap();
            let b = reports.get(j).unwrap();
            if a.1.cpu < b.1.cpu {
                reports.set(j - 1, b);
                reports.set(j, a);
                j -= 1;
            } else {
                break;
            }
        }
    }

    std::println!("\n[gas] hot paths (descending cpu):");
    for (name, report) in reports.iter() {
        std::println!(
            "[gas]   {:<16} cpu={:>12} mem={:>12}",
            name, report.cpu, report.mem
        );
    }

    // The will must be settled after distribution.
    assert_eq!(client.status(&owner), WillStatus::Distributed);
}
