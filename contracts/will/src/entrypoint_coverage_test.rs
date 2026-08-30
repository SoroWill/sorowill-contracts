#![cfg(test)]

//! Automated CI check for Issue #287: ensures every `#[contractimpl]` public
//! method on `WillContract` is covered by at least one reference in the test suite.

use std::fs;
use std::path::Path;

#[test]
fn test_all_contractimpl_entry_points_are_covered_by_tests() {
    let lib_content = include_str!("lib.rs");

    // Extract the impl WillContract block
    let impl_start = lib_content
        .find("#[contractimpl]")
        .expect("Could not find #[contractimpl] in lib.rs");

    let impl_block = &lib_content[impl_start..];
    let block_end = impl_block
        .find("\n// ── Private helpers")
        .or_else(|| impl_block.find("\n// --- Private"))
        .unwrap_or(impl_block.len());

    let impl_body = &impl_block[..block_end];

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
        for entry in fs::read_dir(&src_dir).expect("Failed to read src dir").flatten() {
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
        for entry in fs::read_dir(&tests_dir).expect("Failed to read tests dir").flatten() {
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
        let is_covered = test_contents.iter().any(|content| content.contains(entry_point));
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
