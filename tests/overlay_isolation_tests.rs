//! The §6.3 acceptance suite for hank #2 slice 2: tenant isolation is
//! ABSOLUTE, masking/revert/deletion behave, overlay cost is `O(touched)`,
//! and FR-15 interning shares parses across tenants. Everything goes through
//! the public API: `Base` → `TenantRegistry` → `TenantView` → the FR-12 walk.

use hank::graph::{reachable_over, Base, Dir, TenantRegistry};
use std::sync::Arc;

/// A committed 3-hop chain (leaf ← mid ← top) plus `pad` files so `O(touched)`
/// vs `O(repo)` is observable.
fn registry() -> (tempfile::TempDir, TenantRegistry) {
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
    std::fs::write(dir.path().join("top.rs"), "fn top() { mid(); }\n").unwrap();
    for i in 0..40 {
        std::fs::write(
            dir.path().join(format!("pad{i}.rs")),
            format!("fn pad{i}() {{}}\n"),
        )
        .unwrap();
    }
    run(&["add", "-A"]);
    run(&["commit", "-qm", "base"]);
    let base = Base::build_at(dir.path(), "main").unwrap();
    (dir, TenantRegistry::new(base))
}

fn caller_names(reg: &TenantRegistry, tenant: &str, of: &str) -> Vec<String> {
    let view = reg.view(tenant);
    let mut names: Vec<String> = reachable_over(&view, of, Dir::Callers, 1)
        .into_iter()
        .map(|r| r.name)
        .collect();
    names.sort();
    names.dedup();
    names
}

#[test]
fn tenant_a_overlay_is_NEVER_observable_by_tenant_b() {
    let (_dir, mut reg) = registry();
    reg.touch("a", "a_new.rs", "fn a_only() { leaf(); }\n");

    // A sees its own symbol and its new call edge into the shared base.
    assert!(reg.view("a").has_symbol("a_only"));
    assert_eq!(caller_names(&reg, "a", "leaf"), ["a_only", "mid"]);

    // B — and any tenant never seen — sees NOTHING of it (§6.3 absolute).
    assert!(!reg.view("b").has_symbol("a_only"));
    assert_eq!(caller_names(&reg, "b", "leaf"), ["mid"]);

    // And the shared base object itself is untouched.
    assert!(!reg.base().graph().has_symbol("a_only"));
}

#[test]
fn an_empty_overlay_views_identically_to_the_base() {
    let (_dir, reg) = registry();
    let view = reg.view("never-seen");
    let via_view = reachable_over(&view, "leaf", Dir::Callers, 5);
    let via_base = reg.base().graph().reachable("leaf", Dir::Callers, 5);
    let key = |r: &hank::graph::Reached| (r.name.clone(), r.file.clone(), r.distance);
    let mut a: Vec<_> = via_view.iter().map(key).collect();
    let mut b: Vec<_> = via_base.iter().map(key).collect();
    a.sort();
    b.sort();
    assert_eq!(a, b, "no overlay ⇒ the view IS the base");
}

#[test]
fn a_touched_file_is_masked_the_overlay_owns_its_truth() {
    let (_dir, mut reg) = registry();
    // A rewrites mid.rs: `mid` is gone, `mid2` takes over the call to leaf.
    reg.touch("a", "mid.rs", "fn mid2() { leaf(); }\n");

    // Through A: the base's `mid` is invisible, `mid2` answers.
    assert!(!reg.view("a").has_symbol("mid"), "masked, not merged");
    assert_eq!(caller_names(&reg, "a", "leaf"), ["mid2"]);

    // top's call to `mid` remaps by name — the overlay defines no `mid`, so
    // the edge is GONE for A (never resurrected from the base).
    let top_callees = reachable_over(&reg.view("a"), "top", Dir::Callees, 5);
    assert!(
        top_callees.is_empty(),
        "top called mid; mid.rs is masked and defines no mid: {top_callees:?}"
    );

    // B is oblivious on every count.
    assert!(reg.view("b").has_symbol("mid"));
    assert_eq!(caller_names(&reg, "b", "leaf"), ["mid"]);
}

#[test]
fn base_callers_reach_an_overlay_redefinition_transitively() {
    let (_dir, mut reg) = registry();
    // A rewrites leaf.rs, keeping the name: base callers must reach the NEW
    // definition, and the transitive impact chain must survive the remap.
    reg.touch(
        "a",
        "leaf.rs",
        "fn leaf() { /* rewritten */ }\nfn extra() {}\n",
    );

    let impact = reachable_over(&reg.view("a"), "leaf", Dir::Callers, 5);
    let names: Vec<&str> = impact.iter().map(|r| r.name.as_str()).collect();
    assert!(names.contains(&"mid"), "direct base caller: {names:?}");
    assert!(names.contains(&"top"), "transitive base caller: {names:?}");

    // And mid's callees point at the overlay's leaf.rs, not a ghost.
    let mid_callees = reachable_over(&reg.view("a"), "mid", Dir::Callees, 1);
    assert_eq!(mid_callees.len(), 1);
    assert_eq!(mid_callees[0].name, "leaf");
}

