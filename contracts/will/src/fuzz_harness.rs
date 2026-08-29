//! Shared fuzzing harness for the SoroWill contract.
//!
//! The harness is deliberately independent of any particular fuzzing engine.
//! It exposes plain, `Env`-free input structs plus a *runner* per entry point
//! that materialises those inputs into Soroban values, invokes the contract,
//! and asserts the contract's invariants. Two front-ends drive it:
//!
//! - the `proptest` suite in `fuzz_test.rs`, which runs on stable Rust as part
//!   of `cargo test` (and therefore as part of CI);
//! - the `cargo-fuzz`/libFuzzer targets under `fuzz/`, which drive the same
//!   runners with coverage-guided input on nightly.
//!
//! # The central invariant
//!
//! Every entry point must either succeed or fail with a *declared*
//! [`WillError`]. It must never abort. The Soroban test host distinguishes the
//! two for us: `panic_with_error!` surfaces through a `try_*` client method as
//! `Err(Ok(error))`, whereas any other panic — an arithmetic overflow, an
//! `unwrap` on `None`, an out-of-bounds index — surfaces as
//! `Err(Err(InvokeError::Abort))`. An abort means the contract hit a code path
//! its author never considered, so the harness treats it as a hard failure.
//!
//! Beyond "does not abort", each runner checks the state invariants that the
//! contract's documentation claims: shares sum to 10,000, deadlines are
//! representable, custody matches the recorded balance, and the reverse
//! indexes agree with the beneficiary list.
//!
//! # Address slots
//!
//! Soroban `Address`es cannot be generated without an `Env`, so inputs refer to
//! addresses by a small `u8` *slot* index into a fixed pool. This keeps the
//! input structs derivable by `arbitrary`, and — more usefully — makes address
//! collisions common rather than astronomically unlikely, so the fuzzer
//! actually explores duplicate beneficiaries, duplicate guardians, and owners
//! who are also beneficiaries.

use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::{Client as TokenClient, StellarAssetClient},
    Address, Env, Vec as SorobanVec,
};

use crate::{
    Allocation, Beneficiary, Will, WillContract, WillContractClient, WillStatus,
    MAX_BENEFICIARIES, MAX_GUARDIANS, MAX_PERIOD_DAYS, SECONDS_PER_DAY,
};

/// Number of distinct addresses the fuzzer picks from.
const ADDRESS_POOL_SIZE: u8 = 6;

/// Ledger timestamp every scenario starts at, far enough from zero that
/// realistic periods neither underflow nor look suspicious.
const BASE_TIMESTAMP: u64 = 1_700_000_000;

/// Upper bound on how much of the test token the owner is funded with. Kept
/// well below `i64::MAX` so the Stellar Asset Contract never rejects the mint
/// itself — we want the *will* contract to be the thing under test.
const MAX_MINT: i128 = 1_000_000_000_000_000_000;

/// Beneficiary lists longer than this are truncated. Deliberately larger than
/// [`MAX_BENEFICIARIES`] so over-long lists are still exercised.
const MAX_FUZZ_BENEFICIARIES: usize = MAX_BENEFICIARIES as usize + 3;

/// Guardian lists longer than this are truncated. Deliberately larger than
/// [`MAX_GUARDIANS`] for the same reason.
const MAX_FUZZ_GUARDIANS: usize = MAX_GUARDIANS as usize + 3;

/// How many successive update attempts a single `update_beneficiaries`
/// scenario replays against one will.
pub const MAX_FUZZ_UPDATES: usize = 4;

/// One beneficiary entry, addressed by pool slot rather than by `Address`.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "fuzzing", derive(arbitrary::Arbitrary))]
pub struct BeneficiarySpec {
    pub address_slot: u8,
    pub basis_points: u32,
}

/// A full `create_will` invocation, including how much the owner is funded
/// with beforehand.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "fuzzing", derive(arbitrary::Arbitrary))]
pub struct CreateWillInput {
    /// Amount minted to the owner before the call. Clamped to `0..=MAX_MINT`.
    pub mint: i128,
    /// Amount passed to `create_will`, verbatim — including zero, negative,
    /// and values far larger than the owner holds.
    pub amount: i128,
    pub beneficiaries: Vec<BeneficiarySpec>,
    pub guardian_slots: Vec<u8>,
    pub checkin_period_days: u64,
    pub grace_period_days: u64,
    /// Number of guardian votes required to trigger early release.
    /// Sanitized to `1..=guardian_slots.len()`.
    pub guardian_threshold: u32,
    /// When set, a successfully created will is driven all the way through
    /// trigger and release so the distribution path is fuzzed too.
    pub release_after_create: bool,
}

