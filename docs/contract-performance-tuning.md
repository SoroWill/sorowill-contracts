# Contract Performance Tuning Guide

This guide covers how to keep WillVault contract interactions fast and cheap: reducing gas, batching operations, load testing, and benchmarking. It applies to both the Soroban contracts and the client code that invokes them.

## Gas Optimization Tips

Soroban charges for CPU instructions, ledger reads/writes, and bandwidth. Most savings come from touching less state and doing less work per invocation.

- **Minimize storage reads and writes.** Each `env.storage()` access has a cost. Read a value once into a local variable and reuse it instead of calling `get` repeatedly.
- **Prefer temporary storage for short-lived data.** Use `temporary` for values that only need to live for a few ledgers, and `persistent` only for data that must survive indefinitely (e.g. will records).
- **Avoid unbounded loops.** Iterate over a bounded, known-size collection. If you must process many items, split the work across multiple invocations (see batching below).
- **Store compact types.** Use the smallest integer type that fits (`u32` vs `u64` vs `i128`) and avoid storing redundant fields that can be derived.
- **Cache computed values.** If a value is derived from storage and used multiple times in one call, compute it once.
- **Short-circuit validation.** Check cheap preconditions (auth, existence flags) before expensive storage lookups or cross-contract calls.
- **Avoid unnecessary cross-contract calls.** Each call adds overhead; batch related logic into a single entry point where it makes sense.
- **Reuse event data.** Emit one well-structured event rather than several small ones when the information belongs together.

## Batching Best Practices

Batching amortizes fixed per-invocation overhead (auth checks, transaction fees, ledger footprint setup) across many operations.

- **Group independent operations.** Batch operations that do not depend on each other's results so they can be processed in one pass.
- **Keep batches bounded.** Choose a batch size that keeps a single invocation well under the network's resource limits. A common starting point is tens of items, tuned by benchmarking.
- **Make batches idempotent where possible.** If a batch is retried after a partial failure, already-applied items should be safely skippable.
- **Fail fast or report per-item results.** Decide explicitly whether a batch is all-or-nothing or returns per-item status, and document it for callers.
- **Order by cost.** Process the cheapest or most likely-to-fail items first so invalid batches abort before doing expensive work.
- **Avoid mixing unrelated concerns.** Keep check-ins, claims, and admin operations in separate batches so a failure in one does not roll back the others.

## Load Testing Guide

Load testing verifies the contracts and clients behave correctly and within limits under realistic concurrency.

1. **Define the workload.** Model real usage: check-in frequency, claim volume, will creation rate, and read/write ratios.
2. **Start small.** Begin with a single account and a handful of operations to confirm correctness, then scale up.
3. **Use a dedicated test network.** Never load test against mainnet. Use a local or test network with representative ledger settings.
4. **Ramp concurrency gradually.** Increase concurrent accounts/transactions in steps and watch for error rates, latency, and resource-limit rejections.
5. **Measure the right signals.** Track success rate, p50/p95/p99 latency, CPU instructions per invocation, and ledger footprint size.
6. **Test failure paths.** Include expired wills, unauthorized callers, and duplicate check-ins to confirm error handling holds under load.
7. **Record and compare.** Save results per run so regressions are visible across contract changes.

## Benchmarking Methodology

Benchmarking produces repeatable numbers you can compare over time.

- **Isolate the operation.** Benchmark one entry point at a time with fixed inputs so results are comparable.
- **Warm up first.** Run a few iterations before measuring to avoid one-time setup skewing results.
- **Run multiple iterations.** Report the median and spread (min/max or percentiles) rather than a single sample.
- **Control the environment.** Use the same network settings, ledger state, and hardware for every run.
- **Measure resource usage, not just wall time.** Capture CPU instructions, read/write counts, and footprint size, since these drive fees.
- **Benchmark before and after.** Establish a baseline on the current code, then compare each change against it.
- **Document the setup.** Record contract version, inputs, batch sizes, and tooling so results can be reproduced.

## Quick Checklist

- [ ] Storage reads/writes minimized and cached locally
- [ ] Loops bounded; large workloads split into batches
- [ ] Batch size tuned by benchmark, not guessed
- [ ] Load test run on a test network with realistic workload
- [ ] Benchmarks recorded with baseline for comparison
