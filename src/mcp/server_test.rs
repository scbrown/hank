//! FR-3 enforcement: every `hank_*` MCP response carries its provenance tier
//! (aegis-8yrn). Child module of `server`, so it can drive the private tool
//! handlers directly; size-exempt (`_test.rs`).
//!
//! The bug this pins: `hank_impact`, `hank_callers`, `hank_callees` and
//! `hank_dataflow` served an unlabelled tree-sitter approximation — no `tier`
//! anywhere — which FR-3 exists to forbid, and which is worse on `hank_impact`
//! precisely because it is the trust-boundary/capability-scoping surface. This
//! walk asserts the served WIRE JSON, so a future response type that omits the
//! tag fails here rather than shipping silent.

use super::HankMcpServer;
use crate::mcp::tools::{
    AnalyzeRequest, CommunitiesRequest, DataflowRequest, ImpactRequest, NeighborsRequest,
    ReferencesRequest, SymbolsRequest, VerifyRequest,
};
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::CallToolResult;

const KNOWN_TIERS: &[&str] = &["treesitter", "lsp", "cpg"];

/// A two-function project: `a` calls `b`, so `b` has a caller and the graph is
/// non-empty. Fresh temp dir per test.
fn fixture() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("x.rs"), "fn a() { b(); }\nfn b() {}\n").unwrap();
    dir
}

fn server(dir: &tempfile::TempDir) -> HankMcpServer {
    HankMcpServer::new(dir.path().to_path_buf(), None, None)
}

/// The served JSON payload, parsed out of the MCP `CallToolResult` wire form
/// (`{ "content": [ { "type": "text", "text": "<json>" } ] }`). Asserting the
/// actual wire bytes is the point — a `tier` field that exists on the struct but
/// is dropped in serialization would still be caught.
fn served(result: Result<CallToolResult, rmcp::ErrorData>) -> serde_json::Value {
    let result = result.expect("handler returned Ok");
    let wire = serde_json::to_value(&result).expect("CallToolResult serializes");
    let text = wire
        .pointer("/content/0/text")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_else(|| panic!("no text content in result: {wire}"));
    serde_json::from_str(text).expect("served payload is JSON")
}

/// Does any object in the tree carry a `tier` key? Covers both per-item tags
/// (symbols/references) and the top-level tag (graph responses).
fn carries_tier(v: &serde_json::Value) -> bool {
    match v {
        serde_json::Value::Object(m) => m.contains_key("tier") || m.values().any(carries_tier),
        serde_json::Value::Array(a) => a.iter().any(carries_tier),
        _ => false,
    }
}

/// A top-level `tier` that is one of the known tiers. Used for the graph
/// responses, where the top-level tag is what makes an EMPTY / not-found answer
/// still declare its provenance.
fn assert_top_level_tier(payload: &serde_json::Value, tool: &str) {
    let tier = payload
        .get("tier")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_else(|| panic!("{tool}: no top-level `tier` in {payload}"));
    assert!(
        KNOWN_TIERS.contains(&tier),
        "{tool}: tier {tier:?} is not one of {KNOWN_TIERS:?}"
    );
}

#[tokio::test]
async fn impact_carries_a_top_level_tier() {
    let dir = fixture();
    let payload = served(
        server(&dir)
            .hank_impact(Parameters(ImpactRequest {
                symbol: "b".into(),
                path: None,
                hops: None,
                cochange: None,
            }))
            .await,
    );
    // The bug was that this — the trust-boundary surface — served no tier at all.
    assert_top_level_tier(&payload, "hank_impact");
    // And the per-item reach facts carry it too.
    let first = &payload["reachable"][0];
    assert_eq!(
        first["tier"], "treesitter",
        "reach item missing tier: {first}"
    );
}

#[tokio::test]
async fn impact_on_a_missing_symbol_still_declares_its_tier() {
    // The empty-case hole: a not-found answer has no items to tag, so without the
    // top-level tag it would arrive unlabelled and read as authoritative.
    let dir = fixture();
    let payload = served(
        server(&dir)
            .hank_impact(Parameters(ImpactRequest {
                symbol: "does_not_exist".into(),
                path: None,
                hops: None,
                cochange: None,
            }))
            .await,
    );
    assert_eq!(payload["found"], false);
    assert_top_level_tier(&payload, "hank_impact(not-found)");
}

#[tokio::test]
async fn callers_and_callees_carry_a_top_level_tier() {
    let dir = fixture();
    let callers = served(
        server(&dir)
            .hank_callers(Parameters(NeighborsRequest {
                symbol: "b".into(),
                path: None,
            }))
            .await,
    );
    assert_top_level_tier(&callers, "hank_callers");

    let callees = served(
        server(&dir)
            .hank_callees(Parameters(NeighborsRequest {
                symbol: "a".into(),
                path: None,
            }))
            .await,
    );
    assert_top_level_tier(&callees, "hank_callees");
}

#[tokio::test]
async fn dataflow_carries_a_top_level_tier() {
    let dir = fixture();
    let payload = served(
        server(&dir)
            .hank_dataflow(Parameters(DataflowRequest {
                function: "a".into(),
                path: None,
                var: None,
                forward: None,
                hops: None,
            }))
            .await,
    );
    assert_top_level_tier(&payload, "hank_dataflow");
}

#[tokio::test]
async fn every_fact_serving_response_carries_a_tier() {
    // The walk: each fact-serving tool's WIRE response must carry a tier somewhere
    // (top-level for graph/summary responses, per-item for symbol lists). If a new
    // response type omits it, its line here fails rather than shipping unlabelled.
    let dir = fixture();
    let s = server(&dir);

    let cases: Vec<(&str, serde_json::Value)> = vec![
        (
            "hank_symbols",
            served(
                s.hank_symbols(Parameters(SymbolsRequest {
                    file: "x.rs".into(),
                }))
                .await,
            ),
        ),
        (
            "hank_references",
            served(
                s.hank_references(Parameters(ReferencesRequest {
                    symbol: "a".into(),
                    path: None,
                }))
                .await,
            ),
        ),
        (
            "hank_analyze",
            served(
                s.hank_analyze(Parameters(AnalyzeRequest { path: None }))
                    .await,
            ),
        ),
        (
            "hank_communities",
            served(
                s.hank_communities(Parameters(CommunitiesRequest { path: None }))
                    .await,
            ),
        ),
        (
            "hank_verify",
            served(
                s.hank_verify(Parameters(VerifyRequest {
                    file: "x.rs".into(),
                    buffer: "fn a() { b(); }\nfn b() {}\n".into(),
                }))
                .await,
            ),
        ),
        (
            "hank_callers",
            served(
                s.hank_callers(Parameters(NeighborsRequest {
                    symbol: "b".into(),
                    path: None,
                }))
                .await,
            ),
        ),
        (
            "hank_impact",
            served(
                s.hank_impact(Parameters(ImpactRequest {
                    symbol: "b".into(),
                    path: None,
                    hops: None,
                    cochange: None,
                }))
                .await,
            ),
        ),
        (
            "hank_dataflow",
            served(
                s.hank_dataflow(Parameters(DataflowRequest {
                    function: "a".into(),
                    path: None,
                    var: None,
                    forward: None,
                    hops: None,
                }))
                .await,
            ),
        ),
    ];

    for (tool, payload) in &cases {
        assert!(
            carries_tier(payload),
            "{tool}: served response carries NO tier anywhere: {payload}"
        );
    }
}
