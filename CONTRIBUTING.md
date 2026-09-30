# Contributing to sorowill-contracts

This repo participates in the **Stellar Wave Program** on [Drips](https://drips.network/wave). Contribution work is tied to issues that maintainers tag for an active Wave, and contributors earn rewards proportional to the Points assigned to the issues they resolve.

## Ground rules

- **Do not start work on any issue until you have been assigned by the maintainer.** Applying to an issue does not mean you're assigned — wait for confirmation (via the Drips Wave dashboard or a direct assignment on GitHub) before opening a PR.
- Keep PRs scoped to the issue they resolve. Unrelated changes slow down review and can cost you the Wave window.
- Be responsive during an active Wave — issues must be resolved before the Wave ends for Points to be awarded.

## Branch naming

Use the issue number in your branch name:

```
feat/N-short-description
fix/N-short-description
```

For example: `feat/42-guardian-quorum-check` or `fix/17-checkin-deadline-rounding`.

## Pull requests

- Your PR description must reference the issue it resolves (e.g. `Closes #42`).
- Complete the [Before opening a PR](#before-opening-a-pr) checklist before requesting review.
- Add or update unit tests for any behavior change in `contracts/will/src/test.rs`.
- If you change validation rules or add an entry point, extend the fuzzing
  harness too — see [docs/FUZZING.md](./docs/FUZZING.md#adding-a-target).
- If your PR changes contract behavior (new entrypoint, validation change,
  event/error schema change, etc.), add an entry under `[Unreleased]` in
  [CHANGELOG.md](./CHANGELOG.md) describing it in the "Added" / "Changed" /
  "Fixed" section that fits. Pure docs/tooling/test-only PRs don't need one.
- If your PR adds a new `WillError` variant, add a row for it to the
  [error code table in the README](./README.md#error-codes) in the same PR —
  don't let the table and `contracts/will/src/errors.rs` drift apart.
- If your PR changes a public entry-point signature or a shared type
  (`Will`, `Beneficiary`, `Guardian`, `WillStatus`, `WillError`, ...), bump
  the crate `version` in `contracts/will/Cargo.toml` and regenerate the
  contract spec artifact — see [spec/README.md](./spec/README.md).

## Contract versioning

`CONTRACT_VERSION` in `contracts/will/src/lib.rs` is a `u32` constant that is
compiled into the deployed Wasm and returned by the `get_contract_version`
entry point. Because the value is baked into the binary, it is **immutable
after deployment**: once a contract is deployed, its on-chain version can only
change by deploying a new binary. SDKs and integrators rely on this value to
detect behavioral changes, so the constant in source must always match the
version of the binary that is actually deployed.

### Bumping the version

Bump `CONTRACT_VERSION` in the **same PR** as any change that alters contract
behavior — a new entry point, a validation change, an event/error schema
change, or a storage schema change. Do not bump it for docs, tooling, or
test-only changes.

1. Increment `CONTRACT_VERSION` in `contracts/will/src/lib.rs`.
2. Bump the crate `version` in `contracts/will/Cargo.toml` to match.
3. Add a `[Unreleased]` entry in [CHANGELOG.md](./CHANGELOG.md) describing the
   behavioral change.
4. Regenerate the contract spec artifact — see [spec/README.md](./spec/README.md).
5. Tag the release with a matching git tag (e.g. `v1.2.0`) so the deployed
   binary can be traced back to the source that produced it.

### Keeping source and deployment in sync

A build-time check (`.github/scripts/check-contract-version.sh`) verifies
that `CONTRACT_VERSION` matches the crate `version` in
`contracts/will/Cargo.toml` and, when a git tag is present, the tag itself.
Run it locally before opening a PR:

```sh
./.github/scripts/check-contract-version.sh
```

CI runs the same check on every PR (see the "Verify CONTRACT_VERSION matches
git version tag" step in
[.github/workflows/test.yml](./.github/workflows/test.yml)), so a PR that
changes contract behavior without bumping `CONTRACT_VERSION`
fails before it can be merged. This prevents the source constant from drifting
away from the deployed contract version when deployment steps are skipped.

## Storage schema versioning

Every persisted `Will` has a `schema_version` field. Soroban decodes a
`#[contracttype]` struct strictly by field name. If you add, remove or rename
a field on `Will`, every will stored by an older contract version stops
decoding unless you also ship a migration path. The machinery is in
[`contracts/will/src/migration.rs`](./contracts/will/src/migration.rs):

- `CURRENT_SCHEMA_VERSION` is the version stamped on every newly created or
  migrated will.
- `decode_will` handles every read of a `DataKey::Will` entry. It tries the
  current layout first, then each legacy layout, and converts a legacy entry
  into the current `Will` shape. The converted will keeps its old
  `schema_version`.
- `upgrade` runs the `migrate_vN_to_vN+1` steps in order up to
  `CURRENT_SCHEMA_VERSION`. The owner-authorized `migrate_will` entry point
  calls it and persists the result.

**Version history**

| Version | Change |
|---|---|
| 0 | Wills written before `schema_version` was populated. |
| 1 | Current layout. |

**To introduce version N+1** (for example, adding a field to `Will`):

1. Copy the current `Will` definition unchanged into a legacy type, e.g.
   `WillV1` in a `legacy` module, keeping `#[contracttype]` and the original
   field names. This is the only way old entries can still be decoded.
2. Change `Will` (add the field) and bump `CURRENT_SCHEMA_VERSION` to N+1.
3. In `decode_will`, add a fallback that tries `WillVN::try_from_val` and
   converts it into `Will`. Give new fields a safe default and leave
   `schema_version` at N.
4. Add a `migrate_vN_to_vN1` step and wire it into the `match` in `upgrade`.
   Every version below `CURRENT_SCHEMA_VERSION` needs an arm.
5. Add a row to the version history table above and a CHANGELOG entry.
6. Add a test that writes a `WillVN` directly under `DataKey::Will(id)`,
   loads it through `get_will`, and calls `migrate_will` to confirm it ends up
   at N+1 with the new field set.

Never reorder, rename or retype an existing field without going through these
steps. Entry points must also accept wills that have not been migrated yet,
since `migrate_will` is opt-in.

## Before opening a PR

Run every command used by the [Test CI workflow](./.github/workflows/test.yml) and confirm it succeeds:

- [ ] `cargo fmt --all -- --check`
- [ ] `cargo clippy --all-targets -- -D warnings`
- [ ] `cargo test --workspace`
- [ ] `cargo build --workspace --release --target wasm32v1-none`
- [ ] `./.github/scripts/check-contract-version.sh`
- [ ] Confirm the Test workflow is green on the PR.

## Local setup

See the [README](./README.md#local-setup) for toolchain installation and how to run the test suite.

## Task runner (just)

This repo ships a [`justfile`](./justfile) with shortcuts for the commands
above, so you don't have to remember or copy them by hand. Install
[`just`](https://github.com/casey/just#installation), then:

```bash
just --list    # see every available recipe
just lint      # cargo clippy --all-targets -- -D warnings
just test      # cargo test --workspace
just build     # cargo build --workspace --release --target wasm32v1-none
just fmt       # cargo fmt --all
just ci        # runs lint, test, and build in order — the full "Before opening a PR" checklist above in one command
```

If you don't have `just` installed, the raw `cargo` commands in
[Before opening a PR](#before-opening-a-pr) work identically.

## Cargo.lock policy

`Cargo.lock` is committed at the workspace root, and that's intentional:
every crate in this workspace is built and deployed as a Soroban
**contract** (compiled to a `wasm32v1-none` binary and deployed on-chain),
not published as a library for other crates to depend on via crates.io.
Committing the lockfile gives reproducible CI builds and deployments — the
same dependency graph every time, regardless of what's newest on crates.io
the day CI happens to run.

This does **not** constrain anyone who depends on a crate from this
workspace (e.g. `contracts/will`) as a path or git dependency in their own
project: Cargo resolves and locks dependencies per top-level workspace, so a
downstream consumer's own `Cargo.lock` governs their build, not this one.
Our committed lockfile only pins builds performed *inside this repository*
(CI, local `cargo test`/`cargo build`, `scripts/export-spec.sh`, etc.).

If this workspace ever adds a crate meant to be published to crates.io as a
reusable library (as opposed to an on-chain contract), revisit this policy
for that crate specifically — published library crates conventionally do
**not** commit `Cargo.lock`, so their consumers can resolve compatible
dependency versions themselves rather than inheriting exact pins.

## Mutation testing (cargo-mutants)

Code coverage alone can't tell you whether an assertion is too weak or an
edge case is missing — it only tells you a line ran, not that a test would
notice if the logic on that line broke. [`cargo-mutants`](https://mutants.rs/)
closes that gap: it systematically rewrites small pieces of the contract
(flipping a comparison, changing a constant, swapping a boolean) and reruns
the test suite against each mutant. A mutant that **survives** (tests still
pass) means no test would have caught that bug.

There is currently no CI workflow for mutation testing: it is a manual,
advisory check you run locally (see below). A survived mutant is a signal to
add a test, not a build failure.

### Running it locally

```sh
cargo install cargo-mutants --locked
cargo mutants --package will
```

This takes a while: cargo-mutants recompiles and reruns the test suite once
per candidate mutation. To scope a run while iterating on a single file:

```sh
cargo mutants --package will --file contracts/will/src/storage.rs
```

Results are written to `mutants.out/` (or wherever `--output` poi

/* … truncated 5094 chars — edit only what you need near the top … */
