# Contract spec artifacts

This directory holds a versioned, machine-readable export of the `will`
contract's public interface: every entry-point function signature, and the
`Will`, `Beneficiary`, `Guardian`, `WillStatus`, and `WillError` types as
they are actually compiled into the deployed WASM.

Consumers such as [`sorowill-sdk`](https://github.com/SoroWill/sorowill-sdk)
diff their hand-maintained TypeScript types and XDR encoders against this
file to detect spec drift before it ships.

## Files

- `will-v<crate-version>.json` — one file per released crate version, named
  after the `version` field in `contracts/will/Cargo.toml` at the time of
  export (e.g. `will-v0.1.0.json`). Never overwritten once published;
  a new crate version gets a new file.

## How the artifact is produced

The JSON is derived from the compiled contract's on-chain spec, which is
embedded in the WASM as an `SCSpecEntry` XDR stream by `#[contract]` /
`#[contractimpl]` / `#[contracttype]` / `#[contracterror]`. The canonical
way to extract it is the Stellar CLI's spec-export tooling:

```bash
scripts/export-spec.sh
scripts/export-spec.sh --check
```

The script uses `stellar contract build` and writes the contract's
`SCSpecEntry` values as a JSON array.

## Update process

1. Bump `version` in `contracts/will/Cargo.toml` as part of any PR that
   changes a public entry-point signature, `Will`/`Beneficiary`/`Guardian`/
   `WillStatus`, or `WillError`.
2. Run `scripts/export-spec.sh` locally (or let the `Spec Export` CI
   workflow do it) to regenerate `spec/will-v<new-version>.json`.
3. Commit the new file alongside the code change — do not edit or delete
   previously published spec files, they are the historical record SDK
   consumers diff against.
4. On tagged releases, the same JSON file is additionally attached to the
   GitHub Release as a downloadable artifact by
   `.github/workflows/spec-export.yml`.

This is documented in the top-level [README](../README.md#contract-spec-artifact)
as well.
