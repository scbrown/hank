//! Copy-on-write overlay over a [`StateGraph`], and the composed read view.
//!
//! The same shape as [`crate::graph::Overlay`] for code, applied to board facts:
//! the base is shared and immutable, each tenant owns a small delta, and a
//! [`StateView`] composes exactly one of them for the duration of one query.
//!
//! A speculative order-set is applied to a *clone* of the tenant's overlay
//! ([`super::orders`]), so what-if and guard evaluation never touch the tenant's
//! own state — "without committing" (FR-38) is a property of the call graph
//! here, not a discipline the caller has to remember.
//!
//! ## Tombstones, not deletions
//!
//! An overlay cannot delete from the base — the base is shared. Removing a node
//! records `None` against its id, and the view treats that as absent. This is
//! what makes an order that destroys a unit expressible without every tenant
//! seeing it disappear.

use std::collections::{BTreeMap, BTreeSet};

use super::graph::{EdgeKey, StateEdge, StateGraph, StateNode};

/// One tenant's copy-on-write delta over the shared base.
#[derive(Debug, Clone, Default)]
pub struct StateOverlay {
    /// Node id → the overlay's truth for it. `None` is a tombstone.
    nodes: BTreeMap<String, Option<StateNode>>,
    /// Edges this overlay adds.
    edges: BTreeMap<EdgeKey, StateEdge>,
    /// Base edges this overlay masks.
    removed_edges: BTreeSet<EdgeKey>,
}

impl StateOverlay {
    /// An empty overlay — one that composes to the bare base.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether this overlay states nothing (its view IS the base).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty() && self.edges.is_empty() && self.removed_edges.is_empty()
    }

    /// How many nodes and edges this overlay states — `O(delta)`, never
    /// board-sized. Reported by status so an operator can see the COW model is
    /// actually doing its job rather than each tenant holding a whole board.
    #[must_use]
    pub fn stats(&self) -> (usize, usize) {
        (
            self.nodes.len(),
            self.edges.len() + self.removed_edges.len(),
        )
    }

    /// State a node as this overlay's truth.
    pub fn upsert_node(&mut self, node: StateNode) {
        self.nodes.insert(node.id.clone(), Some(node));
    }

    /// Mask a node: the view sees it as absent, and so are its incident edges.
    ///
    /// Base edges touching it are masked here rather than filtered at read time
    /// so `stats()` reports the real cost of the removal, and so the view's edge
    /// walk stays a single set difference.
    pub fn remove_node(&mut self, id: &str, base: &StateGraph) {
        self.nodes.insert(id.to_string(), None);
        self.edges.retain(|(s, _, t), _| s != id && t != id);
        for edge in base.edges() {
            if edge.source == id || edge.target == id {
                self.removed_edges.insert(edge.key());
            }
        }
    }

    /// State an edge.
    pub fn insert_edge(&mut self, edge: StateEdge) {
        let key = edge.key();
        self.removed_edges.remove(&key);
        self.edges.insert(key, edge);
    }

    /// Mask an edge.
    pub fn remove_edge(&mut self, key: &EdgeKey) {
        self.edges.remove(key);
        self.removed_edges.insert(key.clone());
    }

    /// Node ids this overlay has touched (stated or masked) — the seed set for
    /// an FR-38 impact walk.
    pub fn touched_nodes(&self) -> impl Iterator<Item = &str> {
        self.nodes.keys().map(String::as_str)
    }
}

/// The composed `base + one overlay` read view — built per query, dropped at its
/// end, borrowing rather than owning.
///
/// **Isolation is structural (FR-39).** A view holds exactly ONE overlay
/// reference, chosen by the registry from the tenant key. There is no
/// constructor, method, or field through which a second tenant's overlay could
/// be reached, so a cross-tenant read is not "prevented" by a check that could
/// be forgotten — it is unrepresentable.
#[derive(Debug, Clone, Copy)]
pub struct StateView<'a> {
    base: &'a StateGraph,
    overlay: Option<&'a StateOverlay>,
}

impl<'a> StateView<'a> {
    /// Compose `base` with `overlay` (or with nothing, which views the bare base).
    #[must_use]
    pub fn new(base: &'a StateGraph, overlay: Option<&'a StateOverlay>) -> Self {
        Self { base, overlay }
    }

    /// A node by id: the overlay's truth if it states one, the base's otherwise.
    /// `None` for a tombstoned id, exactly as for an id nothing ever stated.
    #[must_use]
    pub fn node(&self, id: &str) -> Option<&'a StateNode> {
        match self.overlay.and_then(|o| o.nodes.get(id)) {
            Some(Some(node)) => Some(node),
            Some(None) => None,
            None => self.base.node(id),
        }
    }

    /// Every visible node, in id order.
    #[must_use]
    pub fn nodes(&self) -> Vec<&'a StateNode> {
        let mut seen: BTreeMap<&str, &StateNode> = BTreeMap::new();
        for node in self.base.nodes() {
            seen.insert(node.id.as_str(), node);
        }
        if let Some(overlay) = self.overlay {
            for (id, slot) in &overlay.nodes {
                match slot {
                    Some(node) => {
                        seen.insert(id.as_str(), node);
                    }
                    None => {
                        seen.remove(id.as_str());
                    }
                }
            }
        }
        seen.into_values().collect()
    }

    /// Every visible edge, in identity order. An edge whose endpoints are not
    /// both visible is dropped: a tombstoned node must not remain reachable
    /// through an edge the overlay never saw.
    #[must_use]
    pub fn edges(&self) -> Vec<&'a StateEdge> {
        let mut out: BTreeMap<&EdgeKey, &StateEdge> = BTreeMap::new();
        for (key, edge) in self.base.edge_index() {
            out.insert(key, edge);
        }
        if let Some(overlay) = self.overlay {
            for key in &overlay.removed_edges {
                out.remove(key);
            }
            for (key, edge) in &overlay.edges {
                out.insert(key, edge);
            }
        }
        out.into_values()
            .filter(|e| self.node(&e.source).is_some() && self.node(&e.target).is_some())
            .collect()
    }

    /// Nodes one hop from `id`, following edges in both directions, paired with
    /// the relation that reached them.
    #[must_use]
    pub fn neighbors(&self, id: &str) -> Vec<(String, &'a str)> {
        self.edges()
            .into_iter()
            .filter_map(|e| {
                if e.source == id {
                    Some((e.target.clone(), e.relation.as_str()))
                } else if e.target == id {
                    Some((e.source.clone(), e.relation.as_str()))
                } else {
                    None
                }
            })
            .collect()
    }

    /// Node and edge counts of the composed view.
    #[must_use]
    pub fn stats(&self) -> (usize, usize) {
        (self.nodes().len(), self.edges().len())
    }

    /// Whether the view holds nothing — the guard's refusal condition. See
    /// [`StateGraph::is_empty`].
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.nodes().is_empty()
    }
}

#[cfg(test)]
#[path = "overlay_test.rs"]
mod overlay_test;
