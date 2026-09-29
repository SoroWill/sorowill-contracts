# Fuzzing SoroWill

The contract is fuzzed at two depths, both driving the same harness
([`contracts/will/src/fuzz_harness.rs`](../contracts/will/src/fuzz_harness.rs)):

| | Engine | Toolchain | Runs in CI | Use it for |
|---|---|---|---|---|
| [`contracts/will/src/fuzz_test.rs`](../contracts/will/src/fuzz_test.rs) | `proptest` | stable | yes, on every PR | regressions, quick local feedback |
| [`fuzz/`](../fuzz) | `cargo-fuzz` / libFuzzer | nightly | on demand + nightly schedule | deep, coverage-guided exploration |

Keeping one harness behind both means a bug found by libFuzzer can be pinned as
a `proptest` case (or a plain `#[test]`) without rewriting it.

## Entry point coverage

`WillContract` has around 35 public entry points. Only the two below are
currently exercised by a `run_*`/`*Input` pair in `fuzz_harness.rs` (and thus
by both the `proptest` suite and a `fuzz/fuzz_targets/` target):

| Entry point | Fuzzed |
|---|---|
| `create_will` | yes — `run_create_will` / `CreateWillInput` |
| `update_beneficiaries` | yes — `run_update_beneficiaries` / `UpdateBeneficiariesInput` |

Every other state-mutating entry point has no coverage-guided fuzzing yet and
is only exercised by hand-written unit tests:

- `cancel_will`
- `merge_wills`
- `split_will`
- `guardian_trigger`
- `guardian_cancel_trigger`
- `accept_guardian_role`
- `reject_guardian_role`
- `release_inheritance`
- `reveal_and_claim`
- `add_hashed_beneficiary`
- `top_up`
- `renounce_beneficiary`
- `batch_create_wills`
- `batch_check_in`
- `clone_will`
- `migrate_will`
- `archive_will`
- `close_will`
- `confirm_will`
- `check_in`
- `set_delegate`
- `trigger_will`
- `emergency_checkin`
- `update_guardians`
- `update_periods`
- `update_will_settings`

`merge_wills`, `split_will`, and `cancel_will` are the highest-priority gaps:
they mutate balances and beneficiary/guardian indexes across two wills (or
split one into two) in ways a differential/invariant harness — in the style
of the existing `run_create_will`/`run_update_beneficiaries` pair — is well
suited to catch. See "Adding a target" below for how to wire one up.

## The invariants

Every target asserts the same central property first:

> Every entry point either succeeds or fails with a **declared** `WillError`.
> It must never abort.

The Soroban test host distinguishes the two. Through a `try_*` client method,
`panic_with_error!` arrives as `Err(Ok(error))`, while any other panic —
arithmetic overflow, `unwrap` on `None`, an out-of-bounds index — arrives as
`Err(Err(InvokeError::Abort))`. An abort means execution reached a path nobody
designed, so the harness fails on it.

On top of that, an accepted `create_will` must leave a will where:

- the status is `Active`, the recorded balance equals the locked amount, and
  the contract's token balance matches it;
- there are 1–10 beneficiaries, each with a share of 1–100, summing to exactly
  100;
- there are at most 3 guardians, with no address repeated (a repeated guardian
  makes the 2-of-N quorum unreachable);
- both `last_checkin + checkin_period_days * 86_400` and
  `grace_period_days * 86_400` are representable as `u64` — otherwise
  `trigger_will` panics on every call and the balance is locked forever;
- the owner and beneficiary reverse indexes contain the new will.

And an accepted `update_beneficiaries` must leave the will with the same
balance, status, owner, guardians and last check-in; the same share invariants
as above; every current beneficiary able to find the will through
`get_wills_by_beneficiary`; and every *dropped* beneficiary no longer able to.
A **rejected** update must change nothing at all. The harness also creates a
second will sharing the address pool, and checks its index survives every
update applied to the first.

With `release_after_create` set, the target additionally drives the will
through `trigger_will` and `release_inheritance` and asserts the contract is
left holding exactly zero — no dust stranded by the rounding remainder.