/// A `create_will` that is forced to succeed, followed by a series of
/// `update_beneficiaries` attempts against the resulting will.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "fuzzing", derive(arbitrary::Arbitrary))]
pub struct UpdateBeneficiariesInput {
    /// Sanitised into a valid will before use, so the update path is always
    /// reached.
    pub create: CreateWillInput,
    /// Replacement beneficiary lists, applied in order. Each is passed
    /// verbatim, so most will be rejected — that is the point.
    pub updates: Vec<Vec<BeneficiarySpec>>,
    /// When set, one update is additionally attempted by a non-owner, which
    /// must always be rejected.
    pub probe_non_owner: bool,
}

/// What a fuzzed invocation did, for front-ends that want to assert coverage
/// (e.g. "at least some inputs were accepted").
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// The call succeeded and every invariant held afterwards.
    Accepted(u64),
    /// The call was rejected with a declared contract error, as designed.
    Rejected,
}

/// Reports an invariant violation, printing the offending input so both
/// libFuzzer artifacts and `proptest` failures are directly reproducible.
fn violation(input: &impl core::fmt::Debug, message: &str) -> ! {
    panic!("SoroWill invariant violation: {message}\n  input: {input:#?}");
}

/// Asserts `condition`, reporting `message` and the input if it does not hold.
fn check(condition: bool, input: &impl core::fmt::Debug, message: &str) {
    if !condition {
        violation(input, message);
    }
}

/// Builds the fixed pool of addresses a scenario draws from.
fn address_pool(env: &Env) -> SorobanVec<Address> {
    let mut pool = SorobanVec::new(env);
    for _ in 0..ADDRESS_POOL_SIZE {
        pool.push_back(Address::generate(env));
    }
    pool
}

/// Resolves a fuzzer-chosen slot to a pool address.
fn slot(pool: &SorobanVec<Address>, index: u8) -> Address {
    pool.get(index as u32 % pool.len()).unwrap()
}

/// Materialises beneficiary specs into the Soroban vector the contract takes.
fn to_beneficiaries(
    env: &Env,
    pool: &SorobanVec<Address>,
    specs: &[BeneficiarySpec],
) -> SorobanVec<Beneficiary> {
    let mut out = SorobanVec::new(env);
    for spec in specs.iter().take(MAX_FUZZ_BENEFICIARIES) {
        out.push_back(Beneficiary {
            address: slot(pool, spec.address_slot),
            allocation: Allocation::Percentage(spec.basis_points),
        });
    }
    out
}

/// Materialises guardian slots into the Soroban vector the contract takes.
fn to_guardians(env: &Env, pool: &SorobanVec<Address>, slots: &[u8]) -> SorobanVec<Address> {
    let mut out = SorobanVec::new(env);
    for &index in slots.iter().take(MAX_FUZZ_GUARDIANS) {
        out.push_back(slot(pool, index));
    }
    out
}

/// Rewrites arbitrary specs into a list the contract is guaranteed to accept:
/// between 1 and [`MAX_BENEFICIARIES`] entries whose basis points sum to
/// exactly 10,000.
///
/// Address slots are left untouched, so duplicate beneficiaries survive
/// sanitisation and keep getting exercised.
pub fn sanitize_specs(specs: &[BeneficiarySpec]) -> Vec<BeneficiarySpec> {
    // Entries whose `address_slot` resolves to the same pool address (mod
    // ADDRESS_POOL_SIZE) would make `create_will`/`update_beneficiaries`
    // reject the list as a duplicate beneficiary, so this only keeps the
    // first entry per distinct resolved address — capping the result at
    // `ADDRESS_POOL_SIZE` regardless of `MAX_BENEFICIARIES`, since the pool
    // cannot supply more unique addresses than it has slots.
    let mut seen_slots: Vec<u8> = Vec::new();
    let mut out: Vec<BeneficiarySpec> = Vec::new();
    for spec in specs.iter() {
        let resolved = spec.address_slot % ADDRESS_POOL_SIZE;
        if seen_slots.contains(&resolved) {
            continue;
        }
        seen_slots.push(resolved);
        out.push(spec.clone());
        if out.len() >= MAX_BENEFICIARIES as usize {
            break;
        }
    }
    if out.is_empty() {
        out.push(BeneficiarySpec {
            address_slot: 0,
            basis_points: 10_000,
        });
    }

    // One point each, and the remainder to the first entry. With at most
    // `MAX_BENEFICIARIES` (10) entries the first share is never below 9_991.
    let extra = 10_000 - (out.len() as u32 - 1);
    for spec in out.iter_mut() {
        spec.basis_points = 1;
    }
    out[0].basis_points = extra;
    out
}

