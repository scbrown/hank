//! hank's own tree must not carry internal identifiers (aegis-wvuhj).
//!
//! WHY THIS FILE EXISTS, AND WHY ITS ABSENCE WAS THE ACTUAL BUG. hank IS the
//! enforcement point for "internal identifiers must not enter public-remote
//! repos" — the rule is `crate::textrules`, the guard is the pre-edit hook, and
//! `src/hook/pre_edit_test.rs` asserts the guard fires on a `.lan` hostname. And
//! `scbrown/hank` is PUBLIC and its default branch carried **60 lines of them
//! across 15 files** (measured 2026-08-04; gennaro measured 58/14 two days
//! earlier, so it was still GROWING while the bead sat open).
//!
//! The edit-time guard was never the gap. It only ever sees text an edit
//! INTRODUCES — by deliberate design, so a dirty file does not brick every edit
//! to it. Nothing ever scanned the tree that already existed, so pre-existing
//! debt was permanently invisible to the mechanism built to prevent it. bobbin,
//! same fleet and same rule, has had `tests/no_internal_identifiers.rs` all
//! along. This is that ratchet, for the repo that enforces the rule.
//!
//! THE SYNTHETIC-NAME RULE, which is the whole remedy for test fixtures: a test
//! that needs a forbidden-looking token must invent one. `db.lan` proves the
//! `.lan` rule fires exactly as well as a real hostname does, and leaks nothing.
//! "The guard needs real data to be tested" is false and bobbin already proved
//! it. Wherever a token did NOT need to match a pattern at all — doc comments,
//! ssh/scp examples, book pages — the scrub used names outside the patterns
//! entirely (`web.example`, `$QUIPU_URL`), so they need no exemption here.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;

use regex::Regex;

/// The identifier classes, mirroring the `aegis:InternalIdentifierPattern`
/// catalogue the pre-edit hook enforces. Self-contained regexes rather than a
/// projection from quipu ON PURPOSE: this test has to run in CI with no graph
/// reachable, and a ratchet that silently skips when its data source is absent
/// is the failure it exists to prevent.
fn patterns() -> Vec<(&'static str, Regex)> {
    vec![
        (
            "internal hostname",
            Regex::new(r"[a-z0-9_.-]+\.lan\b").unwrap(),
        ),
        (
            "internal service host",
            Regex::new(r"[a-z0-9_-]+\.svc\b").unwrap(),
        ),
        (
            "private address",
            Regex::new(r"\b192\.168\.\d{1,3}\.\d{1,3}\b").unwrap(),
        ),
        (
            "operator home path",
            Regex::new(r"/home/(?:braino|stiwi)\b").unwrap(),
        ),
    ]
}

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn tracked_files() -> Vec<PathBuf> {
    let out = Command::new("git")
        .arg("-C")
        .arg(root())
        .arg("ls-files")
        .output()
        .expect("git ls-files");
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(|l| root().join(l))
        .collect()
}

/// Files allowed to contain a matching token, and WHY. Kept to the two that
/// genuinely assert the guard FIRES — a file that must name a forbidden shape to
/// forbid it. This is the same carve-out `TextRule::exempt_path_regex` exists
/// for. Every name inside them is synthetic; the exemption buys the ability to
/// test detection, never the right to name a real host.
fn exempt(rel: &Path) -> bool {
    matches!(
        rel.to_string_lossy().as_ref(),
        "tests/no_internal_identifiers.rs"      // this file names the patterns
            | "src/textrules.rs"                // asserts the .lan rule matches
            | "src/hook/pre_edit_test.rs" // asserts the verdict text
    )
}

/// The ontology namespace is NOT a leak to scrub and must not be quietly
/// swallowed either (aegis-wvuhj category 1).
///
/// `http://aegis.gastown.local/ontology/` is a live DATA CONTRACT, not an
/// example: `src/export.rs` mints every IRI under it and ~102,945 subjects are
/// already stored against it in quipu. Repointing it is a data migration plus a
/// cross-repo decision with bobbin (bobbin#58 is blocked on the same one), and
/// `shapes/code-edges.ttl` states outright that changing the prefix breaks
/// validation.
///
/// So it is ALLOWED — and COUNTED, by the test below, so that "allowed" cannot
/// decay into "unnoticed" the way the rest of this debt did. The day the
/// namespace decision lands, that test fails and points here.
const ONTOLOGY_NS: &str = "aegis.gastown.local";

