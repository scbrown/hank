//! Fast structural extraction via tree-sitter — the always-on breadth tier.
//!
//! This is Hank's build-free extractor: it works on a syntactically-broken
//! buffer and produces a symbol tree with line spans, tagged [`Tier::TreeSitter`].
//! Precise LSP facts (`lsp` feature) and CPG/dataflow (`cpg` feature) layer on
//! top in later phases. Today only Rust is wired; the remaining grammars in
//! Bobbin's set arrive behind the `langs-extra` feature.

use std::path::{Path, PathBuf};

use tree_sitter::Parser;

use crate::errors::{Error, Result};
use crate::types::{Symbol, SymbolKind, Tier};

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
    let mut cursor = tree.walk();
    let mut stack = vec![tree.root_node()];

    while let Some(node) = stack.pop() {
        if let Some(kind) = symbol_kind(node.kind()) {
            if let Some(name_node) = node.child_by_field_name("name") {
                if let Ok(name) = name_node.utf8_text(bytes) {
                    symbols.push(Symbol {
                        name: name.to_string(),
                        kind,
                        start_line: node.start_position().row + 1,
                        end_line: node.end_position().row + 1,
                        tier: Tier::TreeSitter,
                    });
                }
            }
        }
        for child in node.children(&mut cursor) {
            stack.push(child);
        }
    }

    symbols.sort_by_key(|symbol| symbol.start_line);
    Ok(symbols)
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
    fn unsupported_language_errors() {
        let err = extract_symbols("", "cobol").unwrap_err();
        assert!(matches!(err, Error::UnsupportedLanguage(_)));
    }
}
