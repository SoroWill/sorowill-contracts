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
