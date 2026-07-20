//! A focused Rust pass over a *proposed* edit buffer.
//!
//! [`crate::extract`] answers "what structure does this file have?". Verification
//! asks narrower questions the shared extractor deliberately does not carry:
//! how many arguments does this call site pass, was the callee a plain function
//! or a method, and which names are already bound in scope. Those live here so
//! the shared fact model stays lean.
//!
//! Everything collected is *syntactic*. The suppressions below exist because a
//! false "this identifier does not exist" is far worse than a missed one — the
//! verdict feeds a guard that can block an agent.

use std::collections::BTreeSet;

use tree_sitter::{Node, Parser};

use crate::errors::{Error, Result};

/// How a call was written — which determines whether it can be checked at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallForm {
    /// `foo(..)` — a bare identifier. The only form resolvable by name alone.
    Free,
    /// `a::b::foo(..)` — may belong to an external crate, so unresolvable here.
    Path,
    /// `x.foo(..)` — needs the receiver's type, i.e. the LSP tier.
    Method,
}

/// A call site in the buffer, with the detail verification needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Call {
    /// Best-effort callee name.
    pub name: String,
    /// How the call was written.
    pub form: CallForm,
    /// Number of arguments passed.
    pub arity: usize,
    /// 1-based line.
    pub line: usize,
}

/// A function defined in the buffer, with its declared parameter count.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FnDef {
    /// Function name.
    pub name: String,
    /// Declared parameters, `self` included.
    pub params: usize,
    /// Whether the first parameter is a `self` receiver (i.e. it is a method).
    pub takes_self: bool,
    /// 1-based definition line.
    pub start_line: usize,
}

/// Everything the verifier reads out of one buffer.
#[derive(Debug, Clone, Default)]
pub struct BufferFacts {
    /// Functions defined in the buffer.
    pub functions: Vec<FnDef>,
    /// Call sites in the buffer.
    pub calls: Vec<Call>,
    /// Names of bodiless `mod foo;` declarations — these must resolve to a
    /// sibling file, which is checkable without a compiler.
    pub file_modules: Vec<String>,
    /// Every name brought into scope by a `use`, bound by a `let`, or declared
    /// as a parameter. A callee in this set is *not* an unknown identifier —
    /// it is an import, a local closure, or a function-typed argument.
    pub bound_names: BTreeSet<String>,
}

/// Parse `source` as Rust and collect the facts verification needs.
pub fn analyze(source: &str) -> Result<BufferFacts> {
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_rust::LANGUAGE.into())
        .map_err(|e| Error::Parse(e.to_string()))?;
    let tree = parser
        .parse(source, None)
        .ok_or_else(|| Error::Parse("tree-sitter produced no tree".to_string()))?;

    let bytes = source.as_bytes();
    let mut facts = BufferFacts::default();
    let mut stack = vec![tree.root_node()];

    while let Some(node) = stack.pop() {
        match node.kind() {
            "function_item" => collect_function(node, bytes, &mut facts),
            "call_expression" => collect_call(node, bytes, &mut facts),
            "use_declaration" => collect_bound_idents(node, bytes, &mut facts.bound_names),
            // A `let` binding (including `let f = |..| ..`) puts a name in scope.
            "let_declaration" => {
                if let Some(pattern) = node.child_by_field_name("pattern") {
                    collect_bound_idents(pattern, bytes, &mut facts.bound_names);
                }
            }
            "mod_item" if node.child_by_field_name("body").is_none() => {
                if let Some(name) = node
                    .child_by_field_name("name")
                    .and_then(|n| n.utf8_text(bytes).ok())
                {
                    facts.file_modules.push(name.to_string());
                }
            }
            _ => {}
        }

        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            stack.push(child);
        }
    }

    Ok(facts)
}

/// Record a function definition and bind its parameter names.
fn collect_function(node: Node, bytes: &[u8], facts: &mut BufferFacts) {
    let Some(name) = node
        .child_by_field_name("name")
        .and_then(|n| n.utf8_text(bytes).ok())
    else {
        return;
    };
    let Some(params) = node.child_by_field_name("parameters") else {
        return;
    };

    let mut count = 0usize;
    let mut takes_self = false;
    let mut cursor = params.walk();
    for child in params.children(&mut cursor) {
        match child.kind() {
            "parameter" => {
                count += 1;
                if let Some(pattern) = child.child_by_field_name("pattern") {
                    collect_bound_idents(pattern, bytes, &mut facts.bound_names);
                }
            }
            "self_parameter" => {
                count += 1;
                takes_self = true;
            }
            // `(` `)` `,` and friends.
            _ => {}
        }
    }

    facts.functions.push(FnDef {
        name: name.to_string(),
        params: count,
        takes_self,
        start_line: node.start_position().row + 1,
    });
}

