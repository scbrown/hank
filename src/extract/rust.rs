//! Rust grammar spec — the always-on default extractor.

use tree_sitter::Node;

use super::{collect_path_idents, field_name, GrammarSpec};
use crate::types::SymbolKind;

/// The Rust extraction recipe (see [`GrammarSpec`]).
pub(super) fn spec() -> GrammarSpec {
    GrammarSpec {
        language: || tree_sitter_rust::LANGUAGE.into(),
        symbol_kind,
        symbol_name: field_name,
        is_function_kind: |kind| kind == "function_item",
        is_call_kind: |kind| kind == "call_expression",
        callee_name,
        collect_imports,
    }
}

/// Map a Rust node to a [`SymbolKind`], if it names a symbol.
fn symbol_kind(node: Node, _bytes: &[u8]) -> Option<SymbolKind> {
    let kind = match node.kind() {
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

/// Best-effort name of the function invoked by a `call_expression`'s callee.
fn callee_name(call: Node, bytes: &[u8]) -> Option<String> {
    let func = call.child_by_field_name("function")?;
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
        "generic_function" => func.child_by_field_name("function").and_then(|f| {
            // Re-wrap in a synthetic call so we can reuse the same resolution.
            callee_of(f, bytes)
        }),
        _ => None,
    }
}

/// Resolve a callee expression node directly (used for `generic_function`).
fn callee_of(func: Node, bytes: &[u8]) -> Option<String> {
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
        _ => None,
    }
}

/// A `use ...;` names the modules this file depends on; a bodiless `mod foo;`
/// pulls in a sibling file module. Collect the path segments as best-effort
/// module-name references (resolved in the exporter).
fn collect_imports(node: Node, bytes: &[u8], out: &mut Vec<String>) {
    match node.kind() {
        "use_declaration" => collect_path_idents(node, bytes, &["crate", "self", "super"], out),
        "mod_item" if node.child_by_field_name("body").is_none() => {
            if let Some(name) = field_name(node, bytes) {
                out.push(name);
            }
        }
        _ => {}
    }
}
