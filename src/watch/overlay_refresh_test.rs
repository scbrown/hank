//! Tests for the tenant-aware tier handler (FR-17 × FR-16). Child module of
//! `overlay_refresh`; size-exempt (`_test.rs`).

use super::*;
use crate::graph::Base;
use std::sync::Arc;

/// A committed leaf ← mid chain, plus the registry over it.
fn setup() -> (tempfile::TempDir, Arc<RwLock<TenantRegistry>>) {
    let dir = tempfile::tempdir().unwrap();
    let run = |args: &[&str]| {
        let out = std::process::Command::new("git")
            .arg("-C")
            .arg(dir.path())
            .args(args)
            .output()
            .unwrap();
        assert!(out.status.success(), "git {args:?} failed");
    };
    run(&["init", "-q", "-b", "main"]);
    run(&["config", "user.email", "t@t"]);
    run(&["config", "user.name", "t"]);
    std::fs::write(dir.path().join("leaf.rs"), "fn leaf() {}\n").unwrap();
    std::fs::write(dir.path().join("mid.rs"), "fn mid() { leaf(); }\n").unwrap();
    run(&["add", "-A"]);
    run(&["commit", "-qm", "base"]);
    let base = Base::build_at(dir.path(), "main").unwrap();
    (dir, Arc::new(RwLock::new(TenantRegistry::new(base))))
}

#[test]
fn an_edit_updates_the_overlay_and_freshness_transitions_recomputing_then_fresh() {
    let (dir, registry) = setup();
    let mut h = OverlayRefresh::new(Arc::clone(&registry), "dev", dir.path().to_path_buf(), 5);

    // Simulate an on-disk edit: leaf.rs grows a new function.
    std::fs::write(dir.path().join("leaf.rs"), "fn leaf() {}\nfn added() {}\n").unwrap();
    let edited = dir.path().join("leaf.rs");

    // Fast tier: overlay reflects the edit, and the file is RECOMPUTING (the
    // frontier has not been recomputed yet).
    h.tree_sitter(&[edited.clone()]);
    assert!(
        registry.read().unwrap().view("dev").has_symbol("added"),
        "overlay must reflect the on-disk edit after the fast tier"
    );
    assert_eq!(h.freshness_of("leaf.rs"), Some(Freshness::Recomputing));

    // Heavy tier: frontier recomputed → FRESH.
    h.heavy(&[edited]);
    assert_eq!(h.freshness_of("leaf.rs"), Some(Freshness::Fresh));
}

#[test]
fn the_edit_is_isolated_to_the_handlers_tenant() {
    let (dir, registry) = setup();
    let mut h = OverlayRefresh::new(Arc::clone(&registry), "dev", dir.path().to_path_buf(), 5);
    std::fs::write(dir.path().join("leaf.rs"), "fn leaf() {}\nfn added() {}\n").unwrap();
    h.tree_sitter(&[dir.path().join("leaf.rs")]);

    let reg = registry.read().unwrap();
    assert!(reg.view("dev").has_symbol("added"), "dev sees its edit");
    assert!(
        !reg.view("other").has_symbol("added"),
        "an unrelated tenant is unaffected (§6.3)"
    );
}

#[test]
fn a_removed_file_masks_it_via_the_empty_touch() {
    let (dir, registry) = setup();
    let mut h = OverlayRefresh::new(Arc::clone(&registry), "dev", dir.path().to_path_buf(), 5);
    // Remove mid.rs on disk, then dispatch the event.
    std::fs::remove_file(dir.path().join("mid.rs")).unwrap();
    h.tree_sitter(&[dir.path().join("mid.rs")]);
    assert!(
        !registry.read().unwrap().view("dev").has_symbol("mid"),
        "a removed file is masked for the tenant"
    );
}
