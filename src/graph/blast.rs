//! The FR-12 reachability primitive, as a single BFS over an [`Adjacency`].
//!
//! "Build it once": the same walk serves the base [`CodeGraph`] and — in
//! Phase 3 — the composed per-tenant view and `overlay::update_frontier`'s
//! frontier seeding. Each backing graph implements [`Adjacency`]; the walk in
//! [`reachable_over`] is the only breadth-first traversal in the crate.

use std::collections::HashSet;
use std::hash::Hash;

use petgraph::graph::NodeIndex;
use petgraph::Direction as PgDirection;

use super::CodeGraph;

/// Direction of a reachability walk over the call graph.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dir {
    /// Transitive callers — "what does changing this affect?" (impact).
    Callers,
    /// Transitive callees — "what does this depend on?".
    Callees,
}

impl Dir {
    /// The `via` label for an edge traversed in this direction.
    fn via(self) -> &'static str {
        match self {
            Dir::Callees => "calls",
            Dir::Callers => "called_by",
        }
    }
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

/// A symbol's identity facts, independent of any particular walk.
pub struct NodeMeta {
    /// Symbol name.
    pub name: String,
    /// File (relative to the build root).
    pub file: String,
    /// 1-based definition line.
    pub start_line: usize,
}

/// A directed symbol graph the [`reachable_over`] BFS can walk. Implemented by
/// the base [`CodeGraph`] and — in Phase 3 — the composed per-tenant view, so
/// serve-path impact and `update_frontier` share one traversal (FR-12).
pub trait Adjacency {
    /// A node handle in this backing graph.
    type Node: Copy + Eq + Hash;
    /// The nodes defining `name` — the walk seeds.
    fn seeds(&self, name: &str) -> Vec<Self::Node>;
    /// The neighbors of `node` in direction `dir`.
    fn neighbors(&self, node: Self::Node, dir: Dir) -> Vec<Self::Node>;
    /// The identity facts of `node`.
    fn meta(&self, node: Self::Node) -> NodeMeta;
}

/// Symbols reachable from `seed` within `max_hops`, breadth-first — the FR-12
/// primitive over any [`Adjacency`]. `Dir::Callers` is the impact set (who
/// transitively calls `seed`); `Dir::Callees` is the dependency set.
#[must_use]
pub fn reachable_over<A: Adjacency>(adj: &A, seed: &str, dir: Dir, max_hops: u32) -> Vec<Reached> {
    let seeds = adj.seeds(seed);
    if seeds.is_empty() {
        return Vec::new();
    }
    let via = dir.via();
    let mut visited: HashSet<A::Node> = seeds.iter().copied().collect();
    let mut frontier = seeds;
    let mut reached = Vec::new();
    let mut hop = 0;

    while hop < max_hops && !frontier.is_empty() {
        hop += 1;
        let mut next = Vec::new();
        for node in frontier {
            for neighbor in adj.neighbors(node, dir) {
                if visited.insert(neighbor) {
                    let m = adj.meta(neighbor);
                    reached.push(Reached {
                        name: m.name,
                        file: m.file,
                        start_line: m.start_line,
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

/// The base graph walks by petgraph node index.
impl Adjacency for CodeGraph {
    type Node = NodeIndex;

    fn seeds(&self, name: &str) -> Vec<NodeIndex> {
        self.by_name.get(name).cloned().unwrap_or_default()
    }

    fn neighbors(&self, node: NodeIndex, dir: Dir) -> Vec<NodeIndex> {
        let pg_dir = match dir {
            Dir::Callees => PgDirection::Outgoing,
            Dir::Callers => PgDirection::Incoming,
        };
        self.graph.neighbors_directed(node, pg_dir).collect()
    }

    fn meta(&self, node: NodeIndex) -> NodeMeta {
        let symbol = &self.graph[node];
        NodeMeta {
            name: symbol.name.clone(),
            file: symbol.file.clone(),
            start_line: symbol.start_line,
        }
    }
}
