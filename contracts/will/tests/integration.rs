//! Integration test that exercises the *compiled wasm artifact* through the
//! real `stellar` CLI against a local Soroban network, as opposed to the
//! in-process `soroban_sdk::Env::default()` unit tests in `src/test.rs`.
//!
//! This is `#[ignore]`-gated because it requires `stellar`, `docker`, and
//! network access to spin up a local standalone network, none of which are
//! available (or desirable) in the fast unit-test CI job. It is run
//! separately via `scripts/integration_test.sh` and a dedicated CI job.
//! See CONTRIBUTING.md for how to run it locally.
//!
//! # Network and blockchain failure handling (#727)
//!
//! The end-to-end run must tolerate, and the contract must handle, these
//! failure classes:
//!
//! - **RPC failures**: a submission that fails at simulation or send time
//!   never reaches the ledger, so no state changes. The client should re-read
//!   the will before retrying. Modelled in-process by
//!   `src/network_failure_test.rs` (`failed_*` tests).
//! - **Transaction timeouts**: a transaction whose validity window closes
//!   before inclusion is dropped. Time-gated entrypoints (`check_in`,
//!   `emergency_checkin`, `release_inheritance`) re-check the ledger time, so
//!   a late submission is refused rather than applied. Modelled by the
//!   `*_after_deadline*` / `*_after_grace_expiry` tests.
//! - **Resubmission**: a retry of a transaction that already landed is
//!   refused (`trigger_will` on a triggered will, `release_inheritance` on a
//!   settled will), so retries cannot double-apply.
//! - **Reorgs**: deadlines are stored on the will when it is triggered, so a
//!   ledger rewound to an earlier timestamp cannot unlock an early release.
//!   Modelled by `rewound_clock_does_not_unlock_early_release`.
//!
//! Clients should treat any error from a state-changing call as "not applied"
//! unless the ledger confirms otherwise, and re-read `get_will` before
//! resubmitting.

use std::path::PathBuf;
use std::process::Command;

#[test]
#[ignore = "requires stellar CLI + docker; run via scripts/integration_test.sh or the integration CI job"]
fn lifecycle_through_real_wasm_artifact() {
    let script = repo_root().join("scripts/integration_test.sh");
    assert!(
        script.exists(),
        "expected integration script at {}",
        script.display()
    );

    let status = Command::new("bash")
        .arg(&script)
        .current_dir(repo_root())
        .status()
        .expect("failed to spawn scripts/integration_test.sh");

    assert!(status.success(), "integration_test.sh exited with {status}");
}

fn repo_root() -> PathBuf {
    // contracts/will/tests/integration.rs -> repo root is three levels up.
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .expect("contracts/will should be nested two levels under the repo root")
        .to_path_buf()
}
