//! Tests for `extract` — grammar selection, per-language symbol/call extraction,
//! and the multi-language guarantees. Child module of `extract` (`super::*`
//! reaches its private helpers); size-exempt (`_test.rs`).

use super::*;

fn sym(name: &str, kind: SymbolKind, start_line: usize) -> Symbol {
    Symbol {
        name: name.to_string(),
        scope: Vec::new(),
        kind,
        start_line,
        end_line: start_line,
        tier: Tier::TreeSitter,
    }
}

#[test]
fn no_collision_for_unique_names() {
    let symbols = [
        sym("run", SymbolKind::Function, 1),
        sym("walk", SymbolKind::Function, 5),
    ];
    assert!(name_collisions(&symbols).is_empty());
}

#[test]
fn same_kind_collision_needs_distinct_start_lines() {
    // Two `run` functions at different lines: the invisible variant.
    let symbols = [
        sym("run", SymbolKind::Function, 1),
        sym("run", SymbolKind::Function, 40),
    ];
    let collisions = name_collisions(&symbols);
    assert_eq!(collisions.len(), 1);
    assert!(collisions[0].same_kind());
    assert!(!collisions[0].cross_kind());

    // The same definition emitted twice is NOT a collision.
    let dup = [
        sym("run", SymbolKind::Function, 1),
        sym("run", SymbolKind::Function, 1),
    ];
    assert!(name_collisions(&dup).is_empty());
}

#[test]
fn cross_kind_collision_and_mixed() {
    // function + module sharing a name: the shape-refusable variant.
    let symbols = [
        sym("run", SymbolKind::Function, 1),
        sym("run", SymbolKind::Module, 90),
    ];
    let collisions = name_collisions(&symbols);
    assert_eq!(collisions.len(), 1);
    assert!(collisions[0].cross_kind());
    assert!(!collisions[0].same_kind());

    // Three sites, both variants at once; sites come back in line order.
    let mixed = [
        sym("run", SymbolKind::Method, 50),
        sym("run", SymbolKind::Function, 1),
        sym("run", SymbolKind::Method, 90),
    ];
    let collisions = name_collisions(&mixed);
    assert_eq!(collisions.len(), 1);
    assert!(collisions[0].same_kind());
    assert!(collisions[0].cross_kind());
    let lines: Vec<usize> = collisions[0].sites.iter().map(|(_, l)| *l).collect();
    assert_eq!(lines, vec![1, 50, 90]);
}

#[test]
fn extracts_rust_symbols() {
    let source = r#"
struct Point { x: i32 }

enum Shape { Circle, Square }

const MAX: usize = 10;

fn add(a: i32, b: i32) -> i32 { a + b }

trait Greet { fn hello(&self); }
"#;
    let symbols = extract_symbols(source, "rust").unwrap();
    let names: Vec<&str> = symbols.iter().map(|s| s.name.as_str()).collect();
    assert!(names.contains(&"Point"));
    assert!(names.contains(&"Shape"));
    assert!(names.contains(&"MAX"));
    assert!(names.contains(&"add"));
    assert!(names.contains(&"Greet"));

    let add = symbols.iter().find(|s| s.name == "add").unwrap();
    assert_eq!(add.kind, SymbolKind::Function);
    assert_eq!(add.tier, Tier::TreeSitter);
}

#[test]
fn nested_methods_are_found() {
    let source = "struct S; impl S { fn method(&self) {} }";
    let symbols = extract_symbols(source, "rust").unwrap();
    assert!(symbols.iter().any(|s| s.name == "method"));
}

/// The aegis-1q14 collision anatomy, from bobbin's live census: a top-level
/// `mod run;` and an impl-scoped `fn run` in one file must carry DIFFERENT
/// scope chains — that difference is what keeps their IRIs distinct.
#[test]
fn same_named_symbols_in_different_scopes_carry_different_scope_chains() {
    let source = "\
mod run;
struct Cli;
impl Cli { pub fn run(&self) {} }
";
    let symbols = extract_symbols(source, "rust").unwrap();
    let runs: Vec<&Symbol> = symbols.iter().filter(|s| s.name == "run").collect();
    assert_eq!(runs.len(), 2, "both `run` symbols extracted");
    let scopes: Vec<&[String]> = runs.iter().map(|s| s.scope.as_slice()).collect();
    assert!(scopes.contains(&&[][..]), "the mod decl is top-level");
    assert!(
        scopes.contains(&&["Cli".to_string()][..]),
        "the method is impl-scoped, got {scopes:?}"
    );
}

/// Two trait impls on the SAME type both define `fmt`; the type name alone
/// would still collide, so the impl scope carries the trait too.
#[test]
fn trait_impls_on_one_type_get_distinct_scopes() {
    let source = "\
struct A;
impl std::fmt::Debug for A { fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result { Ok(()) } }
impl std::fmt::Display for A { fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result { Ok(()) } }
";
    let symbols = extract_symbols(source, "rust").unwrap();
    let scopes: Vec<String> = symbols
        .iter()
        .filter(|s| s.name == "fmt")
        .map(|s| s.scope.join("::"))
        .collect();
    assert_eq!(scopes.len(), 2);
    assert_ne!(scopes[0], scopes[1], "trait discriminates: {scopes:?}");
}

/// Nested named scopes stack: a fn inside a mod inside a mod.
#[test]
fn scope_chains_nest_outermost_first() {
    let source = "mod outer { mod inner { fn leaf() {} } }";
    let symbols = extract_symbols(source, "rust").unwrap();
    let leaf = symbols.iter().find(|s| s.name == "leaf").unwrap();
    assert_eq!(leaf.scope, vec!["outer".to_string(), "inner".to_string()]);
}

#[test]
fn extracts_call_sites() {
    let source = "\
fn helper() {}
fn caller() { helper(); other::thing(); }
";
    let structure = extract_structure(source, "rust").unwrap();
    let calls: Vec<(&str, &str)> = structure
        .calls
        .iter()
        .map(|c| (c.caller.as_str(), c.callee.as_str()))
        .collect();
    assert!(calls.contains(&("caller", "helper")));
    assert!(calls.contains(&("caller", "thing")));
}

#[test]
fn extracts_import_refs() {
    let source = "\
use crate::graph::reachable;
use std::collections::HashMap;
mod extract;
fn f() {}
";
    let structure = extract_structure(source, "rust").unwrap();
    // `use` path segments and the bodiless `mod` name are collected; path
    // anchors (`crate`) are dropped.
    assert!(structure.import_refs.contains(&"graph".to_string()));
    assert!(structure.import_refs.contains(&"collections".to_string()));
    assert!(structure.import_refs.contains(&"extract".to_string()));
    assert!(!structure.import_refs.contains(&"crate".to_string()));
}

#[test]
fn inline_mod_is_not_an_import() {
    // A `mod foo { ... }` with a body defines a symbol, not a file import.
    let source = "mod inner { fn g() {} }";
    let structure = extract_structure(source, "rust").unwrap();
    assert!(!structure.import_refs.contains(&"inner".to_string()));
    assert!(structure.symbols.iter().any(|s| s.name == "inner"));
}

#[test]
fn unsupported_language_errors() {
    let err = extract_symbols("", "cobol").unwrap_err();
    assert!(matches!(err, Error::UnsupportedLanguage(_)));
}

#[test]
fn rust_extension_maps() {
    assert_eq!(language_for_extension("rs"), Some("rust"));
    assert_eq!(language_for_extension("cobol"), None);
}
