//! Tests for the board HTTP surface (FR-35/37/38).
#![allow(non_snake_case)]

use std::time::Duration;

use super::*;
use crate::daemon::http::router;

fn tiny_repo() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("leaf.rs"), "fn leaf() {}\n").unwrap();
    dir
}

async fn spawn(engine: ResidentEngine) -> u16 {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        let _ = axum::serve(listener, router(engine)).await;
    });
    tokio::time::sleep(Duration::from_millis(50)).await;
    port
}

/// `(status_code, body_text)`.
async fn post(port: u16, path: &str, body: &str) -> (u16, String) {
    let (path, body) = (path.to_string(), body.to_string());
    tokio::task::spawn_blocking(move || {
        use std::io::{Read, Write};
        let mut s = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
        let req = format!(
            "POST {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: application/json\r\n\
             Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        s.write_all(req.as_bytes()).unwrap();
        let mut raw = String::new();
        s.read_to_string(&mut raw).unwrap();
        let code: u16 = raw
            .split_whitespace()
            .nth(1)
            .and_then(|c| c.parse().ok())
            .unwrap();
        let body = raw.split("\r\n\r\n").nth(1).unwrap_or_default().to_string();
        (code, body)
    })
    .await
    .unwrap()
}

async fn get(port: u16, path: &str) -> serde_json::Value {
    let path = path.to_string();
    tokio::task::spawn_blocking(move || {
        use std::io::{Read, Write};
        let mut s = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
        let req = format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n");
        s.write_all(req.as_bytes()).unwrap();
        let mut raw = String::new();
        s.read_to_string(&mut raw).unwrap();
        let body = raw.split("\r\n\r\n").nth(1).unwrap_or_default();
        // Chunked or not, the JSON object is the first `{` to the last `}`.
        let start = body.find('{').unwrap();
        let end = body.rfind('}').unwrap();
        serde_json::from_str(&body[start..=end]).unwrap()
    })
    .await
    .unwrap()
}

const BOARD: &str = r#"{
    "game_id": "g1", "faction_id": "gaians", "visibility": "shared",
    "provenance": {"adapter": "smac-worldview", "turn": 7},
    "entities": [
        {"name": "base_alpha", "type": "smac:BaseState",
         "attrs": {"smac:isBorderBase": true, "smac:garrisonCount": 1}}
    ]
}"#;

const GARRISON_POLICY: &str = r#"{
    "label": "garrison-border-bases",
    "claim": "every border base retains >=1 garrison",
    "boundary": "order", "effect": "deny",
    "selector": {"selector_lang": "graph-pattern",
                 "evidence_source": "?b a smac:BaseState ; smac:isBorderBase true"},
    "predicate": {"selector_lang": "graph-pattern", "match_type": "must-match",
                  "evidence_source": "?b smac:garrisonCount ?n | ?n >= 1"}
}"#;

fn strip_order() -> String {
    r#"{"id": "move-out", "effects": [
        {"op": "set_attr", "id": "base_alpha", "key": "smac:garrisonCount", "value": 0}]}"#
        .to_string()
}

async fn live_board() -> u16 {
    let dir = tiny_repo();
    let engine = ResidentEngine::build(dir.path(), None).unwrap();
    let port = spawn(engine).await;
    // Leak the tempdir for the test's lifetime: the daemon holds the root.
    std::mem::forget(dir);
    let (code, _) = post(port, "/ingest", BOARD).await;
    assert_eq!(code, 200, "the board must ingest");
    port
}

#[tokio::test]
async fn guarding_a_board_that_was_never_ingested_is_409_not_200() {
    // The safety property of this whole surface. Ingest and guard are separate
    // calls, so "nothing was ingested into THIS process" and "these orders are
    // fine" would otherwise be the same 200 with an empty violations list.
    let dir = tiny_repo();
    let engine = ResidentEngine::build(dir.path(), None).unwrap();
    let port = spawn(engine).await;

    let body = format!(
        r#"{{"game_id":"g1","faction_id":"gaians","policies":[{GARRISON_POLICY}],
             "orders":[{}]}}"#,
        strip_order()
    );
    let (code, text) = post(port, "/guard", &body).await;
    assert_eq!(code, 409, "an empty board must REFUSE: {text}");
    assert!(text.contains("no board is loaded"), "{text}");
}

