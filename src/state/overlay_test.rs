//! Tests for the copy-on-write overlay and the composed view.
#![allow(non_snake_case)]

use super::*;
use crate::state::graph::AttrValue;

fn board() -> StateGraph {
    let mut g = StateGraph::new();
    g.upsert_node(StateNode::new("alpha", "BaseState").with("garrison", AttrValue::Num(2.0)));
    g.upsert_node(StateNode::new("beta", "BaseState").with("garrison", AttrValue::Num(1.0)));
    g.upsert_node(StateNode::new("scout", "UnitState"));
    g.insert_edge(StateEdge::new("alpha", "adjacent_to", "beta"));
    g.insert_edge(StateEdge::new("scout", "garrisoned_at", "alpha"));
    g
}

#[test]
fn an_empty_overlay_IS_the_base() {
    let base = board();
    let overlay = StateOverlay::new();
    assert!(overlay.is_empty());
    let view = StateView::new(&base, Some(&overlay));
    assert_eq!(view.stats(), base.stats());
    assert_eq!(view.stats(), StateView::new(&base, None).stats());
}

#[test]
fn an_overlay_masks_the_base_for_ITS_tenant_only() {
    let base = board();
    let mut mine = StateOverlay::new();
    mine.upsert_node(StateNode::new("alpha", "BaseState").with("garrison", AttrValue::Num(9.0)));

    let theirs = StateOverlay::new();
    assert_eq!(
        StateView::new(&base, Some(&mine))
            .node("alpha")
            .unwrap()
            .attrs
            .get("garrison"),
        Some(&AttrValue::Num(9.0))
    );
    assert_eq!(
        StateView::new(&base, Some(&theirs))
            .node("alpha")
            .unwrap()
            .attrs
            .get("garrison"),
        Some(&AttrValue::Num(2.0)),
        "a sibling still sees the base — the overlay is not a mutation of it"
    );
}

#[test]
fn a_tombstoned_entity_is_ABSENT_not_merely_unlisted() {
    // Absent through every read path: `node`, `nodes`, and — the one that is
    // easy to miss — the edges that used to reach it.
    let base = board();
    let mut overlay = StateOverlay::new();
    overlay.remove_node("beta", &base);

    let view = StateView::new(&base, Some(&overlay));
    assert!(view.node("beta").is_none());
    assert!(!view.nodes().iter().any(|n| n.id == "beta"));
    assert!(
        !view.edges().iter().any(|e| e.target == "beta"),
        "an edge into a tombstoned entity must not survive: {:?}",
        view.edges()
    );
    assert!(
        view.neighbors("alpha").iter().all(|(id, _)| id != "beta"),
        "and it must not be reachable as a neighbour either"
    );
}

#[test]
fn an_overlay_edge_to_an_entity_the_overlay_tombstoned_is_not_visible() {
    // The endpoint filter is what stops a stale add from resurrecting a removal.
    let base = board();
    let mut overlay = StateOverlay::new();
    overlay.insert_edge(StateEdge::new("scout", "garrisoned_at", "beta"));
    overlay.remove_node("beta", &base);
    let view = StateView::new(&base, Some(&overlay));
    assert!(!view.edges().iter().any(|e| e.target == "beta"));
}

#[test]
fn masking_an_edge_leaves_its_endpoints_alone() {
    let base = board();
    let mut overlay = StateOverlay::new();
    overlay.remove_edge(&(
        "scout".to_string(),
        "garrisoned_at".to_string(),
        "alpha".to_string(),
    ));
    let view = StateView::new(&base, Some(&overlay));
    assert!(view.node("scout").is_some());
    assert!(view.node("alpha").is_some());
    assert_eq!(view.stats(), (3, 1));
}

#[test]
fn re_adding_a_masked_edge_unmasks_it() {
    let base = board();
    let key = (
        "alpha".to_string(),
        "adjacent_to".to_string(),
        "beta".to_string(),
    );
    let mut overlay = StateOverlay::new();
    overlay.remove_edge(&key);
    overlay.insert_edge(StateEdge::new("alpha", "adjacent_to", "beta"));
    assert_eq!(StateView::new(&base, Some(&overlay)).stats().1, 2);
}

#[test]
fn overlay_cost_is_O_delta_not_board_sized() {
    // The COW model's whole claim. A tenant that touched one entity holds one.
    let base = board();
    let mut overlay = StateOverlay::new();
    overlay.upsert_node(StateNode::new("alpha", "BaseState"));
    assert_eq!(overlay.stats(), (1, 0));
    assert_eq!(base.stats().0, 3, "while the base still holds three");
}

#[test]
fn neighbors_walk_edges_in_BOTH_directions() {
    // Traversal is undirected even though pattern matching is not: an impact
    // walk that only followed outgoing edges would miss everything upstream of
    // the change, which is the half a blast radius is mostly about.
    let base = board();
    let view = StateView::new(&base, None);
    let mut reached: Vec<String> = view
        .neighbors("alpha")
        .into_iter()
        .map(|(id, _)| id)
        .collect();
    reached.sort();
    assert_eq!(reached, vec!["beta".to_string(), "scout".to_string()]);
}
