# Security Policy

The SoroWill team takes the security of our smart contracts and protocol infrastructure seriously. As a protocol managing trustless inheritance and user funds on Stellar Soroban, we encourage responsible disclosure of any security vulnerabilities.

---

## Supported Versions

Only the latest code on the `main` branch and currently active deployments are actively supported for security updates:

| Version / Deployment | Supported | Notes |
| -------------------- | --------- | ----- |
| `main` branch        | :white_check_mark: | Primary codebase |
| Stellar Testnet      | :white_check_mark: | Deployed contract ID in [`deployments/testnet.json`](./deployments/testnet.json) |
| Older releases/tags  | :x: | Unsupported |

---

## Reporting a Vulnerability

**Do not report security vulnerabilities via public GitHub issues, discussions, or pull requests.**

If you discover a security vulnerability, please report it privately using one of the following methods:

1. **GitHub Private Vulnerability Reporting (Preferred)**:
   - Navigate to the [Security Advisories](https://github.com/SoroWill/sorowill-contracts/security/advisories) tab of the repository.
   - Click **"Report a vulnerability"** to submit a confidential report directly to the maintainers.

2. **Email Disclosure**:
   - If Private Vulnerability Reporting is unavailable, send your findings to `security@sorowill.org`.
   - Include a detailed description of the vulnerability, steps or proof-of-concept (PoC) code to reproduce it, and the potential impact on user funds or contract execution.

### Expected Response SLAs

- **Initial Acknowledgment**: Within **48 hours** of receiving your report.
- **Triage & Assessment**: Within **5 business days**, detailing validity, severity classification, and expected remediation timeline.
- **Fix & Public Disclosure**: Fixes will be coordinated and deployed prior to any public advisory release.

---

## Safe Harbor & Responsible Disclosure

If you conduct security research in good faith and follow these guidelines, we will:

- Consider your research to be authorized and refrain from taking legal action against you.
- Work with you to understand and remediate the issue promptly.
- Recognize your contribution in our security advisories and reward pool (where applicable).

### Rules of Engagement

- **Do not compromise user funds**: Test vulnerabilities against local Soroban test environments or mock data on Testnet. Do not attempt to drain live contract instances.
- **Maintain confidentiality**: Keep information about discovered vulnerabilities private until a fix has been released and coordinated disclosure is agreed upon.
- **Avoid disruptive testing**: Do not perform Denial of Service (DoS) attacks, spam transactions, or social engineering against maintainers.

---

## Scope

### In-Scope
- Smart contract source code under [`contracts/will/`](./contracts/will/)
- Storage layouts, authorization logic, and state transitions
- Inherited token handling and balance calculation logic

### Out-of-Scope
- Third-party protocols and external RPC providers
- Social engineering, phishing, or physical attacks
- Issues in un-deployed or experimental code branches

---

## Contract Security Hardening Guide

This section is the operational companion to the disclosure policy above. It
covers the checklist, audit preparation, deployment, and monitoring steps that
maintainers and integrators must follow when shipping or upgrading the SoroWill
contract.

### Security Checklist

Run through this list before every release and before any mainnet deployment:

- [ ] **Authorization**: Every state-changing entry point calls
      `require_auth` on the correct address (owner, claimant, or admin). No
      privileged path is reachable without an auth check.
- [ ] **Reentrancy / ordering**: State is written (e.g. `claimed` flags,
      balances) *before* any token transfer or external call.
- [ ] **Arithmetic**: All balance and time math uses checked operations
      (`checked_add`, `checked_sub`, `checked_mul`) and fails closed on
      overflow/underflow.
- [ ] **Commitments**: Hashed beneficiaries are stored as
      `sha256(beneficiary_address.to_xdr() || salt)` with a random salt of at
      least 16 bytes; the pre-image is validated against the `claimant`
      argument.
- [ ] **Access control**: Admin-only functions (pause, upgrade, config) are
      gated and documented; no default or hardcoded admin keys.
- [ ] **Input validation**: Amounts, intervals, and beneficiary lists are
      validated (non-zero, bounded length, sane time windows).
- [ ] **Storage**: TTLs are extended for long-lived entries; no unbounded
      iteration over user-controlled collections.
- [ ] **Events**: All state transitions emit events with enough data for
      off-chain monitoring.
- [ ] **Dependencies**: `Cargo.lock` is committed and `cargo audit` reports no
      known advisories.
- [ ] **Tests**: Unit and integration tests cover happy paths, auth failures,
      replay attempts, and boundary conditions; `cargo test` is green.

### Audit Preparation

Before engaging an external auditor:

1. **Freeze scope**: Tag the commit to be audited and record the exact
   contract ID(s) and storage layout in the audit request.
2. **Document invariants**: Write down the protocol invariants (e.g. "a slot
   can be claimed at most once", "sum of allocations never exceeds the
   deposited balance") so auditors can target them.
3. **Provide a threat model**: List assets at risk, trusted roles, and known
   risks (see below) with their mitigations.
4. **Reproducible builds**: Ensure `cargo build --target wasm32-unknown-unknown
   --release` produces a deterministic artifact and record its hash.
5. **Test coverage**: Share the test suite and any fuzzing harnesses; note
   areas with intentionally limited coverage.
6. **Triage channel**: Designate a private channel for findings and agree on
   severity classification (Critical / High / Medium / Low).

### Deployment Procedures

1. **Build**: Compile the release WASM from a clean checkout of the tagged
   commit and record the artifact hash.
2. **Verify**: Confirm the hash matches CI output and that `cargo test` passed
   on the same commit.
3. **Dry run on Testnet**: Deploy to Testnet first, exercise create / check-in /
   claim / cancel flows, and confirm events are emitted as expected.
4. **Deploy**: Install the WASM and deploy/upgrade the contract instance using
   the admin key held in the approved signer setup (never a personal key).
5. **Initialize**: Set configuration and admin roles, then verify on-chain
   state matches the intended configuration.
6. **Record**: Update [`deployments/testnet.json`](./deployments/testnet.json)
   (and the mainnet equivalent) with the new contract ID and artifact hash.
7. **Announce**: Publish the deployment notes and any migration steps for
   integrators.

### Monitoring Setup

- **Event streaming**: Subscribe to contract events (create, check-in, claim,
  cancel, upgrade) and alert on unexpected or high-value claims.
- **Balance watch**: Track the contract's token balance against the sum of
  outstanding allocations; alert on any divergence.
- **Failure alerts**: Monitor for spikes in failed invocations, which can
  indicate replay attempts or misconfigured clients.
- **Admin actions**: Alert on any admin/upgrade invocation and require
  out-of-band confirmation.
- **Health checks**: Periodically read contract state (TTL, config) and alert
  before storage entries approach expiration.

---

## Known Risks & Mitigations

### Replay of `reveal_and_claim` (#440)

A hashed beneficiary claims by revealing a pre-image of the commitment stored
on the will. That pre-image is public once the claim transaction is submitted,
so without extra checks an observer could replay it or front-run it. The
contract mitigates this as follows:

- **One claim per slot.** Each hashed slot has a `claimed` flag, which is set
  and persisted in the same invocation that pays out, before the token
  transfer. Any later call for that slot fails with `AlreadyClaimed`.
- **Pre-image bound to the recipient.** The pre-image must be
  `claimant.to_xdr() || salt`. The contract checks the prefix against the
  `claimant` argument, and `claimant` must authorize the call. Someone who
  copies the pre-image cannot redirect the payout to their own address.
- **Signed invocations cannot be replayed.** Soroban authorization entries
  carry a nonce and an expiration ledger.

Integrators must build commitments as
`sha256(beneficiary_address.to_xdr() || salt)` with a random salt of at least
16 bytes. A commitment built any other way can never be claimed.

---

## Rewards & Stellar Wave Program

Resolutions for verified security vulnerabilities reported under this policy are eligible for recognition and reward distribution through the **Stellar Wave Program** on Drips. Point allocations are determined based on severity classification (Critical, High, Medium, Low).
