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

## Contract Security Audit Checklist

Every release of the `will` contract must pass the following review before deployment. Each item should be marked `[x]` (verified), `[ ]` (not yet reviewed), or `N/A` with a justification.

### Access Control & Authorization
- [ ] Every state-mutating entry point calls `require_auth` on the correct principal (owner, beneficiary, or claimant).
- [ ] Admin/owner-only functions (e.g. configuration, upgrades) are gated and cannot be invoked by arbitrary callers.
- [ ] Authorization is checked against the *actual* caller, not a caller-supplied address argument.
- [ ] No privileged action can be triggered indirectly through a helper that skips the auth check.

### Arithmetic Safety
- [ ] All balance and allocation arithmetic uses checked operations (`checked_add`, `checked_sub`, `checked_mul`) or is provably bounded.
- [ ] Token amounts and percentages cannot overflow, underflow, or be truncated by integer division.
- [ ] Sum of beneficiary allocations equals the total deposited amount (no dust left unassigned or double-counted).
- [ ] Rounding behavior is documented and cannot be exploited to extract more than deposited.

### State Transitions
- [ ] The will lifecycle (created → active → claimed/expired) is enforced; invalid transitions revert.
- [ ] Terminal states (fully claimed, cancelled) cannot be re-entered or mutated.
- [ ] `reveal_and_claim` sets the per-slot `claimed` flag and persists it *before* the token transfer (see Known Risks).
- [ ] Reentrancy / re-entrant token callbacks cannot corrupt state or double-pay.

### Events & Observability
- [ ] Every state-changing operation emits an event with the relevant identifiers and amounts.
- [ ] Events are emitted after state is finalized so off-chain indexers observe consistent data.
- [ ] No sensitive pre-images or secrets are leaked in event payloads beyond what is already public on-chain.

### Upgrade & Versioning
- [ ] Storage keys and layouts are versioned; migrations are idempotent and tested.
- [ ] Contract `wasm` hash / version is recorded in `deployments/` for each deployment.
- [ ] Upgrade path preserves existing will state and cannot orphan user funds.

---

## Audit Findings Log

All findings from internal reviews and external audits are recorded here. Severity follows the standard **Critical / High / Medium / Low / Informational** scale.

| ID | Date | Severity | Area | Location | Description | Status |
| -- | ---- | -------- | ---- | -------- | ----------- | ------ |
| F-001 | YYYY-MM-DD | High | Access Control | `contracts/will/src/lib.rs` | _Example: missing auth on config setter_ | Resolved |
| F-002 | YYYY-MM-DD | Medium | Arithmetic | `contracts/will/src/lib.rs` | _Example: unchecked subtraction on balance_ | Open |

**Status values:** `Open` → `In Progress` → `Resolved` → `Verified` (or `Won't Fix` with justification).

---

## Remediation Tracking

Each finding above is tracked to closure. A finding may only be marked `Verified` once a reviewer other than the fix author confirms the remediation.

| Finding | Owner | Status | Fix PR / Commit | Target Date | Verified By |
| ------- | ----- | ------ | --------------- | ----------- | ----------- |
| F-001 | @owner | Verified | #000 | YYYY-MM-DD | @reviewer |
| F-002 | @owner | In Progress | — | YYYY-MM-DD | — |

**Remediation rules:**
- Every `Critical`/`High` finding must be `Verified` before the next deployment.
- `Won't Fix` decisions require written justification and maintainer sign-off.
- Re-test the full checklist after remediation to confirm no regressions.

---

## Final Audit Report

A completed audit is summarized using the template below and stored alongside the release.

```
# SoroWill Contract Audit Report

- Audit date:        YYYY-MM-DD
- Auditor(s):        <names / firm>
- Commit / wasm hash: <git sha / wasm hash>
- Scope:             contracts/will/ (and any listed dependencies)

## Methodology
<Manual review, static analysis tooling, unit/fuzz tests, threat modeling.>

## Summary of Findings
| Severity | Count | Resolved |
| -------- | ----- | -------- |
| Critical | 0     | 0        |
| High     | 0     | 0        |
| Medium   | 0     | 0        |
| Low      | 0     | 0        |
| Info     | 0     | 0        |

## Detailed Findings
<Reference F-xxx entries from the Audit Findings Log.>

## Conclusion
<Overall risk assessment and whether the contract is fit for deployment.>

## Sign-off
- Auditor:  ____________________  Date: __________
- Maintainer: ________________  Date: __________
```

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
