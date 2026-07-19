//! Python grammar spec (behind `langs-extra`).

use tree_sitter::Node;

use super::{collect_path_idents, field_name, GrammarSpec};
use crate::types::SymbolKind;

/// The Python extraction recipe.
pub(super) fn spec() -> GrammarSpec {
    GrammarSpec {
        language: || tree_sitter_python::LANGUAGE.into(),
        symbol_kind,
        symbol_name: field_name,
        is_function_kind: |kind| kind == "function_definition",
        is_call_kind: |kind| kind == "call",
        callee_name,
        collect_imports,
    }
}

/// Map a Python node to a [`SymbolKind`], if it names a symbol.
fn symbol_kind(node: Node, _bytes: &[u8]) -> Option<SymbolKind> {
    let kind = match node.kind() {
        "function_definition" => SymbolKind::Function,
        "class_definition" => SymbolKind::Class,
        _ => return None,
    };
    Some(kind)
}

/// Best-effort name of the callee invoked by a `call`.
fn callee_name(call: Node, bytes: &[u8]) -> Option<String> {
    let func = call.child_by_field_name("function")?;
    match func.kind() {
        "identifier" => func.utf8_text(bytes).ok().map(str::to_string),
        "attribute" => func
            .child_by_field_name("attribute")
            .and_then(|n| n.utf8_text(bytes).ok())
            .map(str::to_string),
        _ => None,
    }
}

/// `import a.b` / `from a.b import c` name module dependencies; collect the
/// dotted-path identifier segments.
fn collect_imports(node: Node, bytes: &[u8], out: &mut Vec<String>) {
    if matches!(node.kind(), "import_statement" | "import_from_statement") {
        collect_path_idents(node, bytes, &[], out);
    }
}

#[cfg(test)]
mod tests {
    use crate::extract::extract_structure;
    use crate::types::SymbolKind;

    const SRC: &str = "\
import os
from a.b import c

class Widget:
    def render(self):
        self.paint()
        helper()

def helper():
    pass
";

    #[test]
    fn extracts_python_symbols() {
        let s = extract_structure(SRC, "python").unwrap();
        let by = |n: &str| s.symbols.iter().find(|s| s.name == n).map(|s| s.kind);
        assert_eq!(by("Widget"), Some(SymbolKind::Class));
        assert_eq!(by("render"), Some(SymbolKind::Function));
        assert_eq!(by("helper"), Some(SymbolKind::Function));
    }

    #[test]
    fn extracts_python_calls_and_imports() {
        let s = extract_structure(SRC, "python").unwrap();
        let calls: Vec<(&str, &str)> = s
            .calls
            .iter()
            .map(|c| (c.caller.as_str(), c.callee.as_str()))
            .collect();
        assert!(calls.contains(&("render", "paint")));
        assert!(calls.contains(&("render", "helper")));
        assert!(s.import_refs.contains(&"os".to_string()));
        assert!(s.import_refs.contains(&"b".to_string()));
    }
}
