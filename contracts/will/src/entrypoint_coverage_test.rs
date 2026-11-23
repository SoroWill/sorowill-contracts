#![cfg(test)]

//! Automated CI check for Issue #287: ensures every `#[contractimpl]` public
//! method on `WillContract` is covered by at least one reference in the test suite.
//!
//! Issue #807 (contract function visibility) extends this file with a check that
//! the visibility contract holds: every `#[contractimpl]` entry point is declared
//! `pub fn`, and internal helpers (declared after the private-helpers marker) are
//! not exposed as `pub fn`.

use std::fs;
use std::path::Path;

/// Marker that separates the `#[contractimpl]` block from private helpers.
const PRIVATE_HELPERS_MARKER: &str = "\n// ── Private helpers";
const PRIVATE_HELPERS_MARKER_ALT: &str = "\n// --- Private";

/// Returns the slice of `lib.rs` that contains the `#[contractimpl]` block.
fn contractimpl_block(lib_content: &str) -> &str {
    let impl_start = lib_content
        .find("#[contractimpl]")
        .expect("Could not find #[contractimpl] in lib.rs");

    let impl_block = &lib_content[impl_start..];
    let block_end = impl_block
        .find(PRIVATE_HELPERS_MARKER)
        .or_else(|| impl_block.find(PRIVATE_HELPERS_MARKER_ALT))
        .unwrap_or(impl_block.len());

    &impl_block[..block_end]
}

#[test]
fn test_all_contractimpl_entry_points_are_covered_by_tests() {
    let lib_content = include_str!("lib.rs");
    let impl_body = contractimpl_block(lib_content);

    // Find all `pub fn <name>` declarations in the contractimpl block
    let mut entry_points = Vec::new();
    for line in impl_body.lines() {
        let trimmed = line.trim();
        if let Some(stripped) = trimmed.strip_prefix("pub fn ") {
            if let Some(fn_name) = stripped.split('(').next() {
                entry_points.push(fn_name.trim().to_string());
            }
        }
    }

    assert!(
        !entry_points.is_empty(),
        "Expected to find at least 1 #[contractimpl] entry point"
    );

    // Read all test files in contracts/will/src and contracts/will/tests
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap_or_else(|_| ".".to_string());
    let src_dir = Path::new(&manifest_dir).join("src");
    let tests_dir = Path::new(&manifest_dir).join("tests");

    let mut test_contents = Vec::new();

    if src_dir.exists() {
        for entry in fs::read_dir(&src_dir)
            .expect("Failed to read src dir")
            .flatten()
        {
            let path = entry.path();
            if path.is_file()
                && path.extension().is_some_and(|ext| ext == "rs")
                && path.file_name().is_some_and(|n| n != "lib.rs")
            {
                if let Ok(content) = fs::read_to_string(&path) {
                    test_contents.push(content);
                }
            }
        }
    }

    if tests_dir.exists() {
        for entry in fs::read_dir(&tests_dir)
            .expect("Failed to read tests dir")
            .flatten()
        {
            let path = entry.path();
            if path.is_file() && path.extension().is_some_and(|ext| ext == "rs") {
                if let Ok(content) = fs::read_to_string(&path) {
                    test_contents.push(content);
                }
            }
        }
    }

    let mut uncovered = Vec::new();
    for entry_point in &entry_points {
        let is_covered = test_contents
            .iter()
            .any(|content| content.contains(entry_point));
        if !is_covered {
            uncovered.push(entry_point.clone());
        }
    }

    assert!(
        uncovered.is_empty(),
        "The following #[contractimpl] entry points have zero references in the test suite: {:?}",
        uncovered
    );
}

/// Issue #807: every function declared inside the `#[contractimpl]` block must be
/// `pub fn` so it is exposed as a contract entry point. A non-`pub` function in
/// that block would silently be unreachable from the contract interface.
#[test]
fn test_contractimpl_functions_are_public() {
    let lib_content = include_str!("lib.rs");
    let impl_body = contractimpl_block(lib_content);

    let mut non_public = Vec::new();
    for line in impl_body.lines() {
        let trimmed = line.trim();
        // Skip doc/attribute lines and the impl header itself.
        if trimmed.starts_with("//") || trimmed.starts_with("#[") || trimmed.starts_with("impl ") {
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("fn ") {
            if let Some(fn_name) = rest.split('(').next() {
                non_public.push(fn_name.trim().to_string());
            }
        }
    }

    assert!(
        non_public.is_empty(),
        "Functions inside #[contractimpl] must be `pub fn` (contract entry points): {:?}",
        non_public
    );
}

/// Issue #807: internal helpers (declared after the private-helpers marker) must
/// not be exposed as `pub fn`, keeping the contract's public surface minimal.
#[test]
fn test_private_helpers_are_not_public() {
    let lib_content = include_str!("lib.rs");

    let helpers_start = lib_content
        .find(PRIVATE_HELPERS_MARKER)
        .or_else(|| lib_content.find(PRIVATE_HELPERS_MARKER_ALT));

    let Some(helpers_start) = helpers_start else {
        // No private-helpers section present; nothing to verify.
        return;
    };

    let helpers_body = &lib_content[helpers_start..];

    let mut leaked = Vec::new();
    for line in helpers_body.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("pub fn ") {
            if let Some(fn_name) = rest.split('(').next() {
                leaked.push(fn_name.trim().to_string());
            }
        }
    }

    assert!(
        leaked.is_empty(),
        "Internal helpers must not be `pub fn` (use `pub(crate)` or private): {:?}",
        leaked
    );
}
