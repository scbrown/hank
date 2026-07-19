//! Java grammar spec (behind `langs-extra`).

use tree_sitter::Node;

use super::{collect_path_idents, field_name, GrammarSpec};
use crate::types::SymbolKind;

/// The Java extraction recipe.
pub(super) fn spec() -> GrammarSpec {
    GrammarSpec {
        language: || tree_sitter_java::LANGUAGE.into(),
        symbol_kind,
        symbol_name: field_name,
        is_function_kind: |kind| matches!(kind, "method_declaration" | "constructor_declaration"),
        is_call_kind: |kind| kind == "method_invocation",
        callee_name,
        collect_imports,
    }
}

/// Map a Java node to a [`SymbolKind`], if it names a symbol.
fn symbol_kind(node: Node, _bytes: &[u8]) -> Option<SymbolKind> {
    let kind = match node.kind() {
        "class_declaration" => SymbolKind::Class,
        "interface_declaration" => SymbolKind::Interface,
        "enum_declaration" => SymbolKind::Enum,
        "method_declaration" => SymbolKind::Method,
        "constructor_declaration" => SymbolKind::Constructor,
        _ => return None,
    };
    Some(kind)
}

/// Best-effort name of the callee invoked by a `method_invocation`. The invoked
/// method name is on the node's `name` field directly.
fn callee_name(call: Node, bytes: &[u8]) -> Option<String> {
    call.child_by_field_name("name")
        .and_then(|n| n.utf8_text(bytes).ok())
        .map(str::to_string)
}

/// `import java.util.List;` names a type dependency; collect the dotted-path
/// identifier segments.
fn collect_imports(node: Node, bytes: &[u8], out: &mut Vec<String>) {
    if node.kind() == "import_declaration" {
        collect_path_idents(node, bytes, &[], out);
    }
}

#[cfg(test)]
mod tests {
    use crate::extract::extract_structure;
    use crate::types::SymbolKind;

    const SRC: &str = "\
import java.util.List;

class Widget {
    Widget() {}
    void render() {
        helper();
        this.paint();
    }
    void helper() {}
}

interface Shape {}

enum Color { RED, GREEN }
";

    #[test]
    fn extracts_java_symbols() {
        let s = extract_structure(SRC, "java").unwrap();
        let by = |n: &str| s.symbols.iter().find(|s| s.name == n).map(|s| s.kind);
        assert_eq!(by("Widget"), Some(SymbolKind::Class));
        assert_eq!(by("Shape"), Some(SymbolKind::Interface));
        assert_eq!(by("Color"), Some(SymbolKind::Enum));
        assert_eq!(by("render"), Some(SymbolKind::Method));
        // The constructor shares the class name; assert a Constructor symbol exists.
        assert!(s
            .symbols
            .iter()
            .any(|s| s.name == "Widget" && s.kind == SymbolKind::Constructor));
    }

    #[test]
    fn extracts_java_calls_and_imports() {
        let s = extract_structure(SRC, "java").unwrap();
        let calls: Vec<(&str, &str)> = s
            .calls
            .iter()
            .map(|c| (c.caller.as_str(), c.callee.as_str()))
            .collect();
        assert!(calls.contains(&("render", "helper")));
        assert!(calls.contains(&("render", "paint")));
        assert!(s.import_refs.contains(&"List".to_string()));
    }
}
