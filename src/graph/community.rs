//! Community detection over the base graph (FR-9).
//!
//! The directed call graph is projected to an undirected weighted graph and
//! partitioned by the deterministic Louvain in [`crate::community`]; this module
//! is the [`CodeGraph`]-facing glue that materializes the resulting clusters.

use std::collections::{BTreeMap, HashMap};

use petgraph::graph::NodeIndex;

use super::CodeGraph;
use crate::types::Tier;

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

    fn write(dir: &std::path::Path, name: &str, body: &str) {
        std::fs::write(dir.join(name), body).unwrap();
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
