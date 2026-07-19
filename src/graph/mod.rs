//! The in-memory call graph and the blast-radius reachability primitive.
//!
//! Phase 2 builds a symbol-level call graph over a subtree and answers
//! reachability in either direction. This is the FR-12 primitive: the same
//! traversal answers "what does this change affect?" (callers, transitively)
//! and "what does this call?" (callees). Phase 3 makes this a hot, per-tenant
//! resident graph (see [`base`]/[`overlay`]/[`tenant`]); today it is built on
//! demand.
//!
//! The single breadth-first traversal lives in [`blast`] behind the [`Adjacency`]
//! trait, so the base graph, the composed per-tenant view, and the frontier
//! update all share one implementation (FR-12, "build it once").

mod blast;
mod community;

use std::collections::HashMap;
use std::path::Path;

use petgraph::graph::{DiGraph, NodeIndex};

use crate::errors::Result;
use crate::extract::{extract_structure, rust_files};
use crate::types::{EdgeKind, Tier};

pub use blast::{reachable_over, Adjacency, Dir, NodeMeta, Reached};
pub use community::{Community, CommunityMember};

/// A node in the call graph: one defined symbol.
#[derive(Debug, Clone)]
pub struct SymbolNode {
    /// Symbol name.
    pub name: String,
    /// Symbol kind (lowercase form).
    pub kind: String,
    /// File the symbol is defined in (relative to the build root).
    pub file: String,
    /// 1-based definition line.
    pub start_line: usize,
    /// Provenance tier.
    pub tier: Tier,
}

/// A symbol-level call graph built over a subtree.
pub struct CodeGraph {
    graph: DiGraph<SymbolNode, EdgeKind>,
    by_name: HashMap<String, Vec<NodeIndex>>,
}

impl CodeGraph {
    /// Build the call graph for the Rust files under `root`.
    ///
    /// Call edges are resolved by name (best-effort): a call to `foo` links to
    /// every symbol named `foo`. Precise resolution arrives with the LSP/CPG
    /// tiers.
    pub fn build(root: &Path) -> Result<Self> {
        let sources = rust_files(root).into_iter().filter_map(|file| {
            let source = std::fs::read_to_string(&file).ok()?;
            // Relative to the build root; fall back to the file name when the
            // root *is* the file (strip yields an empty path).
            let rel = match file.strip_prefix(root) {
                Ok(p) if !p.as_os_str().is_empty() => p.display().to_string(),
                _ => file.file_name().map_or_else(
                    || file.display().to_string(),
                    |n| n.to_string_lossy().into_owned(),
                ),
            };
            Some((rel, source))
        });
        Ok(Self::from_sources(sources))
    }

    /// Build the call graph from the tree content at a git `reference` — the
    /// shared read-only base at a baseline commit (FR-13/§5.5), not the working
    /// tree. Paths are repo-root-relative. Outside a repo, or for an unresolved
    /// ref, the tree is empty and so is the graph (degrade, never fail).
    pub fn build_at_ref(root: &Path, reference: &str) -> Result<Self> {
        let sources = crate::git::list_files_at(root, reference)
            .into_iter()
            .filter(|p| p.extension().is_some_and(|e| e == "rs"))
            .filter_map(|path| {
                let source = crate::git::read_blob_at(root, reference, &path)?;
                Some((path.display().to_string(), source))
            });
        Ok(Self::from_sources(sources))
    }

    /// Shared construction: build symbol nodes and name-resolved call edges from
    /// a stream of `(relative_path, source)` pairs. The two builders differ only
    /// in where the sources come from (working tree vs. a git tree).
    fn from_sources(sources: impl Iterator<Item = (String, String)>) -> Self {
        let mut graph = DiGraph::new();
        let mut by_name: HashMap<String, Vec<NodeIndex>> = HashMap::new();
        let mut calls: Vec<(String, String)> = Vec::new();

        for (rel, source) in sources {
            let Ok(structure) = extract_structure(&source, "rust") else {
                continue;
            };
            for symbol in structure.symbols {
                let idx = graph.add_node(SymbolNode {
                    name: symbol.name.clone(),
                    kind: symbol.kind.as_str().to_string(),
                    file: rel.clone(),
                    start_line: symbol.start_line,
                    tier: symbol.tier,
                });
                by_name.entry(symbol.name).or_default().push(idx);
            }
            for call in structure.calls {
                calls.push((call.caller, call.callee));
            }
        }

        for (caller, callee) in calls {
            let (Some(callers), Some(callees)) = (by_name.get(&caller), by_name.get(&callee))
            else {
                continue;
            };
            for &from in callers {
                for &to in callees {
                    if from != to {
                        graph.add_edge(from, to, EdgeKind::Calls);
                    }
                }
            }
        }

        Self { graph, by_name }
    }

