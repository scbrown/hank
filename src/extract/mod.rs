//! Fast structural extraction via tree-sitter — the always-on breadth tier.
//!
//! This is Hank's build-free extractor: it works on a syntactically-broken
//! buffer and produces a symbol tree with line spans and intra-file call sites,
//! tagged [`Tier::TreeSitter`]. Precise LSP facts (`lsp` feature) and
//! CPG/dataflow (`cpg` feature) layer on top in later phases.
//!
//! ## Grammar registry
//!
//! Rust is always built. Bobbin's remaining grammar set — TypeScript/TSX,
//! Python, Go, Java, C/C++ — arrives behind the `langs-extra` feature. Each
//! language lives in its own sibling module and contributes a [`GrammarSpec`]
//! (grammar + node-kind → [`SymbolKind`] mapping + call/import extraction). The
//! generic walker in this module is language-agnostic: it drives whichever spec
//! [`grammar_spec`] returns for the requested language.

use std::path::{Path, PathBuf};

use tree_sitter::{Node, Parser};

use crate::errors::{Error, Result};
use crate::types::{Symbol, SymbolKind, Tier};

#[cfg(feature = "langs-extra")]
mod cpp;
#[cfg(feature = "langs-extra")]
mod go;
#[cfg(feature = "langs-extra")]
mod java;
#[cfg(feature = "langs-extra")]
mod python;
mod rust;
#[cfg(feature = "langs-extra")]
mod typescript;

/// A resolved-by-name call site: `caller` invokes `callee` at `line`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallSite {
    /// Name of the enclosing function the call is made from.
    pub caller: String,
    /// Name of the invoked function/method (best-effort, by name).
    pub callee: String,
    /// 1-based line of the call.
    pub line: usize,
}

/// The structure extracted from one source file: its symbols and call sites.
#[derive(Debug, Clone, Default)]
pub struct FileStructure {
    /// Named symbols defined in the file.
    pub symbols: Vec<Symbol>,
    /// Intra-file call sites (caller/callee by name).
    pub calls: Vec<CallSite>,
    /// Candidate module-name references from `import` / `use` / `include`
    /// declarations. These are the path segments seen in imports (best-effort,
    /// by name — the tree-sitter tier); the exporter resolves them to sibling
    /// modules by matching module stems (§9.2 `bobbin:imports`,
    /// [`Tier::TreeSitter`]).
    pub import_refs: Vec<String>,
}

/// A per-language extraction recipe. Each grammar module builds one of these; the
/// generic [`walk`] below is driven entirely by these function pointers, so no
/// language-specific logic lives in the walker itself.
///
/// `Node` is `Copy`, so passing it by value costs nothing.
pub(crate) struct GrammarSpec {
    /// The tree-sitter grammar for this language.
    pub language: fn() -> tree_sitter::Language,
    /// Map a node to the kind of symbol it defines, if any.
    pub symbol_kind: fn(Node, &[u8]) -> Option<SymbolKind>,
    /// The declared name of a symbol node (for caller attribution + the symbol).
    pub symbol_name: fn(Node, &[u8]) -> Option<String>,
    /// Whether a node kind introduces a callable scope (so calls inside it are
    /// attributed to it as their caller).
    pub is_function_kind: fn(&str) -> bool,
    /// Whether a node kind is a call site.
    pub is_call_kind: fn(&str) -> bool,
    /// Best-effort name of the callee invoked by a call node.
    pub callee_name: fn(Node, &[u8]) -> Option<String>,
    /// Collect any module-name references contributed by this node.
    pub collect_imports: fn(Node, &[u8], &mut Vec<String>),
}

/// Look up the extraction recipe for `language`, or `None` if this build cannot
/// parse it. The extra grammars are gated behind `langs-extra`.
fn grammar_spec(language: &str) -> Option<GrammarSpec> {
    match language {
        "rust" => Some(rust::spec()),
        #[cfg(feature = "langs-extra")]
        "typescript" => Some(typescript::spec_typescript()),
        #[cfg(feature = "langs-extra")]
        "tsx" => Some(typescript::spec_tsx()),
        #[cfg(feature = "langs-extra")]
        "python" => Some(python::spec()),
        #[cfg(feature = "langs-extra")]
        "go" => Some(go::spec()),
        #[cfg(feature = "langs-extra")]
        "java" => Some(java::spec()),
        #[cfg(feature = "langs-extra")]
        "cpp" => Some(cpp::spec()),
        _ => None,
    }
}

