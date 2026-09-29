# WASM binary size

Soroban charges resource fees partly based on contract code size, and a
smaller binary is cheaper to deploy and marginally cheaper to invoke (less
code to parse/instantiate per call). This document records the release
profile tuning for the `will` contract and how to measure its effect.

## Size budget

The compiled `will.wasm` release binary MUST NOT exceed **150,000 bytes** (150 KB). CI automatically enforces this budget on every commit and pull request.

## Current release profile

`Cargo.toml`'s `[profile.release]` (reviewed for this issue):

| Setting | Value | Why |
|---|---|---|
| `opt-level` | `"z"` | Optimizes for minimum code size, at some cost to CPU-instruction count versus `"s"` or the default `3`. Soroban bills both code size (deploy) and CPU instructions (invoke); for this contract's simple arithmetic and storage-bound entry points, the size savings dominate. |
| `lto` | `true` | Full link-time optimization enables cross-crate inlining and dead-code elimination that per-crate compilation can't do — the single biggest lever for size on small contracts. |
| `codegen-units` | `1` | A single codegen unit is required for LTO to fully do its job; it disables parallel codegen (slower local builds) in exchange for the compiler seeing the whole crate at once. |
| `panic` | `"abort"` | Removes unwinding tables and landing pads. Soroban's `no_std` contract target has no unwind runtime to support anyway, so this only removes dead weight. |
| `strip` | `"symbols"` | Debug/symbol info is never needed in the deployed wasm. |
| `overflow-checks` | `true` | Kept **on** deliberately even though it costs some size: silent `i128`/`u64` wraparound in balance or period arithmetic is a correctness/security risk, and the size cost of checked arithmetic is small relative to the safety it buys. This is a conscious trade-off, not an oversight. |

All four settings named in the acceptance criteria for this issue
(`opt-level`, `lto`, `codegen-units`, `panic = "abort"`) were already
present before this review; this pass re-verified each one is still
appropriate for the current `no_std` Soroban target and documented the
reasoning above so future changes don't regress it silently.

## Measuring the binary size

```bash
cargo build -p will --release --target wasm32v1-none
ls -la target/wasm32v1-none/release/will.wasm

# For a size breakdown by symbol/section (optional, requires `twiggy`):
cargo install twiggy
twiggy top target/wasm32v1-none/release/will.wasm | head -30
```

When changing `[profile.release]` in a future PR, record the before/after
`.wasm` file size (in bytes) in the PR description, e.g.:

```
Before: 24,731 bytes
After:  21,408 bytes  (-13.4%)
```

## Confirming no behavior change

Profile changes must not change contract behavior. After any
`[profile.release]` edit, run:

```bash
cargo test --workspace
cargo clippy --all-targets -- -D warnings
cargo build --workspace --release --target wasm32v1-none
```

All three must pass exactly as they did before the change — a smaller
binary is only a win if it's still correct.
