//! Export the referential structure as RDF Turtle in the `bobbin:` code ontology.
//!
//! This is the governed projection of Hank's live graph — the substrate under
//! Phase-4 promotion (`--to quipu`). It emits *precise, typed referential
//! structure* (modules, symbols, `definedIn` / `calls` edges), **not** the
//! embedding-oriented chunking Bobbin produces. Facts validate against Quipu's
//! `shapes/code-entities.ttl` (`CodeModule`, `CodeSymbol`), whose namespace this
//! mirrors. Document/`Section` nodes and `Section → references → CodeSymbol`
//! edges fold in as the markdown extractor lands (spec §5.10).

use std::collections::{BTreeSet, HashMap};
use std::path::Path;

use crate::errors::Result;
use crate::extract::{extract_structure, rust_files};

/// The code-ontology namespace (matches `shapes/code-entities.ttl`).
const ONTO: &str = "http://aegis.gastown.local/ontology/";

/// Emit the referential structure of the Rust files under `root` as Turtle,
/// attributing entities to repository `repo`.
pub fn to_turtle(root: &Path, repo: &str) -> Result<String> {
    let mut modules: Vec<(String, String)> = Vec::new();
    let mut symbols: Vec<SymbolTriple> = Vec::new();
    let mut by_name: HashMap<String, Vec<String>> = HashMap::new();
    let mut raw_calls: Vec<(String, String)> = Vec::new();

    for file in rust_files(root) {
        let Ok(source) = std::fs::read_to_string(&file) else {
            continue;
        };
        let Ok(structure) = extract_structure(&source, "rust") else {
            continue;
        };
        let rel = rel_path(&file, root);
        let module = module_iri(repo, &rel);
        modules.push((module.clone(), rel));
        for symbol in &structure.symbols {
            let iri = format!("{module}::{}", symbol.name);
            symbols.push(SymbolTriple {
                iri: iri.clone(),
                name: symbol.name.clone(),
                kind: symbol.kind.as_str().to_string(),
                module: module.clone(),
            });
            by_name.entry(symbol.name.clone()).or_default().push(iri);
        }
        for call in &structure.calls {
            raw_calls.push((call.caller.clone(), call.callee.clone()));
        }
    }

    let mut call_edges: BTreeSet<(String, String)> = BTreeSet::new();
    for (caller, callee) in raw_calls {
        if let (Some(from), Some(to)) = (by_name.get(&caller), by_name.get(&callee)) {
            for a in from {
                for b in to {
                    if a != b {
                        call_edges.insert((a.clone(), b.clone()));
                    }
                }
            }
        }
    }

    Ok(render(repo, &modules, &symbols, &call_edges))
}

/// A `CodeSymbol` ready to emit.
struct SymbolTriple {
    iri: String,
    name: String,
    kind: String,
    module: String,
}

/// Render the collected structure as a Turtle document.
fn render(
    repo: &str,
    modules: &[(String, String)],
    symbols: &[SymbolTriple],
    call_edges: &BTreeSet<(String, String)>,
) -> String {
    let mut out = String::new();
    out.push_str("@prefix bobbin: <http://aegis.gastown.local/ontology/> .\n");
    out.push_str("@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .\n\n");

    for (iri, rel) in modules {
        out.push_str(&format!(
            "<{iri}> a bobbin:CodeModule ;\n    bobbin:filePath \"{}\" ;\n    \
             bobbin:repo \"{}\" ;\n    bobbin:language \"rust\" .\n\n",
            esc(rel),
            esc(repo),
        ));
    }

    for symbol in symbols {
        out.push_str(&format!(
            "<{}> a bobbin:CodeSymbol ;\n    bobbin:name \"{}\" ;\n    \
             bobbin:symbolKind \"{}\" ;\n    bobbin:definedIn <{}> .\n",
            symbol.iri,
            esc(&symbol.name),
            esc(&symbol.kind),
            symbol.module,
        ));
    }

    if !call_edges.is_empty() {
        out.push('\n');
        for (from, to) in call_edges {
            out.push_str(&format!("<{from}> bobbin:calls <{to}> .\n"));
        }
    }
    out
}

/// Mint a `CodeModule` IRI: `{ONTO}code/{repo}/{path}` with `/` percent-encoded
/// in the path segment (mirrors Quipu's `namespace.rs` construction).
fn module_iri(repo: &str, rel: &str) -> String {
    format!("{ONTO}code/{repo}/{}", rel.replace('/', "%2F"))
}

/// Path relative to `root`, falling back to the file name when the root is the
/// file itself.
fn rel_path(file: &Path, root: &Path) -> String {
    match file.strip_prefix(root) {
        Ok(p) if !p.as_os_str().is_empty() => p.display().to_string(),
        _ => file.file_name().map_or_else(
            || file.display().to_string(),
            |n| n.to_string_lossy().into_owned(),
        ),
    }
}

/// Escape a Turtle string literal.
fn esc(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn emits_modules_symbols_and_calls() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.rs"), "fn leaf() {}\n").unwrap();
        std::fs::write(dir.path().join("b.rs"), "fn mid() { leaf(); }\n").unwrap();

        let ttl = to_turtle(dir.path(), "demo").unwrap();
        assert!(ttl.contains("a bobbin:CodeModule"));
        assert!(ttl.contains("a bobbin:CodeSymbol"));
        assert!(ttl.contains("bobbin:name \"leaf\""));
        assert!(ttl.contains("bobbin:symbolKind \"function\""));
        assert!(ttl.contains("bobbin:definedIn"));
        assert!(ttl.contains("bobbin:calls"));
        assert!(ttl.contains("code/demo/"));
    }

    #[test]
    fn symbol_iris_are_module_scoped() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.rs"), "fn only() {}\n").unwrap();
        let ttl = to_turtle(dir.path(), "demo").unwrap();
        assert!(ttl.contains("a.rs::only"));
    }
}