## Running the proptest suite

No extra tooling; it is part of the normal test run.

```sh
cargo test                                  # everything
cargo test --package will fuzz_test         # just the fuzzing suite
```

To explore harder than the default 48 cases per property:

```sh
PROPTEST_CASES=5000 cargo test --package will fuzz_test -- --nocapture
```

A failing case is written to `contracts/will/proptest-regressions/` and replayed
on every subsequent run. **Commit that file** — it is how a regression stays
caught.

## Running cargo-fuzz

libFuzzer needs a nightly toolchain, and works on Linux and macOS. (On Windows,
use WSL, or rely on the `proptest` suite.)

```sh
rustup toolchain install nightly
cargo install cargo-fuzz

cd fuzz
cargo +nightly fuzz run create_will
cargo +nightly fuzz run update_beneficiaries
```

Each target runs until you stop it. Useful flags — everything after `--` goes
to libFuzzer itself:

```sh
# Stop after 5 minutes (CI-friendly).
cargo +nightly fuzz run create_will -- -max_total_time=300

# Use all cores.
cargo +nightly fuzz run create_will -- -jobs=8 -workers=8

# Cap each input's size; the harness ignores bytes past the end anyway.
cargo +nightly fuzz run create_will -- -max_len=256
```

A Soroban `Env` is registered per iteration, so expect thousands of execs per
second rather than millions. That is normal for contract fuzzing — the
interesting inputs are structural, not byte-level.

### When it finds something

libFuzzer writes the failing input to `fuzz/artifacts/<target>/` and prints the
harness's message, which names the violated invariant and pretty-prints the
decoded input. To replay it:

```sh
cargo +nightly fuzz run create_will fuzz/artifacts/create_will/crash-<hash>
```

Shrink it first if the input is large:

```sh
cargo +nightly fuzz tmin create_will fuzz/artifacts/create_will/crash-<hash>
```

Then pin the bug as a `#[test]` in `contracts/will/src/fuzz_test.rs` before
fixing it, so CI keeps it fixed.

### Coverage

```sh
cargo +nightly fuzz coverage create_will
cargo cov -- show target/*/coverage/*/coverage.profdata \
  --instr-profile=fuzz/coverage/create_will/coverage.profdata
```

## How the harness is wired

Soroban `Address`es cannot be built without an `Env`, so they are not
`Arbitrary`. Inputs therefore refer to addresses by a `u8` **slot** into a
fixed six-address pool. Besides making the input types derivable, this makes
address collisions common instead of astronomically unlikely, so duplicate
beneficiaries, duplicate guardians, and an owner who is also a beneficiary all
get exercised routinely.

The harness lives inside the `will` crate rather than in `fuzz/`, behind
`#[cfg(any(test, feature = "fuzzing"))]`, so both front-ends can share it. The
`fuzzing` feature links `std` and the Soroban test host — **never enable it for
a wasm build.** The default (and CI) wasm build sees neither `cfg(test)` nor
the feature and stays `no_std`.

`fuzz/` is excluded from the workspace and carries its own `[workspace]` table,
so `cargo test --workspace` and `cargo clippy --all-targets` never try to build
libFuzzer on stable. It also sets its own `[profile.release]`: the root
profile's `panic = "abort"` would stop the Soroban host from catching contract
panics, turning every legitimate `panic_with_error!` rejection into a false
crash, and `overflow-checks`/`debug-assertions` must stay on or a wrapping
overflow would go unnoticed.

## Adding a target

1. Add an input struct and a `run_*` function to `fuzz_harness.rs`, with
   `#[cfg_attr(feature = "fuzzing", derive(arbitrary::Arbitrary))]` on the
   input.
2. Add `fuzz/fuzz_targets/<name>.rs` and a matching `[[bin]]` in
   `fuzz/Cargo.toml`.
3. Add a `proptest` property in `fuzz_test.rs` driving the same runner, so the
   new invariants are checked in CI too.
