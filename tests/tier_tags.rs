//! FR-3 on the CLI surface: `hank impact` / `callers` / `dataflow --json` carry
//! their provenance `tier` (aegis-8yrn). These served an unlabelled tree-sitter
//! approximation before the fix — no `tier` field — which is what FR-3 exists to
//! forbid, worst of all on `impact` (the blast-radius / trust-boundary surface).
//!
//! Kept out of `tests/cli.rs`, which already sits at the file-size limit.

use assert_cmd::Command;
use predicates::prelude::*;

/// A two-function project (`a` calls `b`) so the call graph is non-empty.
fn project() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("x.rs"), "fn a() { b(); }\nfn b() {}\n").unwrap();
    dir
}

fn run(dir: &tempfile::TempDir, args: &[&str]) -> serde_json::Value {
    let out = Command::cargo_bin("hank")
        .unwrap()
        .args(args)
        .arg(dir.path())
        .arg("--json")
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    serde_json::from_slice(&out).expect("stdout is JSON")
}

#[test]
fn impact_json_carries_tier() {
    let dir = project();
    let v = run(&dir, &["impact", "b"]);
    // The bug: this was served with no tier at all. It is the trust-boundary
    // surface, so an unlabelled approximation here is exactly what FR-3 forbids.
    assert_eq!(v["tier"], "treesitter", "impact --json lost its tier: {v}");
}

#[test]
fn impact_json_on_a_missing_symbol_still_carries_tier() {
    // The empty-case hole: a not-found answer has no items to tag, so the tier has
    // to ride on the response itself or the result arrives unlabelled.
    let dir = project();
    let v = run(&dir, &["impact", "does_not_exist"]);
    assert_eq!(v["found"], false);
    assert_eq!(
        v["tier"], "treesitter",
        "not-found impact lost its tier: {v}"
    );
}

#[test]
fn callers_json_carries_tier() {
    let dir = project();
    let v = run(&dir, &["callers", "b"]);
    assert_eq!(v["tier"], "treesitter", "callers --json lost its tier: {v}");
}

#[test]
fn dataflow_json_carries_tier() {
    let dir = project();
    let v = run(&dir, &["dataflow", "a"]);
    assert_eq!(
        v["tier"], "treesitter",
        "dataflow --json lost its tier: {v}"
    );
}

#[test]
fn refs_not_found_message_is_unchanged() {
    // Guard the shared not_found() edit: adding a tier field must not disturb the
    // human-readable path other commands rely on.
    let dir = project();
    Command::cargo_bin("hank")
        .unwrap()
        .args(["callers", "does_not_exist"])
        .arg(dir.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("not found in the call graph"));
}

#[test]
fn status_json_advertises_only_implemented_tiers() {
    // aegis-qe5z: `hank status` used to push "lsp"/"cpg" onto the advertised tier
    // list under empty Cargo features that gated no code, so `--features lsp` made
    // the tool claim a precision tier it did not have. status now advertises exactly
    // the tiers with a real extractor — treesitter alone today.
    let out = Command::cargo_bin("hank")
        .unwrap()
        .args(["status", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let v: serde_json::Value = serde_json::from_slice(&out).expect("stdout is JSON");
    assert_eq!(
        v["tiers"],
        serde_json::json!(["treesitter"]),
        "status advertised an unimplemented tier: {v}"
    );
}