#[tokio::test]
async fn ingest_then_guard_DENIES_the_order_that_breaks_the_policy() {
    let port = live_board().await;
    let body = format!(
        r#"{{"game_id":"g1","faction_id":"gaians","policies":[{GARRISON_POLICY}],
             "orders":[{}]}}"#,
        strip_order()
    );
    let (code, text) = post(port, "/guard", &body).await;
    assert_eq!(code, 200, "{text}");
    let json: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(json["outcome"], "evaluated");
    assert_eq!(json["violations"][0]["policy"], "garrison-border-bases");
    assert_eq!(json["violations"][0]["tier"], "engine-state");
    assert_eq!(json["violations"][0]["offending_order_ids"][0], "move-out");
}

#[tokio::test]
async fn a_compliant_order_set_is_allowed_over_the_SAME_board() {
    // The positive control for the test above: same board, same policy, an order
    // that keeps the garrison. Without this, "409 then 200-with-violations"
    // could both be produced by a guard that denies everything.
    let port = live_board().await;
    let body = format!(
        r#"{{"game_id":"g1","faction_id":"gaians","policies":[{GARRISON_POLICY}],
             "orders":[{{"id":"reinforce","effects":[
               {{"op":"set_attr","id":"base_alpha","key":"smac:garrisonCount","value":3}}]}}]}}"#
    );
    let (code, text) = post(port, "/guard", &body).await;
    assert_eq!(code, 200);
    let json: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(json["violations"].as_array().unwrap().len(), 0);
    assert_eq!(json["vacuous"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn whatif_over_an_empty_board_is_409_too() {
    let dir = tiny_repo();
    let engine = ResidentEngine::build(dir.path(), None).unwrap();
    let port = spawn(engine).await;
    let (code, text) = post(
        port,
        "/whatif",
        r#"{"game_id":"g1","faction_id":"gaians","orders":[]}"#,
    )
    .await;
    assert_eq!(code, 409, "{text}");
}

#[tokio::test]
async fn whatif_answers_without_committing_and_the_board_is_unchanged() {
    let port = live_board().await;
    let body = format!(
        r#"{{"game_id":"g1","faction_id":"gaians","orders":[{}],"hops":2}}"#,
        strip_order()
    );
    let (code, text) = post(port, "/whatif", &body).await;
    assert_eq!(code, 200, "{text}");
    let json: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(json["committed"], false);
    assert_eq!(json["changes"][0]["detail"], "smac:garrisonCount: 1 -> 0");

    // Ask the board again: the speculation left nothing behind.
    let after: serde_json::Value = serde_json::from_str(
        &post(
            port,
            "/whatif",
            &format!(
                r#"{{"game_id":"g1","faction_id":"gaians","orders":[{}],"hops":2}}"#,
                strip_order()
            ),
        )
        .await
        .1,
    )
    .unwrap();
    assert_eq!(
        after["changes"][0]["detail"], "smac:garrisonCount: 1 -> 0",
        "the same delta from the same starting board — the first call did not commit"
    );
}

#[tokio::test]
async fn a_faction_stamped_SHARED_ingest_is_refused_and_counted_in_status() {
    let port = live_board().await;
    let leak = r#"{"game_id":"g1","faction_id":"gaians","visibility":"shared",
        "provenance":{"adapter":"a","turn":7,"faction":"gaians"},
        "entities":[{"name":"scout_7","type":"smac:UnitState"}]}"#;
    let (code, text) = post(port, "/ingest", leak).await;
    assert_eq!(
        code, 200,
        "a refusal is reported IN the report, not as a 5xx"
    );
    let json: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(json["nodes_added"], 0);
    assert_eq!(json["fog_leaks_blocked"], 1);

    let status = get(port, "/status").await;
    assert_eq!(status["board_layer"]["fog_leaks_blocked"], 1);
}

#[tokio::test]
async fn status_reports_the_board_layer_alongside_the_code_graph() {
    let port = live_board().await;
    let status = get(port, "/status").await;
    assert!(
        status["nodes"].as_u64().unwrap() > 0,
        "the code graph is still there"
    );
    let game = &status["board_layer"]["games"][0];
    assert_eq!(game["game_id"], "g1");
    assert_eq!(game["shared_nodes"], 1);
    assert!(
        status["tier"]
            .as_array()
            .unwrap()
            .contains(&serde_json::json!("engine-state")),
        "and the tier this build can serve is advertised: {}",
        status["tier"]
    );
}
