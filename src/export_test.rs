//! Tests for `export` — module/symbol/call/import/doc Turtle emission and
//! the multi-language (aegis-81t2) guarantees. Child module of `export`
//! (`super::*` reaches its private helpers); size-exempt (`_test.rs`).

use super::*;

#[test]
fn emits_modules_symbols_and_calls() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.rs"), "fn leaf() {}\n").unwrap();
    std::fs::write(dir.path().join("b.rs"), "fn mid() { leaf(); }\n").unwrap();

    let ttl = to_turtle(dir.path(), "demo").unwrap();
    assert!(ttl.contains("a bobbin:CodeModule"));
    assert!(ttl.contains("a bobbin:CodeSymbol"));
    assert!(ttl.contains("bobbin:name \"leaf\""));
    assert!(ttl.contains("bobbin:symbolKind \"function\""));
    assert!(ttl.contains("bobbin:definedIn"));
    assert!(ttl.contains("bobbin:calls"));
    assert!(ttl.contains("code/demo/"));
}

#[test]
fn symbol_iris_are_module_scoped() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.rs"), "fn only() {}\n").unwrap();
    let ttl = to_turtle(dir.path(), "demo").unwrap();
    assert!(ttl.contains("a.rs::only"));
}

/// The aegis-1q14 acceptance at the export layer: two same-named symbols in
/// one file emit DISTINCT IRIs (before the scope chain they collapsed into
/// one node in the BTreeSet — invisible downstream by construction).
#[test]
fn same_named_symbols_in_one_file_get_distinct_iris() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("m.rs"),
        "mod run;\nstruct Cli;\nimpl Cli { pub fn run(&self) {} }\n",
    )
    .unwrap();
    let ttl = to_turtle(dir.path(), "demo").unwrap();
    assert!(ttl.contains("m.rs::run"), "top-level mod keeps plain IRI");
    assert!(
        ttl.contains("m.rs::Cli::run"),
        "impl method is scope-qualified:\n{ttl}"
    );
}

/// Impl types can carry generics — IRI-hostile characters are encoded, and
/// injectively (`%` first), so distinct raw scopes stay distinct IRIs.
#[test]
fn scope_segments_are_iri_encoded_injectively() {
    assert_eq!(iri_segment("Foo<T>"), "Foo%3CT%3E");
    assert_eq!(iri_segment("A@Debug"), "A@Debug");
    // Injectivity guard: a raw segment containing a literal "%3C" must not
    // encode to the same text as one containing "<".
    assert_ne!(iri_segment("x%3C"), iri_segment("x<"));
    let iri = symbol_iri("m", &["Foo<T>".to_string()], "new");
    assert_eq!(iri, "m::Foo%3CT%3E::new");
}

#[test]
fn emits_import_edges_between_modules() {
    let dir = tempfile::tempdir().unwrap();
    // `consumer` imports from the `helper` module by `use`.
    std::fs::write(dir.path().join("helper.rs"), "pub fn thing() {}\n").unwrap();
    std::fs::write(
        dir.path().join("consumer.rs"),
        "use crate::helper::thing;\nfn run() { thing(); }\n",
    )
    .unwrap();

    let ttl = to_turtle(dir.path(), "demo").unwrap();
    assert!(
        ttl.contains("bobbin:imports"),
        "expected an imports edge, got:\n{ttl}"
    );
    // Edge points consumer → helper (the module IRI ends in the file path).
    let consumer = "consumer.rs";
    let helper = "helper.rs";
    let edge_line = ttl
        .lines()
        .find(|l| l.contains("bobbin:imports"))
        .unwrap_or_default();
    assert!(edge_line.contains(consumer), "from should be consumer");
    assert!(edge_line.contains(helper), "to should be helper");
}

#[test]
fn mod_rs_resolves_by_directory_name() {
    assert_eq!(module_stem("mcp/mod.rs"), "mcp");
    assert_eq!(module_stem("graph.rs"), "graph");
    assert_eq!(module_stem("src/graph.rs"), "graph");
}