/// Rewrites an arbitrary `create_will` input into one the contract is
/// guaranteed to accept, so scenarios that fuzz a *later* entry point always
/// have a will to work with.
fn sanitize_create(input: &CreateWillInput) -> CreateWillInput {
    // Pool slot 0 is always the scenario's owner (see `Scenario::new`), and
    // the owner can never be their own guardian, so it is excluded here in
    // addition to deduplicating repeated slots.
    let mut guardian_slots: Vec<u8> = Vec::new();
    for &index in input.guardian_slots.iter() {
        let resolved = index % ADDRESS_POOL_SIZE;
        if resolved != 0
            && guardian_slots.len() < MAX_GUARDIANS as usize
            && !guardian_slots.contains(&resolved)
        {
            guardian_slots.push(resolved);
        }
    }

    let guardian_count = guardian_slots.len() as u32;
    let threshold = if guardian_count > 0 {
        input.guardian_threshold.clamp(1, guardian_count)
    } else {
        0
    };

    CreateWillInput {
        mint: MAX_MINT,
        // 1..=1_000_000_000, so the owner can always cover it.
        amount: input.amount.rem_euclid(1_000_000_000) + 1,
        beneficiaries: sanitize_specs(&input.beneficiaries),
        guardian_slots,
        checkin_period_days: input.checkin_period_days % MAX_PERIOD_DAYS + 1,
        grace_period_days: input.grace_period_days % MAX_PERIOD_DAYS + 1,
        guardian_threshold: threshold,
        release_after_create: false,
    }
}

/// Everything a scenario needs: a live env, a funded owner, a token, and a
/// registered will contract.
struct Scenario<'a> {
    env: Env,
    client: WillContractClient<'a>,
    token: TokenClient<'a>,
    token_address: Address,
    pool: SorobanVec<Address>,
    owner: Address,
}

impl<'a> Scenario<'a> {
    /// Registers the contracts and mints `mint` (clamped) to the owner.
    fn new(mint: i128) -> Scenario<'a> {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().set_timestamp(BASE_TIMESTAMP);

        let pool = address_pool(&env);
        let owner = pool.get(0).unwrap();

        let sac = env.register_stellar_asset_contract_v2(owner.clone());
        let token_address = sac.address();
        let minted = mint.clamp(0, MAX_MINT);
        if minted > 0 {
            StellarAssetClient::new(&env, &token_address).mint(&owner, &minted);
        }

        let contract_id = env.register(WillContract, ());

        Self {
            client: WillContractClient::new(&env, &contract_id),
            token: TokenClient::new(&env, &token_address),
            token_address,
            pool,
            owner,
            env,
        }
    }

    /// Advances the ledger clock, saturating rather than panicking: the
    /// harness must never be the thing that overflows.
    fn advance_days(&self, days: u64) {
        let seconds = days.saturating_mul(SECONDS_PER_DAY);
        self.env.ledger().with_mut(|ledger| {
            ledger.timestamp = ledger.timestamp.saturating_add(seconds);
        });
    }
}

/// Drives `create_will` with arbitrary input.
///
/// Rejects malformed input via a declared error, or accepts it and holds to
/// every invariant a freshly created will is documented to have.
pub fn run_create_will(input: &CreateWillInput) -> Outcome {
    let scenario = Scenario::new(input.mint);
    let beneficiaries = to_beneficiaries(&scenario.env, &scenario.pool, &input.beneficiaries);
    let guardians = to_guardians(&scenario.env, &scenario.pool, &input.guardian_slots);
    let tokens: SorobanVec<(Address, i128)> =
        soroban_sdk::vec![&scenario.env, (scenario.token_address.clone(), input.amount)];

    let keeper_bounty: Option<u32> = None;
    let result = scenario.client.try_create_will(
        &scenario.owner,
        &tokens,
        &beneficiaries,
        &input.checkin_period_days,
        &input.grace_period_days,
        &guardians,
        &input.guardian_threshold,
        &keeper_bounty,
        &0,
    );

    let will_id = match result {
        Ok(Ok(will_id)) => will_id,
        Ok(Err(_)) => violation(input, "create_will returned a value that is not a u64 will id"),
        // A declared contract error is the correct response to bad input.
        Err(Ok(_)) => return Outcome::Rejected,
        Err(Err(invoke_error)) => violation(
            input,
            &std::format!("create_will aborted instead of returning a declared WillError: {invoke_error:?}"),
        ),
    };

    assert_created_will(&scenario, input, will_id, &beneficiaries, &guardians);
    if input.release_after_create {
        assert_full_release(&scenario, input, will_id);
    }

    Outcome::Accepted(will_id)
}