/// Record a call site, classified by how it was written.
fn collect_call(node: Node, bytes: &[u8], facts: &mut BufferFacts) {
    let Some(func) = node.child_by_field_name("function") else {
        return;
    };
    let (name, form) = match func.kind() {
        "identifier" => (func.utf8_text(bytes).ok(), CallForm::Free),
        "scoped_identifier" => (
            func.child_by_field_name("name")
                .and_then(|n| n.utf8_text(bytes).ok()),
            CallForm::Path,
        ),
        "field_expression" => (
            func.child_by_field_name("field")
                .and_then(|n| n.utf8_text(bytes).ok()),
            CallForm::Method,
        ),
        // Generic calls (`foo::<T>(..)`) and anything exotic: not checkable.
        _ => (None, CallForm::Path),
    };
    let Some(name) = name else { return };

    let arity = node.child_by_field_name("arguments").map_or(0, |args| {
        let mut cursor = args.walk();
        args.children(&mut cursor)
            .filter(|c| c.is_named() && c.kind() != "line_comment" && c.kind() != "block_comment")
            .count()
    });

    facts.calls.push(Call {
        name: name.to_string(),
        form,
        arity,
        line: node.start_position().row + 1,
    });
}

/// Add every `identifier` under `node` to `out`.
fn collect_bound_idents(node: Node, bytes: &[u8], out: &mut BTreeSet<String>) {
    let mut stack = vec![node];
    while let Some(n) = stack.pop() {
        if n.kind() == "identifier" {
            if let Ok(text) = n.utf8_text(bytes) {
                out.insert(text.to_string());
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

    fn call<'a>(facts: &'a BufferFacts, name: &str) -> &'a Call {
        facts.calls.iter().find(|c| c.name == name).unwrap()
    }

    #[test]
    fn records_arity_and_form_per_call() {
        let facts = analyze("fn f() { free(1, 2); a::b::path(1); x.method(); }").unwrap();
        assert_eq!(call(&facts, "free").form, CallForm::Free);
        assert_eq!(call(&facts, "free").arity, 2);
        assert_eq!(call(&facts, "path").form, CallForm::Path);
        assert_eq!(call(&facts, "method").form, CallForm::Method);
        assert_eq!(call(&facts, "method").arity, 0);
    }

    #[test]
    fn records_parameter_counts_and_self_receivers() {
        let facts =
            analyze("fn free(a: i32, b: i32) {}\nstruct S;\nimpl S { fn m(&self, x: u8) {} }")
                .unwrap();
        let free = facts.functions.iter().find(|f| f.name == "free").unwrap();
        assert_eq!(free.params, 2);
        assert!(!free.takes_self);
        let method = facts.functions.iter().find(|f| f.name == "m").unwrap();
        // `self` counts, which is exactly why methods are not arity-checked.
        assert_eq!(method.params, 2);
        assert!(method.takes_self);
    }

    #[test]
    fn imports_lets_and_params_all_count_as_bound() {
        let facts =
            analyze("use std::mem::swap;\nfn f(callback: fn()) { let helper = || {}; helper(); }")
                .unwrap();
        // Each of these would otherwise look like an undefined identifier.
        assert!(facts.bound_names.contains("swap"));
        assert!(facts.bound_names.contains("helper"));
        assert!(facts.bound_names.contains("callback"));
    }

    #[test]
    fn distinguishes_file_modules_from_inline_ones() {
        let facts = analyze("mod sibling;\nmod inline { fn g() {} }").unwrap();
        // Only the bodiless form must resolve to a file on disk.
        assert_eq!(facts.file_modules, vec!["sibling".to_string()]);
    }

    #[test]
    fn a_syntactically_broken_buffer_still_parses() {
        // The point of the tree-sitter tier: it never fails on a mid-edit
        // buffer. What it *recovers* varies — a call truncated mid-argument may
        // not form a `call_expression` at all — so the guarantee tested here is
        // "returns facts rather than an error", not "recovers everything".
        for broken in [
            "fn f() { broken(1, ",
            "fn f() { good(1); broken(1, ",
            "fn f( { }",
            "",
        ] {
            assert!(analyze(broken).is_ok(), "failed on {broken:?}");
        }
        // Recovery is real where the syntax closes: the complete call survives
        // even though a later one is truncated.
        let facts = analyze("fn f() { good(1); broken(1, ").unwrap();
        assert_eq!(call(&facts, "good").arity, 1);
    }
}
