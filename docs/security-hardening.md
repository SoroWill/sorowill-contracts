# Contract Security Hardening Guide

This guide covers security best practices for deploying and operating the
contracts in this repository. It is intended for maintainers, auditors, and
operators who deploy to production networks.

> Scope: this document is guidance only. It does not change contract behavior.
> Always review the actual contract source before deploying.

## 1. Security Checklist

Use this checklist before every production deployment.

### Access control
- [ ] Admin/owner roles are assigned to the intended addresses (multisig where possible).
- [ ] No privileged function is left callable by an unintended principal.
- [ ] Role transfer / renounce flows are tested and documented.
- [ ] Emergency pause (if present) is wired to a trusted operator.

### Input validation
- [ ] All public entry points validate arguments (amounts, addresses, IDs).
- [ ] Amounts reject zero/negative and overflow-prone values.
- [ ] Caller identity is checked where authorization is required.
- [ ] Error codes are returned consistently (see `docs/examples/error-handling.md`).

### State and funds
- [ ] State transitions are ordered to avoid partial updates on failure.
- [ ] Token transfers use the standard contract interfaces and check results.
- [ ] Re-entrancy / re-entry paths are reviewed for external calls.
- [ ] No funds can be locked by an unreachable code path.

### Configuration
- [ ] Network-specific constants (contract IDs, token addresses) are correct.
- [ ] No hardcoded secrets, private keys, or mnemonics in source or config.
- [ ] Dependencies and toolchain versions are pinned.

## 2. Audit Preparation

Prepare the following before engaging an auditor.

1. **Freeze scope** — tag the commit to be audited and record it in the audit request.
2. **Document architecture** — describe contracts, entry points, storage layout, and trust boundaries.
3. **List invariants** — write down properties that must always hold (e.g. total supply, balances).
4. **Provide test coverage** — include unit and integration tests, and note any gaps.
5. **Describe known risks** — call out accepted risks and out-of-scope areas explicitly.
6. **Prepare a threat model** — enumerate actors, assets, and attack surfaces.

### Audit request template

```
Repository:   <repo url>
Commit:       <sha / tag>
Contracts:    <list of contract crates>
Networks:     <testnet / mainnet>
Invariants:   <bulleted list>
Known risks:  <bulleted list>
Contact:      <maintainer>
```

Address audit findings in dedicated commits so reviewers can verify each fix.

## 3. Deployment Procedures

### Pre-deployment
1. Confirm the audited commit matches the build artifact.
2. Run the full test suite and record results.
3. Verify network configuration and funded deployer account.
4. Announce the deployment window to stakeholders.

### Deployment
1. Build the contract wasm from the pinned toolchain.
2. Record the wasm hash and compare against the audited build.
3. Deploy to testnet first and run smoke tests (see `docs/examples/`).
4. Deploy to mainnet using the documented deployer identity.
5. Initialize contracts and set admin roles in a single reviewed transaction batch.

### Post-deployment
1. Verify on-chain state: admin roles, initialized config, contract IDs.
2. Record deployed contract IDs and transaction hashes in the release notes.
3. Run a read-only smoke test against the live contract.
4. Keep the deployer keys in a secure, access-controlled location.

### Rollback
- Document how to pause or migrate if a critical issue is found.
- Never redeploy over live state without a migration plan.

## 4. Monitoring Setup

### What to monitor
- **Availability** — contract invocation success rate and latency.
- **Authorization** — unexpected admin/role changes or privileged calls.
- **Funds** — large transfers, balance anomalies, failed transfers.
- **Errors** — spikes in error codes returned by entry points.

### How to monitor
1. Subscribe to contract events / ledger entries for the deployed contract IDs.
2. Forward events to your logging/alerting stack.
3. Configure alerts for:
   - privileged function calls,
   - transfers above a threshold,
   - repeated failed invocations,
   - any change to admin configuration.
4. Keep an incident runbook with on-call contacts and escalation steps.

### Incident response
1. Detect and confirm the anomaly via logs and on-chain state.
2. Pause affected operations if a pause mechanism exists.
3. Notify stakeholders and, if needed, engage the auditor.
4. Document the incident, root cause, and remediation.

## Related documentation

- [Troubleshooting](./troubleshooting.md)
- [Contract performance tuning](./contract-performance-tuning.md)
- [Error handling examples](./examples/error-handling.md)
