//! The in-memory call graph and the blast-radius reachability primitive.
//!
//! Phase 2 builds a symbol-level call graph over a subtree and answers
//! reachability in either direction. This is the FR-12 primitive: the same
//! traversal answers "what does this change affect?" (callers, transitively)
//! and "what does this call?" (callees). Phase 3 will make this a hot,
//! per-tenant resident graph; today it is built on demand.

use std::collections::{BTreeMap, HashMap, HashSet};
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

/// One member symbol of a detected community.
#[derive(Debug, Clone)]
pub struct CommunityMember {
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

/// A detected community: a densely-connected cluster of symbols (FR-9).
#[derive(Debug, Clone)]
pub struct Community {
    /// Stable community id (0-based, assigned largest-cluster-first).
    pub id: usize,
    /// Member symbols, sorted by `(file, line, name)`.
    pub members: Vec<CommunityMember>,
}

impl CodeGraph {
    /// Detect communities over the graph via deterministic Louvain (FR-9).
    ///
    /// The directed call graph is projected to an undirected weighted graph
    /// (parallel/opposing edges sum), partitioned by [`crate::community::louvain`],
    /// then grouped into clusters ordered largest-first with members sorted by
    /// location — a stable, reproducible partition for a given graph.
    #[must_use]
    pub fn communities(&self) -> Vec<Community> {
        let n = self.graph.node_count();
        if n == 0 {
            return Vec::new();
        }
        // Fold the directed call graph into undirected weights.
        let mut weights: HashMap<(usize, usize), f64> = HashMap::new();
        for edge in self.graph.edge_indices() {
            let Some((a, b)) = self.graph.edge_endpoints(edge) else {
                continue;
            };
            let (a, b) = (a.index(), b.index());
            if a == b {
                continue;
            }
            let key = if a < b { (a, b) } else { (b, a) };
            *weights.entry(key).or_insert(0.0) += 1.0;
        }
        let edges: Vec<(usize, usize, f64)> =
            weights.into_iter().map(|((a, b), w)| (a, b, w)).collect();
        let labels = crate::community::louvain(n, &edges);

        // Group node indices by community label.
        let mut groups: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
        for (idx, &label) in labels.iter().enumerate() {
            groups.entry(label).or_default().push(idx);
        }
        // Materialize members, sorted by location within each community.
        let mut clusters: Vec<Vec<CommunityMember>> = groups
            .into_values()
            .map(|nodes| {
                let mut members: Vec<CommunityMember> = nodes
                    .into_iter()
                    .map(|i| {
                        let symbol = &self.graph[NodeIndex::new(i)];
                        CommunityMember {
                            name: symbol.name.clone(),
                            kind: symbol.kind.clone(),
                            file: symbol.file.clone(),
                            start_line: symbol.start_line,
                            tier: symbol.tier,
                        }
                    })
                    .collect();
                members.sort_by(|a, b| {
                    (a.file.as_str(), a.start_line, a.name.as_str()).cmp(&(
                        b.file.as_str(),
                        b.start_line,
                        b.name.as_str(),
                    ))
                });
                members
            })
            .collect();
        // Order communities largest-first, tie-broken by first member's location.
        clusters.sort_by(|a, b| {
            b.len().cmp(&a.len()).then_with(|| {
                let key = |m: &[CommunityMember]| {
                    m.first()
                        .map(|f| (f.file.clone(), f.start_line, f.name.clone()))
                };
                key(a).cmp(&key(b))
            })
        });
        clusters
            .into_iter()
            .enumerate()
            .map(|(id, members)| Community { id, members })
            .collect()
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

    #[test]
    fn communities_split_two_call_clusters() {
        // Two independent call chains that never touch → two communities.
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "lib.rs",
            "\
fn a_leaf() {}
fn a_mid() { a_leaf(); }
fn a_top() { a_mid(); a_leaf(); }
fn b_leaf() {}
fn b_mid() { b_leaf(); }
fn b_top() { b_mid(); b_leaf(); }
",
        );
        let graph = CodeGraph::build(dir.path()).unwrap();
        let comms = graph.communities();
        assert_eq!(comms.len(), 2, "expected two clusters: {comms:?}");
        // Ids are dense and largest-first (6 symbols, 3 per cluster here).
        assert_eq!(comms[0].id, 0);
        assert_eq!(comms[1].id, 1);
        // Every member of a community shares its `a_`/`b_` prefix.
        for comm in &comms {
            let prefixes: std::collections::BTreeSet<&str> =
                comm.members.iter().map(|m| &m.name[..2]).collect();
            assert_eq!(prefixes.len(), 1, "cluster mixes chains: {comm:?}");
        }
        // Deterministic across runs.
        let again = CodeGraph::build(dir.path()).unwrap().communities();
        let names = |cs: &[Community]| -> Vec<Vec<String>> {
            cs.iter()
                .map(|c| c.members.iter().map(|m| m.name.clone()).collect())
                .collect()
        };
        assert_eq!(names(&comms), names(&again));
    }
}
