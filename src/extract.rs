//! Fast structural extraction via tree-sitter — the always-on breadth tier.
//!
//! This is Hank's build-free extractor: it works on a syntactically-broken
//! buffer and produces a symbol tree with line spans and intra-file call sites,
//! tagged [`Tier::TreeSitter`]. Precise LSP facts (`lsp` feature) and
//! CPG/dataflow (`cpg` feature) layer on top in later phases. Today only Rust is
//! wired; the remaining grammars in Bobbin's set arrive behind `langs-extra`.

use std::path::{Path, PathBuf};

use tree_sitter::{Node, Parser};

use crate::errors::{Error, Result};
use crate::types::{Symbol, SymbolKind, Tier};

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
pub fn extract_structure(source: &str, language: &str) -> Result<FileStructure> {
    if language != "rust" {
        return Err(Error::UnsupportedLanguage(language.to_string()));
    }

    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_rust::LANGUAGE.into())
        .map_err(|e| Error::Parse(e.to_string()))?;
    let tree = parser
        .parse(source, None)
        .ok_or_else(|| Error::Parse("tree-sitter produced no tree".to_string()))?;

    let bytes = source.as_bytes();
    let mut symbols = Vec::new();
    let mut calls = Vec::new();
    // Each frame carries the name of the nearest enclosing function, so a call
    // site can be attributed to its caller.
    let mut stack: Vec<(Node, Option<String>)> = vec![(tree.root_node(), None)];

    while let Some((node, enclosing)) = stack.pop() {
        let mut inner = enclosing.clone();

        if let Some(kind) = symbol_kind(node.kind()) {
            if let Some(name) = node
                .child_by_field_name("name")
                .and_then(|n| n.utf8_text(bytes).ok())
            {
                symbols.push(Symbol {
                    name: name.to_string(),
                    kind,
                    start_line: node.start_position().row + 1,
                    end_line: node.end_position().row + 1,
                    tier: Tier::TreeSitter,
                });
                if node.kind() == "function_item" {
                    inner = Some(name.to_string());
                }
            }
        }

        if node.kind() == "call_expression" {
            if let (Some(caller), Some(callee)) = (
                &enclosing,
                node.child_by_field_name("function")
                    .and_then(|f| callee_name(f, bytes)),
            ) {
                calls.push(CallSite {
                    caller: caller.clone(),
                    callee,
                    line: node.start_position().row + 1,
                });
            }
        }

        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            stack.push((child, inner.clone()));
        }
    }

    symbols.sort_by_key(|symbol| symbol.start_line);
    Ok(FileStructure { symbols, calls })
}

/// Best-effort name of the function invoked by a `call_expression`'s callee.
fn callee_name(func: Node, bytes: &[u8]) -> Option<String> {
    match func.kind() {
        "identifier" => func.utf8_text(bytes).ok().map(str::to_string),
        "field_expression" => func
            .child_by_field_name("field")
            .and_then(|n| n.utf8_text(bytes).ok())
            .map(str::to_string),
        "scoped_identifier" => func
            .child_by_field_name("name")
            .and_then(|n| n.utf8_text(bytes).ok())
            .map(str::to_string),
        "generic_function" => func
            .child_by_field_name("function")
            .and_then(|f| callee_name(f, bytes)),
        _ => None,
    }
}

/// Map a Rust tree-sitter node kind to a [`SymbolKind`], if it names a symbol.
fn symbol_kind(node_kind: &str) -> Option<SymbolKind> {
    let kind = match node_kind {
        "function_item" => SymbolKind::Function,
        "struct_item" => SymbolKind::Struct,
        "enum_item" => SymbolKind::Enum,
        "union_item" => SymbolKind::Struct,
        "trait_item" => SymbolKind::Interface,
        "mod_item" => SymbolKind::Module,
        "const_item" | "static_item" => SymbolKind::Constant,
        "type_item" => SymbolKind::TypeAlias,
        _ => return None,
    };
    Some(kind)
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
    fn unsupported_language_errors() {
        let err = extract_symbols("", "cobol").unwrap_err();
        assert!(matches!(err, Error::UnsupportedLanguage(_)));
    }
}
