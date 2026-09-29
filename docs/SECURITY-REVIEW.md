# Security Review Report — SoroWill Contract

**Date:** 2026-07-27
**Contract version:** 1.0.0 (`CONTRACT_VERSION = 1_000_000`, git ref: `main` as of 2026-07-27)
**Scope:** `contracts/will/src/` — full contract source
**Reviewer:** Automated review pass
**Methodology:** Manual code review covering reentrancy, authorization bypass, integer overflow/underflow, and general Soroban security best practices.

> **Staleness note:** This review was performed against contract version **1.0.0**. After any
> change that alters observable contract behaviour (new entry points, changed authorization
> paths, modified payout logic, or bumped `CONTRACT_VERSION`), a follow-up review should be
> scheduled. To request a re-review, open an issue tagged `security-review` describing the
> changes since this document's last update date, or contact the maintainers via
> [`SECURITY.md`](../SECURITY.md).

---

## 1. Reentrancy Risk

### Finding: No reentrancy risk

Soroban transactions execute atomically — a contract invocation cannot be interrupted by another invocation. Cross-contract calls (e.g. `token::Client::transfer`) are synchronous within the same transaction. This eliminates the classic reentrancy attack vector found in EVM contracts.

**State updates around token transfers:**

| Function | Transfer | State Update | Safe? |
|---|---|---|---|
| `distribute()` | Transfers to each beneficiary | `will.balance = 0`, `status = Released` after all transfers | Yes — atomic |
| `cancel_will()` | Refunds to owner | `will.balance = 0`, `status = Cancelled` after transfer | Yes — atomic |
| `create_will()` | Transfers from owner to contract | Will saved after transfer | Yes — atomic |
| `top_up()` | Transfers from owner to contract | Balance incremented after transfer | Yes — atomic |

**Verdict:** No reentrancy vulnerabilities.

---

## 2. Authorization Bypass Paths

### 2.1 Auth-checked functions (owner)

| Function | Auth Required | Implementation |
|---|---|---|
| `create_will` | `owner.require_auth()` | Correct |
| `check_in` | `owner.require_auth()` | Correct |
| `emergency_checkin` | `owner.require_auth()` + ownership check | Correct |
| `cancel_will` | `owner.require_auth()` + ownership check | Correct |
| `update_beneficiaries` | `owner.require_auth()` + ownership check | Correct |
| `update_guardians` | `owner.require_auth()` + ownership check | Correct |
| `top_up` | `owner.require_auth()` + ownership check | Correct |

### 2.2 Auth-checked functions (guardian)

| Function | Auth Required | Implementation |
|---|---|---|
| `guardian_trigger` | `guardian.require_auth()` + guardian list check + vote dedup | Correct |

### 2.3 Intentionally permissionless functions

| Function | Auth Required | Rationale |
|---|---|---|
| `trigger_will` | None | Off-chain keeper pattern; anyone can trigger after deadline |
| `release_inheritance` | None | Anyone can finalize after grace period expires |
| `archive_will` | None | Settled wills can be archived by anyone to reduce storage |
| `get_will` | None | Read-only query |
| `get_wills_by_owner` | None | Read-only query |
| `get_wills_by_beneficiary` | None | Read-only query |
| `get_will_history` | None | Read-only query |

### 2.4 Ownership verification

`load_owned()` (`lib.rs:511-518`) loads the will and asserts `will.owner == caller`. This is used by all owner-gated functions. The check is correct and cannot be bypassed because Soroban's `require_auth()` cryptographically verifies the signer.

**Verdict:** No authorization bypass vulnerabilities.

---

## 3. Integer Overflow / Underflow

### 3.1 Release profile

`Cargo.toml` line 10: `overflow-checks = true` — Rust panics on arithmetic overflow in all profiles (including release). This is the primary defense.

### 3.2 Arithmetic operations audit

| Location | Operation | Risk |
|---|---|---|
| `lib.rs:152` | `now + will.checkin_period_days * SECONDS_PER_DAY` | u64 overflow if period > ~21M years — impossible in practice |
| `lib.rs:171` | `will.last_checkin + will.checkin_period_days * SECONDS_PER_DAY` | Same as above |
| `lib.rs:178` | `now + will.grace_period_days * SECONDS_PER_DAY` | Same as above |
| `lib.rs:212` | `trigger_time + will.grace_period_days * SECONDS_PER_DAY` | Same as above |
| `lib.rs:394` | `will.balance += amount` | i128 overflow protected by release profile |
| `lib.rs:553` | `total * (beneficiary.percentage as i128) / 100` | i128 overflow protected; `percentage` ≤ 100, so max multiplier is 100x |
| `lib.rs:530` | `total += beneficiary.percentage` | u32 sum; max 10 entries × 100 = 1000, well within u32 |
| `storage.rs:48` | `current + 1` | u64 counter; would require ~584 billion years at 1 will/ledger to overflow |