#[test]
fn no_internal_identifiers_in_any_tracked_file() {
    let pats = patterns();
    let mut offenders: BTreeSet<String> = BTreeSet::new();

    for path in tracked_files() {
        let rel = path.strip_prefix(root()).unwrap_or(&path).to_path_buf();
        if exempt(&rel) {
            continue;
        }
        let Ok(bytes) = std::fs::read(&path) else {
            continue; // unreadable or deleted-but-tracked: not this test's job
        };
        let text = String::from_utf8_lossy(&bytes);
        for (label, rx) in &pats {
            for caps in rx.captures_iter(&text) {
                let hit = caps.get(0).unwrap().as_str();
                // The namespace is a data contract, tracked separately below.
                if hit.contains(ONTOLOGY_NS) {
                    continue;
                }
                offenders.insert(format!("{}: {label} {hit:?}", rel.display()));
            }
        }
    }

    assert!(
        offenders.is_empty(),
        "internal identifier(s) in a PUBLIC repo (scbrown/hank):\n  {}\n\n\
         Use a synthetic name. If the token does NOT need to match a rule \
         pattern, put it outside them entirely (`web.example`, `$QUIPU_URL`). \
         If a test must prove the guard FIRES, invent one that matches \
         (`db.lan`) and add the file to `exempt()` with a reason.",
        offenders.into_iter().collect::<Vec<_>>().join("\n  ")
    );
}

#[test]
fn the_ratchet_catches_each_class() {
    // POSITIVE CONTROL, one per class. A guard never seen catching anything is a
    // function returning an empty set, and it looks exactly like a clean repo —
    // which is precisely how 60 lines accumulated in a repo that enforces this
    // rule on everyone else.
    let pats = patterns();
    for (expect, sample) in [
        ("internal hostname", "connect to db.lan now"),
        ("internal service host", "http://thing.svc/knot"),
        ("private address", "addr 192.168.0.1"),
        ("operator home path", "/home/braino/src/x"),
    ] {
        let caught = pats
            .iter()
            .any(|(label, rx)| *label == expect && rx.is_match(sample));
        assert!(caught, "the ratchet missed a planted {expect}: {sample:?}");
    }
}

#[test]
fn the_ontology_namespace_allowance_is_still_needed_and_still_bounded() {
    // The allowance above is deliberate, and this is what stops it rotting into
    // a silent permanent exception. It pins the namespace to the files that are
    // genuinely part of the data contract. A NEW file reaching for the real
    // namespace fails here and has to justify itself; the day bobbin#58's
    // repointing decision lands, this fails too and leads straight to the
    // allowance to delete.
    let mut carriers: BTreeSet<String> = BTreeSet::new();
    for path in tracked_files() {
        let rel = path.strip_prefix(root()).unwrap_or(&path).to_path_buf();
        let Ok(bytes) = std::fs::read(&path) else {
            continue;
        };
        if String::from_utf8_lossy(&bytes).contains(ONTOLOGY_NS) {
            carriers.insert(rel.display().to_string());
        }
    }

    let expected: BTreeSet<String> = [
        "docs/book/src/concepts/promotion.md",
        "docs/hank-spec.md",
        "scripts/delegate-boundary-guard.py",
        "shapes/code-edges.ttl",
        "shapes/fixtures/conforming.ttl",
        "shapes/fixtures/violating.ttl",
        "src/export.rs",
        "src/project.rs",
        "src/project_queries.rs",
        "src/promote_test.rs",
        "src/verdict.rs",
        "tests/no_internal_identifiers.rs",
    ]
    .iter()
    .map(std::string::ToString::to_string)
    .collect();

    assert_eq!(
        carriers, expected,
        "the ontology-namespace footprint moved. This is NOT a scrub target — it \
         is a live data contract (see ONTOLOGY_NS above and bobbin#58). If a new \
         file legitimately needs it, add it here. If the repointing decision has \
         landed, delete the allowance and this test together."
    );
}