#[test]
fn emits_doc_references_to_known_symbols() {
    let dir = tempfile::tempdir().unwrap();
    // A code file defining two symbols the doc will reference.
    std::fs::write(
        dir.path().join("graph.rs"),
        "pub fn reachable() {}\npub struct Frontier;\n",
    )
    .unwrap();
    // A doc that references them by backtick, qualifier, and fenced code.
    std::fs::write(
        dir.path().join("guide.md"),
        "# Traversal\n\nThe `reachable` fn walks the graph; \
             see `graph::reachable` too.\n\n\
             ## Types\n\n```rust\nlet f: Frontier;\n```\n\n\
             ## Unrelated\n\nMentions `nonexistent_symbol` only.\n",
    )
    .unwrap();

    let ttl = to_turtle(dir.path(), "demo").unwrap();

    // Document + Section entities materialize for referenced sections.
    assert!(ttl.contains("a bobbin:Document"), "got:\n{ttl}");
    assert!(ttl.contains("bobbin:filePath \"guide.md\""));
    assert!(ttl.contains("a bobbin:Section"));
    assert!(ttl.contains("bobbin:heading \"Traversal\""));
    assert!(ttl.contains("bobbin:headingDepth 1"));

    // The references edge points a Section IRI at a real CodeSymbol IRI.
    assert!(
        ttl.contains("bobbin:references"),
        "no references edge:\n{ttl}"
    );
    let reachable_sym = "code/demo/graph.rs::reachable";
    let frontier_sym = "code/demo/graph.rs::Frontier";
    assert!(
        ttl.lines()
            .any(|l| l.contains("bobbin:references") && l.contains(reachable_sym)),
        "expected a references edge to reachable:\n{ttl}"
    );
    assert!(
        ttl.lines()
            .any(|l| l.contains("bobbin:references") && l.contains(frontier_sym)),
        "expected a references edge to Frontier (from fenced code):\n{ttl}"
    );

    // The Section IRI on the edge is anchored to the document.
    assert!(ttl.contains("doc/demo/guide.md#traversal"));

    // A mention that resolves to nothing is dropped — no fabricated symbol,
    // and its heading-only section is not materialized.
    assert!(
        !ttl.contains("nonexistent_symbol"),
        "fabricated symbol:\n{ttl}"
    );
    assert!(
        !ttl.contains("#unrelated"),
        "empty section materialized:\n{ttl}"
    );
}

#[test]
fn qualifier_narrows_ambiguous_symbol_to_its_module() {
    let dir = tempfile::tempdir().unwrap();
    // Two modules each define a `run` symbol — an ambiguous bare name.
    std::fs::write(dir.path().join("graph.rs"), "pub fn run() {}\n").unwrap();
    std::fs::write(dir.path().join("serve.rs"), "pub fn run() {}\n").unwrap();
    // The doc qualifies the mention: `graph::run` should hit only graph's.
    std::fs::write(
        dir.path().join("doc.md"),
        "# H\n\nCall `graph::run` here.\n",
    )
    .unwrap();

    let ttl = to_turtle(dir.path(), "demo").unwrap();
    let edges: Vec<&str> = ttl
        .lines()
        .filter(|l| l.contains("bobbin:references"))
        .collect();
    assert_eq!(
        edges.len(),
        1,
        "qualifier should narrow to one edge:\n{ttl}"
    );
    assert!(edges[0].contains("code/demo/graph.rs::run"));
    assert!(!edges[0].contains("serve.rs"));
}

#[cfg(feature = "langs-extra")]
#[test]
fn a_python_repo_exports_real_structure_with_its_own_language() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.py"), "def leaf():\n    pass\n").unwrap();
    std::fs::write(dir.path().join("b.py"), "def mid():\n    leaf()\n").unwrap();
    let ttl = to_turtle(dir.path(), "pyrepo").unwrap();
    assert!(ttl.contains("bobbin:language \"python\""), "{ttl}");
    assert!(ttl.contains("a.py::leaf"), "{ttl}");
    assert!(
        ttl.contains("bobbin:calls"),
        "a py call edge must resolve: {ttl}"
    );
}

#[cfg(feature = "langs-extra")]
#[test]
fn cross_language_name_collisions_mint_no_edges() {
    // `main` exists in both files; a global name map would draw
    // rust-main -> py-main (or worse, both directions). Language-scoped
    // resolution draws neither.
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("x.rs"),
        "fn helper() {}\nfn main() { helper(); }\n",
    )
    .unwrap();
    std::fs::write(dir.path().join("y.py"), "def helper():\n    pass\n").unwrap();
    let ttl = to_turtle(dir.path(), "mixed").unwrap();
    // rust's call edge resolves within rust...
    assert!(ttl.contains("x.rs::main"), "{ttl}");
    // ...and no edge crosses into the python helper.
    let py_helper_called = ttl
        .lines()
        .any(|l| l.contains("bobbin:calls") && l.contains("y.py"));
    assert!(!py_helper_called, "a cross-language edge is a lie: {ttl}");
}

