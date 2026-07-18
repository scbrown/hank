//! Export the referential structure as RDF Turtle in the `bobbin:` code ontology.
//!
//! This is the governed projection of Hank's live graph — the substrate under
//! Phase-4 promotion (`--to quipu`). It emits *precise, typed referential
//! structure* (modules, symbols, `definedIn` / `calls` / `imports` edges),
//! **not** the embedding-oriented chunking Bobbin produces. Facts validate
//! against Quipu's
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
    // module IRI → its import-name references; and module stem → module IRI(s),
    // for resolving `use`/`mod` references to sibling modules.
    let mut raw_imports: Vec<(String, Vec<String>)> = Vec::new();
    let mut by_stem: HashMap<String, Vec<String>> = HashMap::new();

    for file in rust_files(root) {
        let Ok(source) = std::fs::read_to_string(&file) else {
            continue;
        };
        let Ok(structure) = extract_structure(&source, "rust") else {
            continue;
        };
        let rel = rel_path(&file, root);
        let module = module_iri(repo, &rel);
        modules.push((module.clone(), rel.clone()));
        by_stem
            .entry(module_stem(&rel))
            .or_default()
            .push(module.clone());
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
        raw_imports.push((module.clone(), structure.import_refs.clone()));
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

    // Resolve each import reference to a sibling module by matching its stem.
    let mut import_edges: BTreeSet<(String, String)> = BTreeSet::new();
    for (from, refs) in raw_imports {
        for name in refs {
            if let Some(targets) = by_stem.get(&name) {
                for to in targets {
                    if *to != from {
                        import_edges.insert((from.clone(), to.clone()));
                    }
                }
            }
        }
    }

    Ok(render(repo, &modules, &symbols, &call_edges, &import_edges))
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
    import_edges: &BTreeSet<(String, String)>,
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

    if !import_edges.is_empty() {
        out.push('\n');
        for (from, to) in import_edges {
            out.push_str(&format!("<{from}> bobbin:imports <{to}> .\n"));
        }
    }
    out
}

/// The module-name stem used to resolve `use`/`mod` references to a module: the
/// file stem, except a `mod.rs` takes its parent directory's name (Rust's
/// directory-module convention). Root modules (`lib`/`main`) keep their stem;
/// they are valid import *sources* but rarely import targets.
fn module_stem(rel: &str) -> String {
    let path = Path::new(rel);
    let stem = path
        .file_stem()
        .map_or_else(String::new, |s| s.to_string_lossy().into_owned());
    if stem == "mod" {
        return path
            .parent()
            .and_then(Path::file_name)
            .map_or(stem, |n| n.to_string_lossy().into_owned());
    }
    stem
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

    #[test]
    fn emits_import_edges_between_modules() {
        let dir = tempfile::tempdir().unwrap();
        // `consumer` imports from the `helper` module by `use`.
        std::fs::write(dir.path().join("helper.rs"), "pub fn thing() {}\n").unwrap();
        std::fs::write(
            dir.path().join("consumer.rs"),
            "use crate::helper::thing;\nfn run() { thing(); }\n",
        )
        .unwrap();

        let ttl = to_turtle(dir.path(), "demo").unwrap();
        assert!(
            ttl.contains("bobbin:imports"),
            "expected an imports edge, got:\n{ttl}"
        );
        // Edge points consumer → helper (the module IRI ends in the file path).
        let consumer = "consumer.rs";
        let helper = "helper.rs";
        let edge_line = ttl
            .lines()
            .find(|l| l.contains("bobbin:imports"))
            .unwrap_or_default();
        assert!(edge_line.contains(consumer), "from should be consumer");
        assert!(edge_line.contains(helper), "to should be helper");
    }

    #[test]
    fn mod_rs_resolves_by_directory_name() {
        assert_eq!(module_stem("mcp/mod.rs"), "mcp");
        assert_eq!(module_stem("graph.rs"), "graph");
        assert_eq!(module_stem("src/graph.rs"), "graph");
    }
}
