//! Tests for the FR-35 ingestion seam.
#![allow(non_snake_case)]

use super::*;
use crate::state::graph::StateGraph;

fn request(json: &str) -> IngestRequest {
    serde_json::from_str(json).expect("request parses")
}

/// The shape an adapter that also feeds `quipu_episode` emits.
const SHARED_BOARD: &str = r#"{
    "game_id": "g1",
    "faction_id": "gaians",
    "visibility": "shared",
    "provenance": {"adapter": "smac-worldview", "turn": 42},
    "entities": [
        {"name": "base_alpha", "type": "smac:BaseState", "description": "border base",
         "attrs": {"smac:isBorderBase": true, "smac:garrisonCount": 2}},
        {"name": "base_beta", "type": "smac:BaseState"}
    ],
    "edges": [
        {"source": "base_alpha", "target": "base_beta", "relation": "adjacent_to"}
    ]
}"#;

#[test]
fn the_quipu_episode_node_and_edge_shape_deserializes_unchanged() {
    // `name` / `type` / `description` for a node and `source` / `target` /
    // `relation` for an edge — the shared subset stays byte-identical so one
    // adapter output feeds both stores.
    let req = request(SHARED_BOARD);
    assert_eq!(req.entities[0].name, "base_alpha");
    assert_eq!(req.entities[0].kind, "smac:BaseState");
    assert_eq!(req.entities[0].description.as_deref(), Some("border base"));
    assert_eq!(
        req.entities[0].attrs.get("smac:isBorderBase"),
        Some(&AttrValue::Bool(true))
    );
    assert_eq!(req.edges[0].relation, "adjacent_to");
}

#[test]
fn a_shared_ingest_lands_in_the_base_and_reports_what_it_did() {
    let (mut base, mut overlay) = (StateGraph::new(), StateOverlay::new());
    let report = apply(&request(SHARED_BOARD), &mut base, &mut overlay);
    assert_eq!(report.nodes_added, 2);
    assert_eq!(report.nodes_replaced, 0);
    assert_eq!(report.edges_added, 1);
    assert!(report.rejected.is_empty());
    assert_eq!(report.board, (2, 1));
    assert_eq!(report.tier, "engine-state");
    assert!(overlay.is_empty(), "shared facts do not touch an overlay");
    assert_eq!(
        base.node("base_alpha").unwrap().provenance.turn,
        Some(42),
        "provenance is the FR-35 replacement for file:line, and it is kept"
    );
}

#[test]
fn re_ingesting_the_same_turn_is_IDEMPOTENT() {
    // A board is restated every turn. If that grew the graph, turn count would
    // be node count — and the "replaced" counter is what makes an overwrite
    // visible instead of reading as an insert.
    let (mut base, mut overlay) = (StateGraph::new(), StateOverlay::new());
    let _ = apply(&request(SHARED_BOARD), &mut base, &mut overlay);
    let second = apply(&request(SHARED_BOARD), &mut base, &mut overlay);
    assert_eq!(second.nodes_added, 0);
    assert_eq!(second.nodes_replaced, 2);
    assert_eq!(second.edges_added, 0);
    assert_eq!(second.edges_unchanged, 1);
    assert_eq!(second.board, (2, 1));
}

#[test]
fn a_private_ingest_lands_in_the_OVERLAY_and_leaves_the_base_alone() {
    let (mut base, mut overlay) = (StateGraph::new(), StateOverlay::new());
    let _ = apply(&request(SHARED_BOARD), &mut base, &mut overlay);
    let private = request(
        r#"{"game_id":"g1","faction_id":"gaians","visibility":"private",
            "provenance":{"adapter":"smac-worldview","turn":42},
            "entities":[{"name":"scout_7","type":"smac:UnitState"}],
            "edges":[{"source":"scout_7","target":"base_alpha","relation":"garrisoned_at"}]}"#,
    );
    let report = apply(&private, &mut base, &mut overlay);
    assert_eq!(report.nodes_added, 1);
    assert_eq!(report.edges_added, 1);
    assert!(base.node("scout_7").is_none(), "the base never saw it");
    assert!(
        StateView::new(&base, Some(&overlay))
            .node("scout_7")
            .is_some(),
        "but the ingesting tenant does"
    );
}

