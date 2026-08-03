//! Tests for the generic fact graph (FR-35).
//!
//! Test names shout the invariant they pin, as they do across this repo; the
//! lint exemption is scoped to this module.
#![allow(non_snake_case)]

use super::*;

fn base() -> StateGraph {
    let mut g = StateGraph::new();
    g.upsert_node(StateNode::new("base_alpha", "BaseState"));
    g.upsert_node(StateNode::new("base_beta", "BaseState"));
    g.insert_edge(StateEdge::new("base_alpha", "adjacent_to", "base_beta"));
    g
}

#[test]
fn a_restated_entity_replaces_rather_than_duplicating() {
    // A board is rebuilt every turn from the world view, so the same base
    // arrives again with new numbers. If that grew the graph, the turn count
    // would be the node count.
    let mut g = base();
    let displaced = g.upsert_node(
        StateNode::new("base_alpha", "BaseState").with("garrisonCount", AttrValue::Num(2.0)),
    );
    assert!(displaced.is_some(), "the previous node is handed back");
    assert_eq!(g.stats().0, 2, "still two entities, not three");
    assert_eq!(
        g.node("base_alpha").unwrap().attrs.get("garrisonCount"),
        Some(&AttrValue::Num(2.0))
    );
}

#[test]
fn a_restated_edge_is_idempotent() {
    let mut g = base();
    assert!(!g.insert_edge(StateEdge::new("base_alpha", "adjacent_to", "base_beta")));
    assert_eq!(g.stats().1, 1);
}

#[test]
fn removing_an_entity_takes_its_edges_with_it() {
    // An edge to a removed node is a dangling reference, and a pattern
    // traversing it would bind a variable to an id with nothing behind it.
    let mut g = base();
    assert!(g.remove_node("base_beta"));
    assert_eq!(g.stats(), (1, 0), "the incident edge went too");
    assert!(!g.remove_node("base_beta"), "a second removal is a no-op");
}

#[test]
fn an_ordering_comparison_against_a_non_number_has_NO_answer() {
    // Not `0.0`: a coerced zero would make every `>= 1` filter against a string
    // resolve on a fiction, in whichever direction the fiction happened to fall.
    assert_eq!(AttrValue::Num(3.0).as_num(), Some(3.0));
    assert_eq!(AttrValue::Str("3".to_string()).as_num(), None);
    assert_eq!(AttrValue::Bool(true).as_num(), None);
}

#[test]
fn integers_render_without_a_spurious_decimal() {
    assert_eq!(AttrValue::Num(2.0).render(), "2");
    assert_eq!(AttrValue::Num(2.5).render(), "2.5");
    assert_eq!(AttrValue::Bool(false).render(), "false");
}

#[test]
fn attr_values_round_trip_through_json_without_changing_variant() {
    // `untagged` picks by shape, and the ORDER of the variants is what makes
    // `true` a Bool rather than a number. A reorder would be silent here without
    // this, and every boolean policy would stop matching.
    for value in [
        AttrValue::Bool(true),
        AttrValue::Num(2.0),
        AttrValue::Str("border".to_string()),
    ] {
        let json = serde_json::to_string(&value).unwrap();
        let back: AttrValue = serde_json::from_str(&json).unwrap();
        assert_eq!(back, value, "{json} changed variant on the way back");
    }
}

#[test]
fn a_faction_stamped_entity_in_a_shared_base_is_COUNTABLE() {
    // The FR-39 backstop: the shared base is common knowledge, so a
    // faction-stamped fact in it is private intel that reached the layer
    // everybody reads. Detection is the whole point — a leak nobody can count
    // is one that never gets found, because the run just looks well-informed.
    let mut g = base();
    assert!(g.faction_tagged_ids().is_empty(), "a clean base is clean");

    let mut leaked = StateNode::new("scout_7", "UnitState");
    leaked.provenance.faction = Some("gaians".to_string());
    g.upsert_node(leaked);
    assert_eq!(
        g.faction_tagged_ids().into_iter().collect::<Vec<_>>(),
        vec!["scout_7".to_string()]
    );
}

#[test]
fn a_state_node_can_only_ever_be_engine_state_tier() {
    // Not a settable field: the only way a board node could claim to be
    // tree-sitter-derived is if something could set it.
    assert_eq!(StateNode::new("x", "Thing").tier(), Tier::EngineState);
}

#[test]
fn provenance_renders_what_it_has_and_says_when_it_has_no_adapter() {
    let full = Provenance {
        adapter: Some("smac-worldview".to_string()),
        turn: Some(42),
        faction: Some("gaians".to_string()),
    };
    assert_eq!(full.render(), "smac-worldview@turn42/gaians");
    assert_eq!(Provenance::default().render(), "unknown-adapter");
}
