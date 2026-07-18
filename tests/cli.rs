//! Integration tests driving the `hank` binary.

use assert_cmd::Command;
use predicates::prelude::*;

/// Write a throwaway Rust file into a fresh temp dir and return the dir.
fn project_with(file: &str, contents: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join(file), contents).unwrap();
    dir
}

#[test]
fn status_json_reports_base_ref() {
    Command::cargo_bin("hank")
        .unwrap()
        .args(["status", "--json"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"base_ref\""));
}

#[test]
fn analyze_counts_symbols() {
    let dir = project_with("a.rs", "fn foo() {}\nstruct Bar;\n");
    Command::cargo_bin("hank")
        .unwrap()
        .args(["analyze", dir.path().to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("2 symbol"));
}

#[test]
fn refs_finds_definition() {
    let dir = project_with("a.rs", "fn target() {}\n");
    Command::cargo_bin("hank")
        .unwrap()
        .args(["refs", "target", dir.path().to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("target"));
}

#[test]
fn refs_json_is_empty_array_when_absent() {
    let dir = project_with("a.rs", "fn other() {}\n");
    Command::cargo_bin("hank")
        .unwrap()
        .args(["refs", "missing", dir.path().to_str().unwrap(), "--json"])
        .assert()
        .success()
        .stdout(predicate::str::contains("[]"));
}