#[test]
fn an_overlay_caller_reaches_base_callees() {
    let (_dir, mut reg) = registry();
    reg.touch("a", "a_new.rs", "fn newfn() { mid(); }\n");
    let callees = reachable_over(&reg.view("a"), "newfn", Dir::Callees, 5);
    let names: Vec<&str> = callees.iter().map(|r| r.name.as_str()).collect();
    assert!(names.contains(&"mid"), "overlay → base edge: {names:?}");
    assert!(
        names.contains(&"leaf"),
        "and onward through the base: {names:?}"
    );
}

#[test]
fn revert_restores_the_base_truth() {
    let (_dir, mut reg) = registry();
    reg.touch("a", "mid.rs", "fn mid2() { leaf(); }\n");
    assert_eq!(caller_names(&reg, "a", "leaf"), ["mid2"]);

    reg.revert("a", "mid.rs");
    assert!(reg.view("a").has_symbol("mid"), "base resumed");
    assert_eq!(caller_names(&reg, "a", "leaf"), ["mid"]);
}

#[test]
fn deletion_is_the_empty_touch() {
    let (_dir, mut reg) = registry();
    reg.touch("a", "leaf.rs", "");
    assert!(!reg.view("a").has_symbol("leaf"), "deleted for A");
    assert!(
        reachable_over(&reg.view("a"), "leaf", Dir::Callers, 5).is_empty(),
        "no seeds, no impact"
    );
    assert!(reg.view("b").has_symbol("leaf"), "present for B");
}

#[test]
fn drop_tenant_reverts_everything_at_once() {
    let (_dir, mut reg) = registry();
    reg.touch("a", "leaf.rs", "");
    reg.touch("a", "a_new.rs", "fn a_only() {}\n");
    reg.drop_tenant("a");
    assert!(reg.view("a").has_symbol("leaf"));
    assert!(!reg.view("a").has_symbol("a_only"));
    assert!(reg.tenants().is_empty());
}

#[test]
fn overlay_cost_is_o_touched_not_o_repo() {
    let (_dir, mut reg) = registry();
    // The base holds 43 files / 43+ symbols; A touches ONE file with TWO
    // symbols. The overlay must hold exactly that — nothing repo-shaped.
    reg.touch("a", "mid.rs", "fn mid() { leaf(); }\nfn mid_b() {}\n");
    let overlay = reg.overlay("a").expect("a touched");
    assert_eq!(overlay.touched_count(), 1);
    assert_eq!(overlay.symbol_count(), 2, "touched symbols only");
    assert!(reg.base().file_count() >= 43, "the base is repo-sized");
}

#[test]
fn identical_content_across_tenants_shares_one_parse_fr15() {
    let (_dir, mut reg) = registry();
    let content = "fn shared_edit() { leaf(); }\n";
    reg.touch("a", "same.rs", content);
    reg.touch("b", "same.rs", content);
    let (pa, pb) = (
        reg.overlay("a").unwrap().parsed("same.rs").unwrap(),
        reg.overlay("b").unwrap().parsed("same.rs").unwrap(),
    );
    assert!(
        Arc::ptr_eq(pa, pb),
        "identical bytes must intern to ONE ParsedFile (FR-15)"
    );

    // Sharing the parse is NOT sharing the view: each tenant still resolves
    // through its own overlay.
    assert!(reg.view("a").has_symbol("shared_edit"));
    assert!(!reg.view("c").has_symbol("shared_edit"));
}

#[test]
fn status_reports_base_commit_and_active_overlays() {
    let (_dir, mut reg) = registry();
    reg.touch("a", "mid.rs", "fn mid2() { leaf(); }\n");
    reg.touch("b", "x.rs", "fn x() {}\nfn y() {}\n");

    let status = reg.status();
    assert_eq!(status.base_commit.len(), 40, "resolved commit id");
    let tenants: Vec<(&str, usize, usize)> = status
        .active_overlays
        .iter()
        .map(|o| (o.tenant.as_str(), o.touched_files, o.symbols))
        .collect();
    assert_eq!(
        tenants,
        [("a", 1, 1), ("b", 1, 2)],
        "sorted, with O(touched) sizes"
    );
}
