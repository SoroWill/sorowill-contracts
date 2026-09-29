---
name: Action symbol decode test
about: Add a table-driven test that asserts every record_transition action symbol decodes to its expected canonical string
labels: testing, medium
---

## Summary

`record_transition` is called with `symbol_short!(…)` literals at seven distinct call sites in `lib.rs`.
`symbol_short!` enforces a ≤ 9-character limit at compile time, so oversized strings are caught by the compiler.
However, **no test asserts what those strings actually say**.
A future PR could introduce a typo—e.g. `symbol_short!("relase")` instead of `"release"`—which would compile and pass all existing tests but silently corrupt every on-chain audit trail entry written by that code path.

---

## Background

### `record_transition` helper (bottom of `lib.rs`)

```rust
fn record_transition(
    env: &Env,
    will_id: u64,
    from_status: WillStatus,
    to_status: WillStatus,
    actor: &Address,
    action: soroban_sdk::Symbol,   // ← the symbol stored on-chain
) {
    let transition = WillStatusTransition { will_id, from_status, to_status,
        timestamp: env.ledger().timestamp(), actor: actor.clone(), action };
    storage::append_history(env, will_id, &transition);
}
```

### Current call sites

| Call site (function)      | Raw string literal        | Expected decoded string |
|---------------------------|---------------------------|-------------------------|
| `create_will`             | `symbol_short!("create")` | `"create"`              |
| `trigger_will`            | `symbol_short!("trigger")`| `"trigger"`             |
| `emergency_checkin`       | `symbol_short!("emerg")`  | `"emerg"`               |
| `release_inheritance`     | `symbol_short!("release")`| `"release"`             |
| `cancel_will`             | `symbol_short!("cancel")` | `"cancel"`              |
| `guardian_trigger` path   | `symbol_short!("gtrigr")` | `"gtrigr"`              |
| `guardian_cancel` path    | `symbol_short!("gcancel")`| `"gcancel"`             |

---

## Acceptance Criteria

1. A new `#[test]` function (or an extension of the existing history tests in `test.rs`) enumerates **all seven** call-site symbols and asserts each one converts to its expected `&str` via `Symbol::to_string` / `as_str` (whichever the Soroban SDK exposes in the test environment).
2. The test is **table-driven** so adding a new call site in future is a one-liner change to the table.
3. `cargo test --workspace` passes with no warnings promoted to errors.
4. `cargo clippy --all-targets -- -D warnings` passes.

---

## Suggested Implementation

Add a new test in `contracts/will/src/test.rs` (alongside the existing `test_will_history_*` tests):

```rust
/// Table-driven test: every action symbol used in record_transition must decode
/// to its expected canonical string.  A typo in a symbol_short!(…) literal will
/// compile fine but silently corrupt on-chain audit history; this test catches it.
#[test]
fn test_action_symbol_table() {
    use soroban_sdk::Env;

    let env = Env::default();

    // (symbol_short!(…), expected &str)
    let cases: &[(soroban_sdk::Symbol, &str)] = &[
        (symbol_short!("create"),  "create"),
        (symbol_short!("trigger"), "trigger"),
        (symbol_short!("emerg"),   "emerg"),
        (symbol_short!("release"), "release"),
        (symbol_short!("cancel"),  "cancel"),
        (symbol_short!("gtrigr"),  "gtrigr"),
        (symbol_short!("gcancel"), "gcancel"),
    ];

    for (sym, expected) in cases {
        // Symbol::to_string returns a String in the test environment.
        assert_eq!(
            sym.to_string(),
            *expected,
            "action symbol mismatch: got {}, expected {}",
            sym.to_string(),
            expected
        );
    }
}
```

> **Note:** If `Symbol::to_string` is not directly available, use the `soroban_sdk` conversion helper that returns a `String` or `&str` in the test environment. The exact API may need a quick check against the SDK version pinned in `Cargo.toml`.

---

## Why This Matters

- **On-chain audit trail** — `WillStatusTransition.action` is persisted to ledger storage and read back by clients/UIs that display a human-readable event history. A corrupted symbol makes that history unintelligible.
- **Cheap to fix, expensive to discover** — the bug only surfaces by manually inspecting on-chain data or reading a log; automated tests won't catch it today.
- **Low effort** — the test is essentially a constant table; it does not require any contract deployment or mock setup.

---

## CI Commands to Verify

```bash
# From repo root
cargo test --workspace
cargo clippy --all-targets -- -D warnings
```

Both must exit 0 before the PR is mergeable.
