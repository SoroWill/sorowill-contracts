# Contract risk assessment

Use the data-only framework in [`audit.toml`](../audit.toml), under
`audit.risk`, for each release candidate and whenever a listed review trigger
occurs. It complements the [security checklist](SECURITY_AUDIT_CHECKLIST.md)
and [security policy](../SECURITY.md); it is not an external audit, a claim
that the contracts are safe, or an automated deployment gate.

## Assessment method

1. **Freeze the scope.** Record the commit and candidate Wasm hashes, contract
   addresses/network (if deployed), reviewer/date, assets at risk, entry
   points, token/whitelist dependencies, privileged roles and exclusions.
   Cover both workspace contracts and their trust boundaries. Exclusions
   need a rationale and must remain visible in the deployment decision.
2. **Identify scenarios.** Review all eight categories in `audit.risk.categories`
   and the linked checklist IDs. Include guardian collusion, unauthorized
   calls, double claims, balance/rounding failures, deadline boundaries,
   failed token calls, TTL/resource exhaustion and migration failures where
   applicable. Mark a category not applicable only with evidence and a reason.
3. **Assess inherent risk.** For each credible scenario, describe the actor,
   prerequisites, affected assets/operations and consequence. Apply the 1–5
   likelihood and impact criteria in the TOML without credit for controls
   specific to that scenario. Explain each rating; these are ordinal review
   judgments, not measured probabilities or financial loss forecasts.
4. **Verify controls.** Link source lines at the assessed commit, negative and
   boundary tests, fuzz reproductions and relevant build/dependency checks.
   A planned fix or a test that has not run is not a verified control. Record
   missing evidence and select the highest plausible rating until resolved.
5. **Assess current residual risk.** Re-score after accounting only for
   verified existing controls. Record which control changes which rating and
   why. Separately label any projected post-fix score as a *target*.
6. **Plan and verify mitigation.** Assign one accountable owner, actions,
   deadline, independent reviewer and measurable verification criteria.
   Reproduce the failure, verify the fix and applicable regressions, then
   re-score against the fixed commit. Record the release decision and
   maintainer sign-off. Reassess whenever scope, assumptions or controls change.

## Scoring and release decisions

For both inherent and current residual risk, calculate **likelihood × impact**.
Use the highest applicable impact, not an average across affected assets.

| Product | Severity |
| --- | --- |
| 1–4 | Low |
| 5–9 | Medium |
| 10–16 | High |
| 17–25 | Critical |

An impact of **5 always has at least High severity**, even if likelihood is 1.
A reviewer may escalate a severity with a written rationale, but must not
lower the calculated severity. An observation with no credible adverse
consequence is **Informational**, with no numeric score; do not use zero or
missing values to represent an assessed risk.

Keep inherent and residual scores separately. Planned mitigations cannot
lower the current residual score. Do not average several risks into a release
score: one unresolved High/Critical risk blocks deployment. Under
`SECURITY.md`, all Critical/High findings require independently verified
remediation before deployment, even if the proposed residual score is lower.
An accepted/wontfix status cannot bypass that rule.

For Medium/Low risks, either remediate or document explicit maintainer
acceptance with justification, a review date and reassessment triggers. A
missed deadline or expired acceptance requires re-triage, not automatic
closure. Controls based on monitoring need an alert owner and response plan;
monitoring alone does not prove prevention of irreversible loss.

## Assessment and mitigation record

Keep one report per reviewed release/commit, with this header and one record
per scenario. Use restricted storage for exploitable findings, following
[private disclosure](../SECURITY.md#reporting-a-vulnerability). Publish only
sanitized reports after coordinated disclosure. This template is not a live
finding and creates no accepted risks.

```text
Assessment ID / date / reviewer:
Commit / Wasm hashes / deployment network and addresses:
Scope / assets / trust boundaries / dependencies / exclusions:
Category coverage and checklist evidence (or justified N/A):

Risk ID: R-<unique id>
Finding ID: F-<id in audit.finding or confidential finding log>
Category IDs / linked checklist IDs:
Scenario: actor, prerequisites, affected operation/asset, consequence
Evidence / assumptions / open evidence gaps:
Inherent: likelihood [1..5], impact [1..5], product, severity, rationales
Verified existing controls and evidence:
Current residual: likelihood [1..5], impact [1..5], product, severity, rationales
Strategy: avoid | reduce | accept
Owner / actions / due date:
Verification criteria / regression tests / independent reviewer:
Target residual (optional, explicitly unverified):
Finding status: open | in-progress | resolved | accepted | wontfix
Remediation status: pending | in-progress | fixed | verified | closed
Fix commit / verification results / verified by and date:
Acceptance (if permitted): justification, maintainer sign-off, review date,
                         reassessment triggers
Final residual score / deployment decision / sign-off:
```

Use the existing `audit.finding` and `audit.remediation` records and link them
by finding ID; do not create a second conflicting lifecycle. `fixed` means a
change exists, `verified` means an independent reviewer has checked it, and
`closed` requires recorded verification and the residual-risk decision.
Attach the assessment to the final `audit.report` via its `methodology` and
`findings_summary` fields. Preserve earlier assessments when a score changes.

## Worked example (hypothetical, not a finding)

Suppose a test fixture models duplicate execution of a payout. A reviewer
assigns inherent likelihood 4 and impact 4: **16, High**. A proposed idempotency
fix may target likelihood 1, impact 4: **4, Low**, but the *current* risk remains
High until the control is verified.

The mitigation owner implements the guard and adds a test that attempts the
same claim twice and checks both rejection and unchanged recipient/contract
balances on the second attempt. An independent reviewer checks the fix commit,
the reproduction and relevant regression results. Only then may the record
use residual 1 × 4 = 4, Low, with an explanation of the reduced likelihood and
maintainer sign-off. The original High finding still needs verified remediation
before deployment. These numbers illustrate the method; they do not assess any
current SoroWill implementation.