### 3.3 Percentage validation

`assert_valid_percentages()` (`lib.rs:527-535`) iterates all beneficiaries and checks the sum equals exactly 100. This prevents:
- Under-allocation (sum < 100)
- Over-allocation (sum > 100)
- Zero-balance distribution issues

**Verdict:** No overflow/underflow vulnerabilities. All arithmetic is protected by `overflow-checks = true`.

---

## 4. Additional Findings

### 4.1 Token Contract Trust (Low — Informational)

**Location:** `lib.rs:98`

`create_will` accepts an arbitrary `token` address without validating it is a legitimate SEP-41 contract. If a non-token address is provided, the `transfer` call will fail, causing the transaction to revert.

**Impact:** No security impact — the transaction simply fails. However, users could waste transaction fees on failed attempts.

**Recommendation:** Consider validating the token at creation (e.g. calling a small transfer or checking contract metadata), though this adds gas cost and may not be worth it for the current use case.

### 4.2 Guardian Collusion (Low — By Design)

**Location:** `lib.rs:460-469`

Two guardians out of three can force an immediate release of all funds, bypassing the check-in mechanism entirely. This is the intended design for handling an incapacitated owner.

**Impact:** If two guardians collude (or are compromised), they can steal the will's balance.

**Recommendation:** This is an accepted trust assumption. The SDK/app should clearly communicate this risk when users configure guardians.

### 4.3 Front-Running (Low — By Design)

**Location:** `lib.rs:166-191`, `lib.rs:247-271`

`trigger_will` and `release_inheritance` are permissionless. An attacker could front-run the legitimate keeper to call these functions first.

**Impact:** None — the outcome is identical regardless of who calls these functions. The attacker gains nothing.

### 4.4 Duplicate Beneficiary Address (Low — Informational)

**Location:** `lib.rs:101-103`, `lib.rs:291-293`

A will can be created with the same address appearing multiple times in the beneficiary list with different percentages. This is valid but potentially confusing for users.

**Impact:** Functionally correct — the `distribute()` function iterates all entries, so each entry gets its share independently. The total percentage is still validated to be 100.

**Recommendation:** Consider deduplicating or warning at the SDK level.

### 4.5 Will ID Enumeration (Low — Informational)

**Location:** `lib.rs:404-406`

Will IDs are sequential starting from 1. An attacker could enumerate all wills by calling `get_will(id)` for ids 1, 2, 3, ...

**Impact:** All will data is already public on-chain (owner, beneficiaries, balance, status). Enumeration does not expose additional information.

### 4.6 Storage Exhaustion on Queries (Low — Informational)

**Location:** `lib.rs:409-430`

`get_wills_by_owner` and `get_wills_by_beneficiary` return all wills for an address. If an owner creates many wills, these queries could become expensive and potentially exceed transaction resource limits.

**Impact:** Limited by Soroban's transaction resource limits. A single query cannot exhaust chain-wide storage.

**Recommendation:** Consider adding pagination parameters for production use.

---

## 5. Summary

| Category | Severity | Count | Status |
|---|---|---|---|
| Reentrancy | Critical | 0 | N/A |
| Authorization Bypass | Critical | 0 | N/A |
| Integer Overflow/Underflow | Critical | 0 | N/A |
| Token Trust | Low | 1 | Accepted |
| Guardian Collusion | Low | 1 | By Design |
| Front-Running | Low | 1 | By Design |
| Duplicate Beneficiaries | Low | 1 | Informational |
| Will ID Enumeration | Low | 1 | Informational |
| Query Exhaustion | Low | 1 | Informational |

**Overall Assessment:** The contract is secure against all critical vulnerability classes. No high-severity issues were found. All low-severity findings are either by design, informational, or accepted trade-offs.

---

## 6. Recommendations

1. **Run formal audit** before mainnet deployment — this review is a best-effort manual pass, not a substitute for a professional audit.
2. **Consider token validation** at creation time to prevent user confusion (optional, low priority).
3. **Add pagination** to `get_wills_by_owner` and `get_wills_by_beneficiary` for production SDK integration.
4. **Document guardian trust assumptions** clearly in user-facing documentation.