#[cfg(feature = "langs-extra")]
#[test]
fn python_init_takes_its_directory_name_like_mod_rs() {
    assert_eq!(module_stem("pkg/__init__.py"), "pkg");
    assert_eq!(module_stem("pkg/index.ts"), "pkg");
    assert_eq!(module_stem("pkg/mod.rs"), "pkg");
    assert_eq!(module_stem("pkg/other.py"), "other");
}

#[test]
fn a_rust_only_build_still_exports_rust() {
    // The positive control for the default feature set: the walk change
    // must not have cost the original language anything.
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.rs"), "fn only() {}\n").unwrap();
    let ttl = to_turtle(dir.path(), "r").unwrap();
    assert!(ttl.contains("a.rs::only"), "{ttl}");
    assert!(ttl.contains("bobbin:language \"rust\""), "{ttl}");
}

/// FR-22: promotion reads the COMMITTED tree, never the working tree — an
/// uncommitted edit (the shape an in-flight overlay/unsaved buffer takes on
/// disk) must not reach the promoted projection, and a not-yet-committed new
/// file must not either.
#[test]
fn to_turtle_at_promotes_the_committed_tree_not_working_churn() {
    let dir = tempfile::tempdir().unwrap();
    let run = |args: &[&str]| {
        assert!(std::process::Command::new("git")
            .arg("-C")
            .arg(dir.path())
            .args(args)
            .output()
            .unwrap()
            .status
            .success());
    };
    run(&["init", "-q", "-b", "main"]);
    run(&["config", "user.email", "t@t"]);
    run(&["config", "user.name", "t"]);
    std::fs::write(dir.path().join("a.rs"), "fn committed() {}\n").unwrap();
    run(&["add", "-A"]);
    run(&["commit", "-qm", "base"]);

    // Working-tree churn AFTER the commit: rewrite a.rs and add an untracked file.
    std::fs::write(
        dir.path().join("a.rs"),
        "fn committed() {}\nfn uncommitted_edit() {}\n",
    )
    .unwrap();
    std::fs::write(dir.path().join("b.rs"), "fn untracked_file() {}\n").unwrap();

    // The committed-tree projection (what promote uses) sees ONLY the commit.
    let at = to_turtle_at(dir.path(), "demo", "HEAD").unwrap();
    assert!(
        at.contains("a.rs::committed"),
        "committed symbol present: {at}"
    );
    assert!(
        !at.contains("uncommitted_edit"),
        "uncommitted edit must NOT promote"
    );
    assert!(
        !at.contains("untracked_file"),
        "untracked file must NOT promote"
    );

    // The working-tree projection (local `export`, not promotion) DOES see them
    // — proving the difference is the read source, not a filter.
    let wt = to_turtle(dir.path(), "demo").unwrap();
    assert!(
        wt.contains("uncommitted_edit"),
        "working tree sees the edit"
    );
    assert!(
        wt.contains("untracked_file"),
        "working tree sees the new file"
    );
}

/// The committed-tree read resolves any commit-ish, so promoting an OLD commit
/// promotes that commit's facts — the arbitrary-`--commit` case the old code
/// refused because it could only read the working tree.
#[test]
fn to_turtle_at_reads_an_older_commit_not_just_head() {
    let dir = tempfile::tempdir().unwrap();
    let run = |args: &[&str]| {
        assert!(std::process::Command::new("git")
            .arg("-C")
            .arg(dir.path())
            .args(args)
            .output()
            .unwrap()
            .status
            .success());
    };
    run(&["init", "-q", "-b", "main"]);
    run(&["config", "user.email", "t@t"]);
    run(&["config", "user.name", "t"]);
    std::fs::write(dir.path().join("a.rs"), "fn original() {}\n").unwrap();
    run(&["add", "-A"]);
    run(&["commit", "-qm", "first"]);
    std::fs::write(dir.path().join("a.rs"), "fn renamed() {}\n").unwrap();
    run(&["add", "-A"]);
    run(&["commit", "-qm", "second"]);

    let first = to_turtle_at(dir.path(), "demo", "HEAD~1").unwrap();
    assert!(first.contains("a.rs::original"), "old commit's facts");
    assert!(!first.contains("renamed"), "not the newer commit's");
    let head = to_turtle_at(dir.path(), "demo", "HEAD").unwrap();
    assert!(head.contains("a.rs::renamed"), "HEAD's facts");
}
