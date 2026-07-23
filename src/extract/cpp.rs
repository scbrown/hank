//! C / C++ grammar spec (behind `langs-extra`).
//!
//! Both C and C++ are parsed by `tree-sitter-cpp` (a superset grammar), matching
//! Bobbin's language set. Function and typedef names are nested inside a chain of
//! declarators rather than a flat `name` field, so this module supplies its own
//! [`symbol_name`].

use tree_sitter::Node;

use super::{field_name, GrammarSpec};
use crate::types::SymbolKind;

/// The C/C++ extraction recipe.
pub(super) fn spec() -> GrammarSpec {
    GrammarSpec {
        language: || tree_sitter_cpp::LANGUAGE.into(),
        symbol_kind,
        symbol_name,
        is_function_kind: |kind| kind == "function_definition",
        is_call_kind: |kind| kind == "call_expression",
        callee_name,
        collect_imports,
        scope_name,
    }
}

/// Namespaces and class/struct bodies open named scopes (two classes' same-named
/// member functions in one file must not share an IRI — aegis-1q14).
fn scope_name(node: Node, bytes: &[u8]) -> Option<String> {
    match node.kind() {
        "namespace_definition" | "class_specifier" | "struct_specifier" => field_name(node, bytes),
        _ => None,
    }
}

/// Map a C/C++ node to a [`SymbolKind`], if it names a symbol.
fn symbol_kind(node: Node, _bytes: &[u8]) -> Option<SymbolKind> {
    let kind = match node.kind() {
        "function_definition" => SymbolKind::Function,
        "struct_specifier" | "union_specifier" => SymbolKind::Struct,
        "class_specifier" => SymbolKind::Class,
        "enum_specifier" => SymbolKind::Enum,
        "namespace_definition" => SymbolKind::Module,
        "type_definition" => SymbolKind::TypeAlias,
        _ => return None,
    };
    Some(kind)
}

/// The symbol's name. Type/namespace/enum specifiers carry a `name` field; a
/// `function_definition` or `type_definition` hides its identifier inside a
/// declarator chain, which [`declarator_name`] unwraps.
fn symbol_name(node: Node, bytes: &[u8]) -> Option<String> {
    match node.kind() {
        "function_definition" | "type_definition" => {
            declarator_name(node.child_by_field_name("declarator")?, bytes)
        }
        _ => field_name(node, bytes),
    }
}

/// Unwrap the identifier from a (possibly pointer/reference/function/array)
/// declarator chain. `void K::m()` resolves through `function_declarator` →
/// `qualified_identifier` to `m`.
fn declarator_name(node: Node, bytes: &[u8]) -> Option<String> {
    match node.kind() {
        "identifier" | "field_identifier" | "type_identifier" | "destructor_name"
        | "operator_name" => node.utf8_text(bytes).ok().map(str::to_string),
        "qualified_identifier" => node
            .child_by_field_name("name")
            .and_then(|n| declarator_name(n, bytes)),
        _ => node
            .child_by_field_name("declarator")
            .and_then(|d| declarator_name(d, bytes)),
    }
}

/// Best-effort name of the callee invoked by a `call_expression`.
fn callee_name(call: Node, bytes: &[u8]) -> Option<String> {
    let func = call.child_by_field_name("function")?;
    match func.kind() {
        "identifier" => func.utf8_text(bytes).ok().map(str::to_string),
        "field_expression" => func
            .child_by_field_name("field")
            .and_then(|n| n.utf8_text(bytes).ok())
            .map(str::to_string),
        "qualified_identifier" => func
            .child_by_field_name("name")
            .and_then(|n| declarator_name(n, bytes)),
        _ => None,
    }
}

/// `#include <hdr>` / `#include "hdr"` name header dependencies; collect the
/// included path.
fn collect_imports(node: Node, bytes: &[u8], out: &mut Vec<String>) {
    if node.kind() == "preproc_include" {
        if let Some(path) = node.child_by_field_name("path") {
            if let Ok(text) = path.utf8_text(bytes) {
                out.push(
                    text.trim_matches(|c| matches!(c, '<' | '>' | '"'))
                        .to_string(),
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::extract::extract_structure;
    use crate::types::SymbolKind;

    const SRC: &str = "\
#include <cstdio>

struct Point { int x; };
class Widget { void render(); };
enum Color { Red, Green };
namespace ns { int inner(); }
typedef int Id;

int helper() { return 0; }

void Widget::render() {
    helper();
}
";

    #[test]
    fn extracts_cpp_symbols() {
        let s = extract_structure(SRC, "cpp").unwrap();
        let by = |n: &str| s.symbols.iter().find(|s| s.name == n).map(|s| s.kind);
        assert_eq!(by("Point"), Some(SymbolKind::Struct));
        assert_eq!(by("Widget"), Some(SymbolKind::Class));
        assert_eq!(by("Color"), Some(SymbolKind::Enum));
        assert_eq!(by("ns"), Some(SymbolKind::Module));
        assert_eq!(by("Id"), Some(SymbolKind::TypeAlias));
        assert_eq!(by("helper"), Some(SymbolKind::Function));
        // `void Widget::render()` resolves through the qualified declarator.
        assert_eq!(by("render"), Some(SymbolKind::Function));
    }

    #[test]
    fn extracts_cpp_calls_and_imports() {
        let s = extract_structure(SRC, "cpp").unwrap();
        let calls: Vec<(&str, &str)> = s
            .calls
            .iter()
            .map(|c| (c.caller.as_str(), c.callee.as_str()))
            .collect();
        assert!(calls.contains(&("render", "helper")));
        assert!(s.import_refs.contains(&"cstdio".to_string()));
    }
}
