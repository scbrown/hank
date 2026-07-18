//! The in-memory call graph and the blast-radius reachability primitive.
//!
//! Phase 2 builds a symbol-level call graph over a subtree and answers
//! reachability in either direction. This is the FR-12 primitive: the same
//! traversal answers "what does this change affect?" (callers, transitively)
//! and "what does this call?" (callees). Phase 3 will make this a hot,
//! per-tenant resident graph; today it is built on demand.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use petgraph::graph::{DiGraph, NodeIndex};
use petgraph::Direction as PgDirection;

use crate::errors::Result;
use crate::extract::{extract_structure, rust_files};
use crate::types::{EdgeKind, Tier};

/// Direction of a reachability walk over the call graph.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dir {
    /// Transitive callers — "what does changing this affect?" (impact).
    Callers,
    /// Transitive callees — "what does this depend on?".
    Callees,
}

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

/// A symbol reached during a walk, with its distance from the seed.
#[derive(Debug, Clone)]
pub struct Reached {
    /// Symbol name.
    pub name: String,
    /// File (relative to the build root).
    pub file: String,
    /// 1-based definition line.
    pub start_line: usize,
    /// Hop distance from the seed (1 = direct).
    pub distance: u32,
    /// Relationship to the seed (`calls` or `called_by`).
    pub via: &'static str,
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
        let mut graph = DiGraph::new();
        let mut by_name: HashMap<String, Vec<NodeIndex>> = HashMap::new();
        let mut calls: Vec<(String, String)> = Vec::new();

        for file in rust_files(root) {
            let Ok(source) = std::fs::read_to_string(&file) else {
                continue;
            };
            let Ok(structure) = extract_structure(&source, "rust") else {
                continue;
            };
            // Relative to the build root; fall back to the file name when the
            // root *is* the file (strip yields an empty path).
            let rel = match file.strip_prefix(root) {
                Ok(p) if !p.as_os_str().is_empty() => p.display().to_string(),
                _ => file.file_name().map_or_else(
                    || file.display().to_string(),
                    |n| n.to_string_lossy().into_owned(),
                ),
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

        Ok(Self { graph, by_name })
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
    /// This is the FR-12 primitive: `Dir::Callers` is the impact set (who
    /// transitively calls `seed`); `Dir::Callees` is the dependency set.
    #[must_use]
    pub fn reachable(&self, seed: &str, dir: Dir, max_hops: u32) -> Vec<Reached> {
        let Some(seeds) = self.by_name.get(seed) else {
            return Vec::new();
        };
        let pg_dir = match dir {
            Dir::Callees => PgDirection::Outgoing,
            Dir::Callers => PgDirection::Incoming,
        };
        let via = match dir {
            Dir::Callees => "calls",
            Dir::Callers => "called_by",
        };

        let mut visited: HashSet<NodeIndex> = seeds.iter().copied().collect();
        let mut frontier: Vec<NodeIndex> = seeds.clone();
        let mut reached = Vec::new();
        let mut hop = 0;

        while hop < max_hops && !frontier.is_empty() {
            hop += 1;
            let mut next = Vec::new();
            for node in frontier {
                for neighbor in self.graph.neighbors_directed(node, pg_dir) {
                    if visited.insert(neighbor) {
                        let symbol = &self.graph[neighbor];
                        reached.push(Reached {
                            name: symbol.name.clone(),
                            file: symbol.file.clone(),
                            start_line: symbol.start_line,
                            distance: hop,
                            via,
                        });
                        next.push(neighbor);
                    }
                }
            }
            frontier = next;
        }
        reached
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
}
