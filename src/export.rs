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

use crate::docref::{doc_files, extract_doc_sections, Mention};
use crate::errors::Result;
use crate::extract::{extract_structure, source_files};

/// The code-ontology namespace (matches `shapes/code-entities.ttl`).
const ONTO: &str = "http://aegis.gastown.local/ontology/";

/// Emit the referential structure of every source file this build can PARSE
/// under `root` as Turtle, attributing entities to repository `repo`.
///
/// EVERY LANGUAGE, not just Rust (the aegis-81t2 class, found again here): the
/// walk is [`source_files`] — the same drift-proof (path, language) pairing the
/// guard's graph uses — so a Python or TypeScript repo promotes its real
/// structure instead of a green empty write. Name/stem resolution is scoped
/// PER LANGUAGE: a global name map would mint cross-language call edges from
/// simple collisions (`main`, `run`, `init` exist everywhere), and a lying
/// edge in the knowledge graph is worse than a missing one.
pub fn to_turtle(root: &Path, repo: &str) -> Result<String> {
    let mut modules: Vec<(String, String, &'static str)> = Vec::new();
    let mut symbols: Vec<SymbolTriple> = Vec::new();
    let mut by_name: HashMap<String, Vec<String>> = HashMap::new();
    let mut raw_calls: Vec<(String, String)> = Vec::new();
    // module IRI → its import-name references; and module stem → module IRI(s),
    // for resolving `use`/`mod` references to sibling modules.
    let mut raw_imports: Vec<(String, Vec<String>)> = Vec::new();
    let mut by_stem: HashMap<String, Vec<String>> = HashMap::new();
    // Symbol IRI → its owning module's stem, for narrowing a `qualifier::symbol`
    // doc mention to the right module (FR-33 resolution).
    let mut stem_by_sym: HashMap<String, String> = HashMap::new();

    for (file, language) in source_files(root) {
        let Ok(source) = std::fs::read_to_string(&file) else {
            continue;
        };
        let Ok(structure) = extract_structure(&source, language) else {
            continue;
        };
        let rel = rel_path(&file, root);
        let module = module_iri(repo, &rel);
        // Language-scoped keys: `use foo` in Rust must never resolve to a
        // Python module named foo, and a call to `run` must stay within the
        // language whose parser saw it.
        let stem = format!("{language}\u{0}{}", module_stem(&rel));
        modules.push((module.clone(), rel.clone(), language));
        by_stem
            .entry(stem.clone())
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
            stem_by_sym.insert(iri.clone(), stem.clone());
            by_name
                .entry(format!("{language}\u{0}{}", symbol.name))
                .or_default()
                .push(iri);
        }
        for call in &structure.calls {
            raw_calls.push((
                format!("{language}\u{0}{}", call.caller),
                format!("{language}\u{0}{}", call.callee),
            ));
        }
        raw_imports.push((
            module.clone(),
            structure
                .import_refs
                .iter()
                .map(|r| format!("{language}\u{0}{r}"))
                .collect(),
        ));
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

    // Doc→code references (FR-33): parse each markdown file into headed
    // sections, resolve every symbol mention against the code graph built above,
    // and emit `Section --references--> CodeSymbol`. Only documents/sections that
    // resolve at least one reference are materialized — the graph is referential
    // structure, not every heading in the repo.
    let mut docs: Vec<DocTriple> = Vec::new();
    let mut sections: Vec<SectionTriple> = Vec::new();
    let mut ref_edges: BTreeSet<(String, String)> = BTreeSet::new();
    for file in doc_files(root) {
        let Ok(source) = std::fs::read_to_string(&file) else {
            continue;
        };
        let rel = rel_path(&file, root);
        let doc = document_iri(repo, &rel);
        let mut doc_used = false;
        for section in extract_doc_sections(&source) {
            let sec_iri = format!("{doc}#{}", section.slug);
            let mut sec_used = false;
            for mention in &section.mentions {
                for target in resolve(mention, &by_name, &stem_by_sym) {
                    if ref_edges.insert((sec_iri.clone(), target)) {
                        sec_used = true;
                    }
                }
            }
            if sec_used {
                sections.push(SectionTriple {
                    iri: sec_iri,
                    heading: section.heading,
                    depth: section.depth,
                    document: doc.clone(),
                });
                doc_used = true;
            }
        }
        if doc_used {
            docs.push(DocTriple {
                iri: doc,
                path: rel,
            });
        }
    }

    Ok(render(
        repo,
        &modules,
        &symbols,
        &call_edges,
        &import_edges,
        &docs,
        &sections,
        &ref_edges,
    ))
}

/// Resolve a doc [`Mention`] to concrete `CodeSymbol` IRIs, never inventing one:
/// a mention contributes edges only if its name matches an extracted symbol. A
/// `qualifier::` hint narrows the match to symbols in the module of that stem
/// when any qualify; otherwise every same-named symbol is returned (recall over
/// precision, per the "start permissive" guidance).
fn resolve(
    mention: &Mention,
    by_name: &HashMap<String, Vec<String>>,
    stem_by_sym: &HashMap<String, String>,
) -> Vec<String> {
    // Docs are PROSE: a mention has no language, so it searches ACROSS the
    // language-scoped keys (`{language}\0{name}`) that keep call/import
    // resolution honest. A doc citing `reachable` may legitimately mean the
    // Rust one or the Python one — recall over precision, as before.
    let candidates: Vec<String> = by_name
        .iter()
        .filter(|(key, _)| {
            key.rsplit_once('\u{0}')
                .map_or(key.as_str(), |(_, name)| name)
                == mention.symbol
        })
        .flat_map(|(_, iris)| iris.iter().cloned())
        .collect();
    if candidates.is_empty() {
        return Vec::new();
    }
    if let Some(qualifier) = &mention.qualifier {
        let narrowed: Vec<String> = candidates
            .iter()
            .filter(|iri| {
                stem_by_sym.get(*iri).is_some_and(|stem| {
                    stem.rsplit_once('\u{0}')
                        .map_or(stem.as_str(), |(_, bare)| bare)
                        .eq_ignore_ascii_case(qualifier)
                })
            })
            .cloned()
            .collect();
        if !narrowed.is_empty() {
            return narrowed;
        }
    }
    candidates
}

/// A `CodeSymbol` ready to emit.
struct SymbolTriple {
    iri: String,
    name: String,
    kind: String,
    module: String,
}

/// A `Document` ready to emit.
struct DocTriple {
    iri: String,
    path: String,
}

/// A `Section` ready to emit (only materialized when it has ≥1 reference).
struct SectionTriple {
    iri: String,
    heading: String,
    depth: usize,
    document: String,
}

/// Render the collected structure as a Turtle document.
#[allow(clippy::too_many_arguments)]
fn render(
    repo: &str,
    modules: &[(String, String, &'static str)],
    symbols: &[SymbolTriple],
    call_edges: &BTreeSet<(String, String)>,
    import_edges: &BTreeSet<(String, String)>,
    docs: &[DocTriple],
    sections: &[SectionTriple],
    ref_edges: &BTreeSet<(String, String)>,
) -> String {
    let mut out = String::new();
    out.push_str("@prefix bobbin: <http://aegis.gastown.local/ontology/> .\n");
    out.push_str("@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .\n\n");

    for (iri, rel, language) in modules {
        out.push_str(&format!(
            "<{iri}> a bobbin:CodeModule ;\n    bobbin:filePath \"{}\" ;\n    \
             bobbin:repo \"{}\" ;\n    bobbin:language \"{}\" .\n\n",
            esc(rel),
            esc(repo),
            esc(language),
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

    // ── Documents / Sections / references (FR-33) ──
    if !docs.is_empty() {
        out.push('\n');
        for doc in docs {
            out.push_str(&format!(
                "<{}> a bobbin:Document ;\n    bobbin:filePath \"{}\" ;\n    \
                 bobbin:repo \"{}\" .\n\n",
                doc.iri,
                esc(&doc.path),
                esc(repo),
            ));
        }
    }

    for section in sections {
        out.push_str(&format!(
            "<{}> a bobbin:Section ;\n    bobbin:heading \"{}\" ;\n    \
             bobbin:headingDepth {} ;\n    bobbin:inDocument <{}> .\n",
            section.iri,
            esc(&section.heading),
            section.depth,
            section.document,
        ));
    }

    if !ref_edges.is_empty() {
        out.push('\n');
        for (from, to) in ref_edges {
            out.push_str(&format!("<{from}> bobbin:references <{to}> .\n"));
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
    // Directory-module conventions, one per ecosystem, same shape: the file
    // that IS its directory. Rust's mod.rs, Python's __init__.py, JS/TS's
    // index.*. Each takes the parent directory's name so `import pkg` and
    // `use pkg` resolve to the module that defines pkg.
    if stem == "mod" || stem == "__init__" || stem == "index" {
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

/// Mint a `Document` IRI: `{ONTO}doc/{repo}/{path}` with `/` percent-encoded in
/// the path segment (mirrors Quipu's `document_iri`; the section anchor is
/// appended as `#{slug}` by the caller).
fn document_iri(repo: &str, rel: &str) -> String {
    format!("{ONTO}doc/{repo}/{}", rel.replace('/', "%2F"))
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

    #[test]
    fn emits_doc_references_to_known_symbols() {
        let dir = tempfile::tempdir().unwrap();
        // A code file defining two symbols the doc will reference.
        std::fs::write(
            dir.path().join("graph.rs"),
            "pub fn reachable() {}\npub struct Frontier;\n",
        )
        .unwrap();
        // A doc that references them by backtick, qualifier, and fenced code.
        std::fs::write(
            dir.path().join("guide.md"),
            "# Traversal\n\nThe `reachable` fn walks the graph; \
             see `graph::reachable` too.\n\n\
             ## Types\n\n```rust\nlet f: Frontier;\n```\n\n\
             ## Unrelated\n\nMentions `nonexistent_symbol` only.\n",
        )
        .unwrap();

        let ttl = to_turtle(dir.path(), "demo").unwrap();

        // Document + Section entities materialize for referenced sections.
        assert!(ttl.contains("a bobbin:Document"), "got:\n{ttl}");
        assert!(ttl.contains("bobbin:filePath \"guide.md\""));
        assert!(ttl.contains("a bobbin:Section"));
        assert!(ttl.contains("bobbin:heading \"Traversal\""));
        assert!(ttl.contains("bobbin:headingDepth 1"));

        // The references edge points a Section IRI at a real CodeSymbol IRI.
        assert!(
            ttl.contains("bobbin:references"),
            "no references edge:\n{ttl}"
        );
        let reachable_sym = "code/demo/graph.rs::reachable";
        let frontier_sym = "code/demo/graph.rs::Frontier";
        assert!(
            ttl.lines()
                .any(|l| l.contains("bobbin:references") && l.contains(reachable_sym)),
            "expected a references edge to reachable:\n{ttl}"
        );
        assert!(
            ttl.lines()
                .any(|l| l.contains("bobbin:references") && l.contains(frontier_sym)),
            "expected a references edge to Frontier (from fenced code):\n{ttl}"
        );

        // The Section IRI on the edge is anchored to the document.
        assert!(ttl.contains("doc/demo/guide.md#traversal"));

        // A mention that resolves to nothing is dropped — no fabricated symbol,
        // and its heading-only section is not materialized.
        assert!(
            !ttl.contains("nonexistent_symbol"),
            "fabricated symbol:\n{ttl}"
        );
        assert!(
            !ttl.contains("#unrelated"),
            "empty section materialized:\n{ttl}"
        );
    }

    #[test]
    fn qualifier_narrows_ambiguous_symbol_to_its_module() {
        let dir = tempfile::tempdir().unwrap();
        // Two modules each define a `run` symbol — an ambiguous bare name.
        std::fs::write(dir.path().join("graph.rs"), "pub fn run() {}\n").unwrap();
        std::fs::write(dir.path().join("serve.rs"), "pub fn run() {}\n").unwrap();
        // The doc qualifies the mention: `graph::run` should hit only graph's.
        std::fs::write(
            dir.path().join("doc.md"),
            "# H\n\nCall `graph::run` here.\n",
        )
        .unwrap();

        let ttl = to_turtle(dir.path(), "demo").unwrap();
        let edges: Vec<&str> = ttl
            .lines()
            .filter(|l| l.contains("bobbin:references"))
            .collect();
        assert_eq!(
            edges.len(),
            1,
            "qualifier should narrow to one edge:\n{ttl}"
        );
        assert!(edges[0].contains("code/demo/graph.rs::run"));
        assert!(!edges[0].contains("serve.rs"));
    }
}

#[cfg(test)]
mod xkwu_tests {
    //! Multi-language export (the aegis-81t2 class, found in this module): a
    //! Python repo must promote its real structure, and cross-language name
    //! collisions must never mint edges.
    use super::*;

    #[cfg(feature = "langs-extra")]
    #[test]
    fn a_python_repo_exports_real_structure_with_its_own_language() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.py"), "def leaf():\n    pass\n").unwrap();
        std::fs::write(dir.path().join("b.py"), "def mid():\n    leaf()\n").unwrap();
        let ttl = to_turtle(dir.path(), "pyrepo").unwrap();
        assert!(ttl.contains("bobbin:language \"python\""), "{ttl}");
        assert!(ttl.contains("a.py::leaf"), "{ttl}");
        assert!(
            ttl.contains("bobbin:calls"),
            "a py call edge must resolve: {ttl}"
        );
    }

    #[cfg(feature = "langs-extra")]
    #[test]
    fn cross_language_name_collisions_mint_no_edges() {
        // `main` exists in both files; a global name map would draw
        // rust-main -> py-main (or worse, both directions). Language-scoped
        // resolution draws neither.
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("x.rs"),
            "fn helper() {}\nfn main() { helper(); }\n",
        )
        .unwrap();
        std::fs::write(dir.path().join("y.py"), "def helper():\n    pass\n").unwrap();
        let ttl = to_turtle(dir.path(), "mixed").unwrap();
        // rust's call edge resolves within rust...
        assert!(ttl.contains("x.rs::main"), "{ttl}");
        // ...and no edge crosses into the python helper.
        let py_helper_called = ttl
            .lines()
            .any(|l| l.contains("bobbin:calls") && l.contains("y.py"));
        assert!(!py_helper_called, "a cross-language edge is a lie: {ttl}");
    }

    #[cfg(feature = "langs-extra")]
    #[test]
    fn python_init_takes_its_directory_name_like_mod_rs() {
        assert_eq!(module_stem("pkg/__init__.py"), "pkg");
        assert_eq!(module_stem("pkg/index.ts"), "pkg");
        assert_eq!(module_stem("pkg/mod.rs"), "pkg");
        assert_eq!(module_stem("pkg/other.py"), "other");
    }

    #[test]
    fn a_rust_only_build_still_exports_rust() {
        // The positive control for the default feature set: the walk change
        // must not have cost the original language anything.
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.rs"), "fn only() {}\n").unwrap();
        let ttl = to_turtle(dir.path(), "r").unwrap();
        assert!(ttl.contains("a.rs::only"), "{ttl}");
        assert!(ttl.contains("bobbin:language \"rust\""), "{ttl}");
    }
}