/// Checks every invariant that must hold for a will `create_will` accepted.
fn assert_created_will(
    scenario: &Scenario<'_>,
    input: &CreateWillInput,
    will_id: u64,
    beneficiaries: &SorobanVec<Beneficiary>,
    guardians: &SorobanVec<Address>,
) {
    let will = scenario.client.get_will(&will_id);

    check(will.id == will_id, input, "stored will id does not match the returned id");
    check(will.owner == scenario.owner, input, "will owner is not the creator");
    check(will.status == WillStatus::Active, input, "a new will is not Active");
    check(
        will.balances.get(scenario.token_address.clone()).unwrap_or(0) == input.amount,
        input,
        "recorded balance does not match the locked amount",
    );
    check(
        will.balances.get(scenario.token_address.clone()).unwrap_or(0) > 0,
        input,
        "a will was created with a non-positive balance",
    );
    check(will.trigger_time.is_none(), input, "a new will already has a trigger time");
    check(will.guardian_votes == 0, input, "a new will already has guardian votes");
    check(will.beneficiaries == *beneficiaries, input, "stored beneficiaries differ from the supplied list");
    // Compare guardian addresses (stored as Vec<Guardian>).
    let guardian_addresses: SorobanVec<Address> = {
        let mut v: SorobanVec<Address> = SorobanVec::new(&scenario.env);
        for g in will.guardians.iter() {
            v.push_back(g.address.clone());
        }
        v
    };
    check(guardian_addresses == *guardians, input, "stored guardians differ from the supplied list");

    // Shares: bounds, and a sum computed in u128 so the check itself cannot
    // overflow while testing for overflow.
    check(
        !will.beneficiaries.is_empty() && will.beneficiaries.len() <= MAX_BENEFICIARIES,
        input,
        "beneficiary count is outside 1..=MAX_BENEFICIARIES",
    );
    let mut total: u128 = 0;
    for beneficiary in will.beneficiaries.iter() {
        total += match beneficiary.allocation {
            Allocation::Percentage(bp) => bp as u128,
            Allocation::FixedAmount(_) => 0,
        };
    }
    check(total == 10_000, input, "beneficiary basis points do not sum to 10,000");

    // Guardians: a stored quorum must be reachable, which it is not if the
    // same address occupies more than one slot.
    check(
        will.guardians.len() <= MAX_GUARDIANS,
        input,
        "guardian count exceeds MAX_GUARDIANS",
    );
    for i in 0..will.guardians.len() {
        for j in (i + 1)..will.guardians.len() {
            check(
                will.guardians.get_unchecked(i) != will.guardians.get_unchecked(j),
                input,
                "the same guardian was stored twice, making the quorum unreachable",
            );
        }
    }

    // Deadlines must be representable. If they are not, `trigger_will` panics
    // on every call and the balance is locked in the contract forever.
    let checkin_deadline = will
        .checkin_period_days
        .checked_mul(SECONDS_PER_DAY)
        .and_then(|seconds| will.last_checkin.checked_add(seconds));
    check(
        checkin_deadline.is_some(),
        input,
        "the check-in deadline overflows u64, so the will can never be triggered",
    );
    check(
        will.grace_period_days.checked_mul(SECONDS_PER_DAY).is_some(),
        input,
        "the grace period overflows u64, so the will can never be released",
    );
    check(
        will.checkin_period_days > 0 && will.grace_period_days > 0,
        input,
        "a zero-length period leaves the will triggerable in the ledger it was created in",
    );

    // Custody: the contract must actually hold what it says it holds.
    check(
        scenario.token.balance(&scenario.client.address) == will.balances.get(scenario.token_address.clone()).unwrap_or(0),
        input,
        "the contract's token balance does not match the will's recorded balance",
    );

    // Reverse indexes must find the will.
    check(
        scenario
            .client
            .get_wills_by_owner(&scenario.owner, &None, &100)
            .iter()
            .any(|indexed| indexed.id == will_id),
        input,
        "the new will is missing from its owner's index",
    );
    for beneficiary in will.beneficiaries.iter() {
        check(
            scenario
                .client
                .get_wills_by_beneficiary(&beneficiary.address, &None, &50)
                .iter()
                .any(|indexed| indexed.id == will_id),
            input,
            "a beneficiary is missing the will from their index",
        );
    }
}

