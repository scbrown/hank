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

#[test]
fn callers_lists_direct_callers() {
    let dir = project_with("a.rs", "fn leaf() {}\nfn mid() { leaf(); }\n");
    Command::cargo_bin("hank")
        .unwrap()
        .args(["callers", "leaf", dir.path().to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("mid"));
}

#[test]
fn impact_reconciles_with_cochange() {
    let dir = project_with(
        "a.rs",
        "fn leaf() {}\nfn mid() { leaf(); }\nfn top() { mid(); }\n",
    );
    // Co-change set: a.rs is corroborated (also structural); other.rs is not.
    std::fs::write(dir.path().join("cochange.json"), "[\"a.rs\", \"other.rs\"]").unwrap();

    Command::cargo_bin("hank")
        .unwrap()
        .args([
            "impact",
            "leaf",
            dir.path().to_str().unwrap(),
            "--cochange",
            dir.path().join("cochange.json").to_str().unwrap(),
            "--json",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"reconciliation\""))
        .stdout(predicate::str::contains("\"corroborated\""))
        .stdout(predicate::str::contains("other.rs"));
}

#[test]
fn dataflow_traces_dependence() {
    let dir = project_with(
        "a.rs",
        "fn f(a: i32) -> i32 { let b = a + 1; let c = b * 2; c }\n",
    );
    Command::cargo_bin("hank")
        .unwrap()
        .args([
            "dataflow",
            "f",
            dir.path().to_str().unwrap(),
            "--var",
            "c",
            "--json",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"b\""))
        .stdout(predicate::str::contains("\"a\""));
}

#[test]
fn impact_reports_transitive_callers() {
    let dir = project_with(
        "a.rs",
        "fn leaf() {}\nfn mid() { leaf(); }\nfn top() { mid(); }\n",
    );
    Command::cargo_bin("hank")
        .unwrap()
        .args([
            "impact",
            "leaf",
            dir.path().to_str().unwrap(),
            "--json",
            "--hops",
            "5",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"top\""))
        .stdout(predicate::str::contains("\"count\": 2"));
}
