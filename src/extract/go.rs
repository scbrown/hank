//! Go grammar spec (behind `langs-extra`).

use tree_sitter::Node;

use super::{field_name, GrammarSpec};
use crate::types::SymbolKind;

/// The Go extraction recipe.
pub(super) fn spec() -> GrammarSpec {
    GrammarSpec {
        language: || tree_sitter_go::LANGUAGE.into(),
        symbol_kind,
        symbol_name: field_name,
        is_function_kind: |kind| matches!(kind, "function_declaration" | "method_declaration"),
        is_call_kind: |kind| kind == "call_expression",
        callee_name,
        collect_imports,
        scope_name,
    }
}

/// A method's receiver type is its scope: `func (a *A) Run()` and
/// `func (b *B) Run()` in one file are different symbols (aegis-1q14).
fn scope_name(node: Node, bytes: &[u8]) -> Option<String> {
    if node.kind() != "method_declaration" {
        return None;
    }
    let recv = node.child_by_field_name("receiver")?;
    // receiver: parameter_list -> parameter_declaration -> type (possibly
    // behind a pointer_type). Take the identifier text of the type.
    let mut cursor = recv.walk();
    for child in recv.children(&mut cursor) {
        if child.kind() == "parameter_declaration" {
            if let Some(ty) = child.child_by_field_name("type") {
                let inner = if ty.kind() == "pointer_type" {
                    ty.named_child(0).unwrap_or(ty)
                } else {
                    ty
                };
                return inner.utf8_text(bytes).ok().map(str::to_string);
            }
        }
    }
    None
}

/// Map a Go node to a [`SymbolKind`], if it names a symbol.
///
/// A `type_spec` is classified by the type it binds: a `struct_type` →
/// [`SymbolKind::Struct`], an `interface_type` → [`SymbolKind::Interface`],
/// anything else → [`SymbolKind::TypeAlias`]. A `type_alias` (`type A = B`) is
/// always a [`SymbolKind::TypeAlias`].
fn symbol_kind(node: Node, _bytes: &[u8]) -> Option<SymbolKind> {
    let kind = match node.kind() {
        "function_declaration" => SymbolKind::Function,
        "method_declaration" => SymbolKind::Method,
        "type_alias" => SymbolKind::TypeAlias,
        "type_spec" => match node.child_by_field_name("type").map(|t| t.kind()) {
            Some("struct_type") => SymbolKind::Struct,
            Some("interface_type") => SymbolKind::Interface,
            _ => SymbolKind::TypeAlias,
        },
        _ => return None,
    };
    Some(kind)
}

/// Best-effort name of the callee invoked by a `call_expression`.
fn callee_name(call: Node, bytes: &[u8]) -> Option<String> {
    let func = call.child_by_field_name("function")?;
    match func.kind() {
        "identifier" => func.utf8_text(bytes).ok().map(str::to_string),
        "selector_expression" => func
            .child_by_field_name("field")
            .and_then(|n| n.utf8_text(bytes).ok())
            .map(str::to_string),
        _ => None,
    }
}

/// `import "fmt"` names a package dependency; collect the quoted import path.
fn collect_imports(node: Node, bytes: &[u8], out: &mut Vec<String>) {
    if node.kind() == "import_spec" {
        if let Some(path) = node.child_by_field_name("path") {
            let mut cursor = path.walk();
            for child in path.children(&mut cursor) {
                if child.kind() == "interpreted_string_literal_content" {
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
package main

import \"fmt\"

type Widget struct { x int }
type Shape interface { Area() int }
type Id = int

func helper() {}

func (w Widget) Render() {
    helper()
    fmt.Println()
}
";

    #[test]
    fn extracts_go_symbols() {
        let s = extract_structure(SRC, "go").unwrap();
        let by = |n: &str| s.symbols.iter().find(|s| s.name == n).map(|s| s.kind);
        assert_eq!(by("Widget"), Some(SymbolKind::Struct));
        assert_eq!(by("Shape"), Some(SymbolKind::Interface));
        assert_eq!(by("Id"), Some(SymbolKind::TypeAlias));
        assert_eq!(by("helper"), Some(SymbolKind::Function));
        assert_eq!(by("Render"), Some(SymbolKind::Method));
    }

    #[test]
    fn extracts_go_calls_and_imports() {
        let s = extract_structure(SRC, "go").unwrap();
        let calls: Vec<(&str, &str)> = s
            .calls
            .iter()
            .map(|c| (c.caller.as_str(), c.callee.as_str()))
            .collect();
        assert!(calls.contains(&("Render", "helper")));
        assert!(calls.contains(&("Render", "Println")));
        assert!(s.import_refs.contains(&"fmt".to_string()));
    }
}