/// Drives a created will through trigger and release, checking that the whole
/// balance leaves the contract with no dust stranded behind.
fn assert_full_release(scenario: &Scenario<'_>, input: &CreateWillInput, will_id: u64) {
    let will = scenario.client.get_will(&will_id);

    scenario.advance_days(will.checkin_period_days.saturating_add(1));
    match scenario.client.try_trigger_will(&will_id) {
        Ok(Ok(())) => {}
        Ok(Err(_)) => violation(input, "trigger_will returned an undecodable value"),
        Err(Ok(_)) => violation(input, "trigger_will was rejected after the check-in deadline passed"),
        Err(Err(e)) => violation(input, &std::format!("trigger_will aborted: {e:?}")),
    }

    scenario.advance_days(will.grace_period_days.saturating_add(1));
    match scenario.client.try_release_inheritance(&will_id, &None) {
        Ok(Ok(())) => {}
        Ok(Err(_)) => violation(input, "release_inheritance returned an undecodable value"),
        Err(Ok(_)) => violation(input, "release_inheritance was rejected after the grace period expired"),
        Err(Err(e)) => violation(input, &std::format!("release_inheritance aborted: {e:?}")),
    }

    let released = scenario.client.get_will(&will_id);
    check(released.status == WillStatus::Released, input, "the will is not Released after release_inheritance");
    check(released.balances.is_empty(), input, "the released will still records balances");
    check(
        scenario.token.balance(&scenario.client.address) == 0,
        input,
        "the contract kept dust after distributing the inheritance",
    );
}

/// Drives `update_beneficiaries` with arbitrary replacement lists.
///
/// A second will sharing the same address pool is created alongside the one
/// under test, so the harness can also detect a beneficiary index that is
/// clobbered across wills.
pub fn run_update_beneficiaries(input: &UpdateBeneficiariesInput) -> Vec<Outcome> {
    let create = sanitize_create(&input.create);
    let scenario = Scenario::new(create.mint);

    let will_id = create_valid_will(&scenario, input, &create);
    // A neighbouring will, whose index entries must survive every update
    // applied to `will_id`.
    let neighbour_id = create_valid_will(&scenario, input, &create);

    let mut outcomes = Vec::new();
    for specs in input.updates.iter().take(MAX_FUZZ_UPDATES) {
        let before = scenario.client.get_will(&will_id);
        let replacement = to_beneficiaries(&scenario.env, &scenario.pool, specs);

        let outcome = match scenario.client.try_update_beneficiaries(
            &will_id,
            &scenario.owner,
            &replacement,
        ) {
            Ok(Ok(())) => {
                assert_updated_beneficiaries(&scenario, input, will_id, &before, &replacement);
                Outcome::Accepted(will_id)
            }
            Ok(Err(_)) => violation(input, "update_beneficiaries returned an undecodable value"),
            Err(Ok(_)) => {
                // A rejected update must leave the will exactly as it was.
                let after = scenario.client.get_will(&will_id);
                check(
                    after.beneficiaries == before.beneficiaries,
                    input,
                    "a rejected update still changed the beneficiary list",
                );
                check(
                    after.balances == before.balances && after.status == before.status,
                    input,
                    "a rejected update changed the will's balance or status",
                );
                Outcome::Rejected
            }
            Err(Err(e)) => violation(
                input,
                &std::format!("update_beneficiaries aborted instead of returning a declared WillError: {e:?}"),
            ),
        };
        outcomes.push(outcome);

        // Whatever happened above, the untouched will must still be findable
        // through every one of its beneficiaries.
        let neighbour = scenario.client.get_will(&neighbour_id);
        for beneficiary in neighbour.beneficiaries.iter() {
            check(
                scenario
                    .client
                    .get_wills_by_beneficiary(&beneficiary.address, &None, &50)
                    .iter()
                    .any(|indexed| indexed.id == neighbour_id),
                input,
                "updating one will dropped an unrelated will from a beneficiary's index",
            );
        }
    }

    if input.probe_non_owner {
        let non_owner = slot(&scenario.pool, 1);
        let before = scenario.client.get_will(&will_id);
        let replacement = to_beneficiaries(
            &scenario.env,
            &scenario.pool,
            &sanitize_specs(&input.create.beneficiaries),
        );
        match scenario
            .client
            .try_update_beneficiaries(&will_id, &non_owner, &replacement)
        {
            Ok(Ok(())) => violation(
                input,
                "update_beneficiaries accepted a caller that does not own the will",
            ),
            Err(Err(e)) => violation(
                input,
                &std::format!("update_beneficiaries aborted for a non-owner caller: {e:?}"),
            ),
            _ => {
                let after = scenario.client.get_will(&will_id);
                check(
                    after.owner == before.owner,
                    input,
                    "a non-owner update changed the will's owner",
                );
            }
        }
    }

    outcomes
}

