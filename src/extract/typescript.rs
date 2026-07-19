//! TypeScript and TSX grammar specs.
//!
//! The two dialects share every extraction rule; only the underlying grammar
//! differs (TSX also parses JSX). Both are wired behind `langs-extra`.

use tree_sitter::Node;

use super::{field_name, GrammarSpec};
use crate::types::SymbolKind;

/// The TypeScript extraction recipe.
pub(super) fn spec_typescript() -> GrammarSpec {
    GrammarSpec {
        language: || tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
        ..common()
    }
}

/// The TSX extraction recipe (TypeScript + JSX).
pub(super) fn spec_tsx() -> GrammarSpec {
    GrammarSpec {
        language: || tree_sitter_typescript::LANGUAGE_TSX.into(),
        ..common()
    }
}

/// The shared TS/TSX recipe minus the grammar pointer.
fn common() -> GrammarSpec {
    GrammarSpec {
        language: || tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
        symbol_kind,
        symbol_name: field_name,
        is_function_kind: |kind| {
            matches!(
                kind,
                "function_declaration"
                    | "generator_function_declaration"
                    | "method_definition"
                    | "function_expression"
            )
        },
        is_call_kind: |kind| kind == "call_expression",
        callee_name,
        collect_imports,
    }
}

/// Map a TypeScript node to a [`SymbolKind`], if it names a symbol.
fn symbol_kind(node: Node, _bytes: &[u8]) -> Option<SymbolKind> {
    let kind = match node.kind() {
        "function_declaration" | "generator_function_declaration" => SymbolKind::Function,
        "method_definition" => SymbolKind::Method,
        "class_declaration" | "abstract_class_declaration" => SymbolKind::Class,
        "interface_declaration" => SymbolKind::Interface,
        "enum_declaration" => SymbolKind::Enum,
        "type_alias_declaration" => SymbolKind::TypeAlias,
        _ => return None,
    };
    Some(kind)
}

/// Best-effort name of the callee invoked by a `call_expression`.
fn callee_name(call: Node, bytes: &[u8]) -> Option<String> {
    let func = call.child_by_field_name("function")?;
    match func.kind() {
        "identifier" => func.utf8_text(bytes).ok().map(str::to_string),
        "member_expression" => func
            .child_by_field_name("property")
            .and_then(|n| n.utf8_text(bytes).ok())
            .map(str::to_string),
        _ => None,
    }
}

/// An `import ... from "mod"` names a module dependency. Collect the quoted
/// source string (best-effort, resolved to a sibling stem in the exporter).
fn collect_imports(node: Node, bytes: &[u8], out: &mut Vec<String>) {
    if node.kind() == "import_statement" {
        if let Some(source) = node.child_by_field_name("source") {
            let mut cursor = source.walk();
            for child in source.children(&mut cursor) {
                if child.kind() == "string_fragment" {
                    if let Ok(text) = child.utf8_text(bytes) {
                        out.push(text.to_string());
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::extract::extract_structure;
    use crate::types::SymbolKind;

    const SRC: &str = "\
import { foo } from './bar';
interface Shape { area(): number }
enum Color { Red, Green }
type Id = string;
class Widget {
    render(): void {
        this.paint();
        helper();
    }
}
function helper(): void {}
";

    #[test]
    fn extracts_typescript_symbols() {
        let s = extract_structure(SRC, "typescript").unwrap();
        let by = |n: &str| s.symbols.iter().find(|s| s.name == n).map(|s| s.kind);
        assert_eq!(by("helper"), Some(SymbolKind::Function));
        assert_eq!(by("render"), Some(SymbolKind::Method));
        assert_eq!(by("Widget"), Some(SymbolKind::Class));
        assert_eq!(by("Shape"), Some(SymbolKind::Interface));
        assert_eq!(by("Color"), Some(SymbolKind::Enum));
        assert_eq!(by("Id"), Some(SymbolKind::TypeAlias));
    }

    #[test]
    fn extracts_typescript_calls_and_imports() {
        let s = extract_structure(SRC, "typescript").unwrap();
        let calls: Vec<(&str, &str)> = s
            .calls
            .iter()
            .map(|c| (c.caller.as_str(), c.callee.as_str()))
            .collect();
        assert!(calls.contains(&("render", "paint")));
        assert!(calls.contains(&("render", "helper")));
        assert!(s.import_refs.contains(&"./bar".to_string()));
    }

    #[test]
    fn tsx_parses_jsx() {
        let src = "function App() { return doWork(); }";
        let s = extract_structure(src, "tsx").unwrap();
        assert!(s.symbols.iter().any(|s| s.name == "App"));
        assert!(s
            .calls
            .iter()
            .any(|c| c.caller == "App" && c.callee == "doWork"));
    }
}