/// Map a source-file extension to the canonical language name understood by
/// [`extract_structure`], or `None` if this build has no grammar for it.
///
/// Only extensions whose grammar is compiled into the current build are
/// returned, so the map never promises a language the extractor would reject.
#[must_use]
pub fn language_for_extension(ext: &str) -> Option<&'static str> {
    let language = match ext {
        "rs" => "rust",
        #[cfg(feature = "langs-extra")]
        "ts" | "mts" | "cts" | "js" | "mjs" | "cjs" => "typescript",
        #[cfg(feature = "langs-extra")]
        "tsx" | "jsx" => "tsx",
        #[cfg(feature = "langs-extra")]
        "py" | "pyi" => "python",
        #[cfg(feature = "langs-extra")]
        "go" => "go",
        #[cfg(feature = "langs-extra")]
        "java" => "java",
        #[cfg(feature = "langs-extra")]
        "c" | "h" | "cc" | "cpp" | "cxx" | "hpp" | "hh" | "hxx" => "cpp",
        _ => return None,
    };
    Some(language)
}

/// Walk `path` for Rust source files, honoring `.gitignore`.
#[must_use]
pub fn rust_files(path: &Path) -> Vec<PathBuf> {
    ignore::WalkBuilder::new(path)
        .build()
        .filter_map(std::result::Result::ok)
        .map(ignore::DirEntry::into_path)
        .filter(|p| p.extension().is_some_and(|ext| ext == "rs"))
        .collect()
}

/// Extract the named symbols from `source`, written in `language`.
///
/// Returns [`Error::UnsupportedLanguage`] for a language this build cannot
/// parse.
pub fn extract_symbols(source: &str, language: &str) -> Result<Vec<Symbol>> {
    Ok(extract_structure(source, language)?.symbols)
}

/// Extract symbols and call sites from `source`, written in `language`.
///
/// Returns [`Error::UnsupportedLanguage`] for a language this build cannot
/// parse.
pub fn extract_structure(source: &str, language: &str) -> Result<FileStructure> {
    let spec =
        grammar_spec(language).ok_or_else(|| Error::UnsupportedLanguage(language.to_string()))?;

    let mut parser = Parser::new();
    parser
        .set_language(&(spec.language)())
        .map_err(|e| Error::Parse(e.to_string()))?;
    let tree = parser
        .parse(source, None)
        .ok_or_else(|| Error::Parse("tree-sitter produced no tree".to_string()))?;

    Ok(walk(&spec, tree.root_node(), source.as_bytes()))
}

/// Language-agnostic traversal: collect symbols, intra-file calls, and import
/// references by asking `spec` about each node. Iterative so a deeply-nested
/// tree can't overflow the stack.
fn walk(spec: &GrammarSpec, root: Node, bytes: &[u8]) -> FileStructure {
    let mut symbols = Vec::new();
    let mut calls = Vec::new();
    let mut import_refs = Vec::new();
    // Each frame carries the name of the nearest enclosing function, so a call
    // site can be attributed to its caller.
    let mut stack: Vec<(Node, Option<String>)> = vec![(root, None)];

    while let Some((node, enclosing)) = stack.pop() {
        let mut inner = enclosing.clone();

        if let Some(kind) = (spec.symbol_kind)(node, bytes) {
            if let Some(name) = (spec.symbol_name)(node, bytes) {
                symbols.push(Symbol {
                    name: name.clone(),
                    kind,
                    start_line: node.start_position().row + 1,
                    end_line: node.end_position().row + 1,
                    tier: Tier::TreeSitter,
                });
                if (spec.is_function_kind)(node.kind()) {
                    inner = Some(name);
                }
            }
        }

        if (spec.is_call_kind)(node.kind()) {
            if let (Some(caller), Some(callee)) = (&enclosing, (spec.callee_name)(node, bytes)) {
                calls.push(CallSite {
                    caller: caller.clone(),
                    callee,
                    line: node.start_position().row + 1,
                });
            }
        }

        (spec.collect_imports)(node, bytes, &mut import_refs);

        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            stack.push((child, inner.clone()));
        }
    }

    symbols.sort_by_key(|symbol| symbol.start_line);
    import_refs.sort();
    import_refs.dedup();
    FileStructure {
        symbols,
        calls,
        import_refs,
    }
}

/// The text of a node's `name` field — the common case for symbol naming across
/// grammars. Languages whose symbol name is nested (e.g. C/C++ declarators)
/// supply their own `symbol_name`.
pub(crate) fn field_name(node: Node, bytes: &[u8]) -> Option<String> {
    node.child_by_field_name("name")
        .and_then(|n| n.utf8_text(bytes).ok())
        .map(str::to_string)
}

/// Collect every `identifier` under `node` into `out`, dropping any of the given
/// path `anchors` (`crate` / `self` / `super` for Rust). Shared by grammars
/// whose imports are dotted identifier paths.
pub(crate) fn collect_path_idents(
    node: Node,
    bytes: &[u8],
    anchors: &[&str],
    out: &mut Vec<String>,
) {
    let mut stack = vec![node];
    while let Some(n) = stack.pop() {
        if n.kind() == "identifier" {
            if let Ok(text) = n.utf8_text(bytes) {
                if !anchors.contains(&text) {
                    out.push(text.to_string());
                }
            }
        }
        let mut cursor = n.walk();
        for child in n.children(&mut cursor) {
            stack.push(child);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