/// Creates a will from a sanitised input, treating any rejection as a harness
/// failure — sanitisation is supposed to guarantee acceptance, so a rejection
/// means validation and the documented rules have drifted apart.
fn create_valid_will(
    scenario: &Scenario<'_>,
    input: &impl core::fmt::Debug,
    create: &CreateWillInput,
) -> u64 {
    let beneficiaries = to_beneficiaries(&scenario.env, &scenario.pool, &create.beneficiaries);
    let guardians = to_guardians(&scenario.env, &scenario.pool, &create.guardian_slots);
    let tokens: SorobanVec<(Address, i128)> =
        soroban_sdk::vec![&scenario.env, (scenario.token_address.clone(), create.amount)];

    let keeper_bounty: Option<u32> = None;
    match scenario.client.try_create_will(
        &scenario.owner,
        &tokens,
        &beneficiaries,
        &create.checkin_period_days,
        &create.grace_period_days,
        &guardians,
        &create.guardian_threshold,
        &keeper_bounty,
        &0,
    ) {
        Ok(Ok(will_id)) => will_id,
        Ok(Err(_)) => violation(input, "create_will returned a value that is not a u64 will id"),
        Err(Ok(e)) => violation(
            input,
            &std::format!("create_will rejected an input the harness built to be valid: {e:?}"),
        ),
        Err(Err(e)) => violation(input, &std::format!("create_will aborted on valid input: {e:?}")),
    }
}

/// Checks every invariant that must hold after an accepted beneficiary update.
fn assert_updated_beneficiaries(
    scenario: &Scenario<'_>,
    input: &UpdateBeneficiariesInput,
    will_id: u64,
    before: &Will,
    replacement: &SorobanVec<Beneficiary>,
) {
    let will = scenario.client.get_will(&will_id);

    check(will.beneficiaries == *replacement, input, "the stored beneficiaries are not the ones supplied");
    check(will.balances == before.balances, input, "an update changed the will's balances");
    check(will.status == before.status, input, "an update changed the will's status");
    check(will.owner == before.owner, input, "an update changed the will's owner");
    check(will.guardians == before.guardians, input, "an update changed the will's guardians");
    check(will.last_checkin == before.last_checkin, input, "an update changed the will's last check-in");

    check(
        !will.beneficiaries.is_empty() && will.beneficiaries.len() <= MAX_BENEFICIARIES,
        input,
        "beneficiary count is outside 1..=MAX_BENEFICIARIES after an update",
    );
    let mut total: u128 = 0;
    for beneficiary in will.beneficiaries.iter() {
        total += match beneficiary.allocation {
            Allocation::Percentage(bp) => bp as u128,
            Allocation::FixedAmount(_) => 0,
        };
    }
    check(total == 10_000, input, "beneficiary basis points do not sum to 10,000 after an update");

    // Every current beneficiary must be able to find the will...
    for beneficiary in will.beneficiaries.iter() {
        check(
            scenario
                .client
                .get_wills_by_beneficiary(&beneficiary.address, &None, &50)
                .iter()
                .any(|indexed| indexed.id == will_id),
            input,
            "a current beneficiary is missing the will from their index",
        );
    }

    // ...and every dropped one must not, or a removed heir keeps a stale claim
    // in every client that reads the index.
    for old in before.beneficiaries.iter() {
        let retained = will
            .beneficiaries
            .iter()
            .any(|current| current.address == old.address);
        if retained {
            continue;
        }
        check(
            !scenario
                .client
                .get_wills_by_beneficiary(&old.address, &None, &50)
                .iter()
                .any(|indexed| indexed.id == will_id),
            input,
            "a removed beneficiary still has the will in their index",
        );
    }
}