    /// Whether any symbol with `name` is in the graph.
    #[must_use]
    pub fn has_symbol(&self, name: &str) -> bool {
        self.by_name.contains_key(name)
    }

    /// Node and edge counts.
    #[must_use]
    pub fn stats(&self) -> (usize, usize) {
        (self.graph.node_count(), self.graph.edge_count())
    }

    /// Direct (1-hop) neighbors of `seed` in the given direction.
    #[must_use]
    pub fn direct(&self, seed: &str, dir: Dir) -> Vec<Reached> {
        self.reachable(seed, dir, 1)
    }

    /// Symbols reachable from `seed` within `max_hops`, breadth-first.
    ///
    /// This is the FR-12 primitive (via the single [`blast::reachable_over`]
    /// walk): `Dir::Callers` is the impact set (who transitively calls `seed`);
    /// `Dir::Callees` is the dependency set.
    #[must_use]
    pub fn reachable(&self, seed: &str, dir: Dir, max_hops: u32) -> Vec<Reached> {
        reachable_over(self, seed, dir, max_hops)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(dir: &Path, name: &str, body: &str) {
        std::fs::write(dir.join(name), body).unwrap();
    }

    #[test]
    fn builds_and_walks_call_graph() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "lib.rs",
            "\
fn leaf() {}
fn mid() { leaf(); }
fn top() { mid(); }
",
        );
        let graph = CodeGraph::build(dir.path()).unwrap();
        assert!(graph.has_symbol("leaf"));

        // Direct callers of leaf: mid.
        let callers = graph.direct("leaf", Dir::Callers);
        assert_eq!(callers.len(), 1);
        assert_eq!(callers[0].name, "mid");

        // Transitive impact of leaf: mid (1) and top (2).
        let impact = graph.reachable("leaf", Dir::Callers, 5);
        let names: Vec<&str> = impact.iter().map(|r| r.name.as_str()).collect();
        assert!(names.contains(&"mid"));
        assert!(names.contains(&"top"));

        // Callees of top: mid.
        let callees = graph.direct("top", Dir::Callees);
        assert_eq!(callees[0].name, "mid");
    }

    #[test]
    fn unknown_symbol_is_empty() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "a.rs", "fn a() {}");
        let graph = CodeGraph::build(dir.path()).unwrap();
        assert!(graph.reachable("nope", Dir::Callers, 3).is_empty());
    }

    #[test]
    fn build_at_ref_reads_historical_tree_not_working_copy() {
        let dir = tempfile::tempdir().unwrap();
        let git = |args: &[&str]| {
            std::process::Command::new("git")
                .arg("-C")
                .arg(dir.path())
                .args(args)
                .output()
        };
        if !git(&["init", "-q"]).is_ok_and(|o| o.status.success()) {
            return; // skip: no git available
        }
        let _ = git(&["config", "user.email", "t@t.test"]);
        let _ = git(&["config", "user.name", "t"]);

        // First commit: only `old_fn` exists.
        write(dir.path(), "lib.rs", "fn old_fn() {}\n");
        let _ = git(&["add", "."]);
        let _ = git(&["commit", "-q", "-m", "first"]);
        let first = crate::git::head_commit(dir.path()).unwrap();

        // Working tree diverges: `old_fn` is gone, `new_fn` appears (uncommitted).
        write(dir.path(), "lib.rs", "fn new_fn() {}\n");

        // The ref build sees the committed past; the working-tree build sees now.
        let at_ref = CodeGraph::build_at_ref(dir.path(), &first).unwrap();
        assert!(
            at_ref.has_symbol("old_fn"),
            "ref graph has the historical symbol"
        );
        assert!(
            !at_ref.has_symbol("new_fn"),
            "ref graph ignores the working tree"
        );

        let working = CodeGraph::build(dir.path()).unwrap();
        assert!(working.has_symbol("new_fn"));
        assert!(!working.has_symbol("old_fn"));
    }

    #[test]
    fn build_at_ref_outside_repo_is_empty() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "a.rs", "fn a() {}");
        let graph = CodeGraph::build_at_ref(dir.path(), "HEAD").unwrap();
        assert_eq!(
            graph.stats(),
            (0, 0),
            "no repo → empty base graph, no crash"
        );
    }
}