#[test]
fn a_private_fact_is_stamped_with_the_tenants_faction_when_the_adapter_omits_one() {
    // Filling this in is not invented provenance — the tenant id is a fact about
    // the call. It is what makes a mis-routed fact detectable later.
    let (mut base, mut overlay) = (StateGraph::new(), StateOverlay::new());
    let private = request(
        r#"{"game_id":"g1","faction_id":"gaians","visibility":"private",
            "entities":[{"name":"scout_7","type":"smac:UnitState"}]}"#,
    );
    let _ = apply(&private, &mut base, &mut overlay);
    assert_eq!(
        StateView::new(&base, Some(&overlay))
            .node("scout_7")
            .unwrap()
            .provenance
            .faction
            .as_deref(),
        Some("gaians")
    );
}

#[test]
fn a_SHARED_write_carrying_a_faction_is_REFUSED_and_COUNTED() {
    // The one fog-isolation path the type system cannot close: overlays are
    // structurally disjoint, but nothing stops an adapter posting private intel
    // with `visibility: shared`. A leak here would be invisible in results —
    // the run just looks unusually well-informed.
    let (mut base, mut overlay) = (StateGraph::new(), StateOverlay::new());
    let leak = request(
        r#"{"game_id":"g1","faction_id":"gaians","visibility":"shared",
            "provenance":{"adapter":"smac-worldview","turn":42,"faction":"gaians"},
            "entities":[{"name":"scout_7","type":"smac:UnitState"}]}"#,
    );
    let report = apply(&leak, &mut base, &mut overlay);
    assert_eq!(report.nodes_added, 0, "nothing was written");
    assert_eq!(report.fog_leaks_blocked, 1, "and it was counted");
    assert!(report.rejected[0].contains("COMMON KNOWLEDGE"));
    assert!(
        report.rejected[0].contains("visibility: private"),
        "the refusal says how to fix it: {}",
        report.rejected[0]
    );
    assert!(base.is_empty());
}

#[test]
fn a_DANGLING_edge_is_refused_rather_than_stored() {
    // A pattern traversing it would bind a variable to an id with no entity
    // behind it — a match against something that is not there.
    let (mut base, mut overlay) = (StateGraph::new(), StateOverlay::new());
    let req = request(
        r#"{"game_id":"g1","faction_id":"gaians","visibility":"shared",
            "entities":[{"name":"base_alpha","type":"smac:BaseState"}],
            "edges":[{"source":"base_alpha","target":"nowhere","relation":"adjacent_to"}]}"#,
    );
    let report = apply(&req, &mut base, &mut overlay);
    assert_eq!(report.edges_added, 0);
    assert_eq!(report.rejected.len(), 1);
    assert!(report.rejected[0].contains("nowhere"));
    assert_eq!(base.stats(), (1, 0));
}

#[test]
fn an_entity_with_no_id_or_no_type_is_refused() {
    let (mut base, mut overlay) = (StateGraph::new(), StateOverlay::new());
    let req = request(
        r#"{"game_id":"g1","faction_id":"gaians","visibility":"shared",
            "entities":[{"name":"","type":"smac:BaseState"},
                        {"name":"x","type":"  "}]}"#,
    );
    let report = apply(&req, &mut base, &mut overlay);
    assert_eq!(report.nodes_added, 0);
    assert_eq!(report.rejected.len(), 2);
}

#[test]
fn visibility_has_NO_default_so_an_adapter_must_state_it() {
    // Guessing here means one faction's private intel silently becoming common
    // knowledge. A missing field must fail to parse, not pick a side.
    let missing = r#"{"game_id":"g1","faction_id":"gaians","entities":[]}"#;
    assert!(serde_json::from_str::<IngestRequest>(missing).is_err());
}
