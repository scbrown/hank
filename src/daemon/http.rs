//! The daemon's HTTP surface (aegis-1qze).
//!
//! - stage 1: `/health` (a bare 200 for the [`super::client`] probe) and
//!   `/status` (the resident graph's real counts).
//! - stage 2: the graph-backed query endpoints — `/callers`, `/callees`,
//!   `/impact` — answered from the RESIDENT graph with no per-call rebuild, which
//!   is the daemon's whole point. They mirror the `hank_callers`/`callees`/`impact`
//!   MCP tools, so stage 3 can point the MCP surface at the same engine methods.
//! - stage 3a: `/measure` (POST) — the pre-edit guard's exact blast-radius
//!   question, sized against the resident graph. This is what the hook becomes a
//!   thin client of; the client + hook cutover (with loud-when-absent) is stage 3b.
//!
//! Still deferred: `/symbols` and `/references` (they re-extract from files rather
//! than reading the resident graph, so they are not the daemon's value-add) and
//! `/dataflow` (a separate subsystem, not the `CodeGraph`). Kept out of stage 2 to
//! keep it about the resident graph; noted here rather than silently omitted.

use std::path::PathBuf;

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;

use super::{EngineStatus, Impact, MeasureReply, Neighbors, ResidentEngine};
use crate::graph::Dir;

/// Build the daemon router over a resident engine.
pub fn router(engine: ResidentEngine) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/status", get(status))
        .route("/callers", get(callers))
        .route("/callees", get(callees))
        .route("/impact", get(impact))
        .route("/measure", post(measure))
        .with_state(engine)
}

/// Liveness: a healthy daemon answers 200. The [`super::client::probe`] keys on
/// this and nothing else, so it stays a bare, dependency-free 200.
async fn health() -> &'static str {
    "ok"
}

/// The resident graph's real facts — what an operator (or `hank daemon status`,
/// later) reads to confirm the daemon is holding a non-empty graph.
async fn status(State(engine): State<ResidentEngine>) -> Json<EngineStatus> {
    Json(engine.status())
}

/// `?symbol=NAME` for the neighbor endpoints.
#[derive(Debug, Deserialize)]
struct SymbolQuery {
    symbol: String,
}

/// `?symbol=NAME&hops=N` for `/impact`; `hops` defaults to 5, matching the CLI.
#[derive(Debug, Deserialize)]
struct ImpactQuery {
    symbol: String,
    hops: Option<u32>,
}

/// Direct callers of a symbol, from the resident graph.
async fn callers(
    State(engine): State<ResidentEngine>,
    Query(q): Query<SymbolQuery>,
) -> Json<Neighbors> {
    Json(engine.neighbors(&q.symbol, Dir::Callers))
}

/// Direct callees of a symbol, from the resident graph.
async fn callees(
    State(engine): State<ResidentEngine>,
    Query(q): Query<SymbolQuery>,
) -> Json<Neighbors> {
    Json(engine.neighbors(&q.symbol, Dir::Callees))
}

/// Blast radius of changing a symbol, from the resident graph.
async fn impact(
    State(engine): State<ResidentEngine>,
    Query(q): Query<ImpactQuery>,
) -> Json<Impact> {
    Json(engine.impact(&q.symbol, q.hops.unwrap_or(5)))
}

/// The pre-edit guard's exact question, as a POST body: size editing `file` (whose
/// anchors are the replaced texts) against the resident graph. This is what the
/// hook thin client calls in the cutover instead of building the graph itself.
#[derive(Debug, Deserialize)]
struct MeasureRequest {
    /// The edited file, absolute or root-relative.
    file: String,
    /// The edited file's path relative to the root (excluded from its own radius).
    rel: String,
    /// The replaced texts the edit lands inside; empty = whole-file.
    #[serde(default)]
    anchors: Vec<String>,
    /// Hops to follow; defaults to the resident policy's `max_hops`.
    #[serde(default)]
    max_hops: Option<u32>,
}

/// Size an edit against the resident graph. CONFINED TO THE ROOT: the request names
/// a file to read, and a localhost daemon must not become an arbitrary-file reader,
/// so a path resolving outside the resident root is refused (400) rather than read.
async fn measure(
    State(engine): State<ResidentEngine>,
    Json(req): Json<MeasureRequest>,
) -> Result<Json<MeasureReply>, StatusCode> {
    let root = engine.root();
    let raw = PathBuf::from(&req.file);
    let file = if raw.is_absolute() {
        raw
    } else {
        root.join(raw)
    };
    // Canonicalize BOTH sides so `..` cannot escape the root; a file that does not
    // exist cannot be canonicalized, but then there is nothing to read either.
    let (Ok(canon_file), Ok(canon_root)) = (file.canonicalize(), root.canonicalize()) else {
        return Err(StatusCode::BAD_REQUEST);
    };
    if !canon_file.starts_with(&canon_root) {
        return Err(StatusCode::BAD_REQUEST);
    }
    let hops = req.max_hops.unwrap_or(engine.policy().max_hops);
    let sizing = engine.measure_edit(&canon_file, &req.rel, &req.anchors, hops);
    Ok(Json(MeasureReply::from_sizing(&sizing)))
}

/// Bind `host:port` and serve the router until the process is signalled.
pub async fn serve(engine: ResidentEngine, host: &str, port: u16) -> anyhow::Result<()> {
    let addr = format!("{host}:{port}");
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    eprintln!("hank daemon: liveness surface on http://{addr}/health");
    axum::serve(listener, router(engine)).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon::client::{probe, Reachability};
    use std::time::Duration;
    use tempfile::tempdir;

    // Build a tiny real repo so the resident graph is non-empty.
    fn tiny_repo() -> tempfile::TempDir {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("leaf.rs"), "fn leaf() {}\n").unwrap();
        std::fs::write(dir.path().join("caller.rs"), "fn caller() { leaf(); }\n").unwrap();
        dir
    }

    #[tokio::test]
    async fn a_running_daemon_answers_the_probe_UP_and_serves_real_counts() {
        let dir = tiny_repo();
        let engine = ResidentEngine::build(dir.path(), None).unwrap();
        let served = engine.status();
        assert!(served.nodes > 0, "the resident graph must be non-empty");

        // Bind an ephemeral port and serve in the background.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            let _ = axum::serve(listener, router(engine)).await;
        });
        // Let the listener accept.
        tokio::time::sleep(Duration::from_millis(50)).await;

        // The client seam sees it as UP.
        let host = "127.0.0.1".to_string();
        let r = tokio::task::spawn_blocking(move || probe(&host, port, Duration::from_millis(500)))
            .await
            .unwrap();
        assert_eq!(r, Reachability::Up, "a live daemon must probe UP");

        // And /status reports the same real counts the engine holds.
        let body = get_json(port, "/status").await;
        assert_eq!(body["nodes"].as_u64().unwrap() as usize, served.nodes);
        assert_eq!(body["edges"].as_u64().unwrap() as usize, served.edges);
        assert_eq!(body["status"].as_str().unwrap(), "ok");
    }

    #[tokio::test]
    async fn a_DEAD_daemon_probes_DOWN_never_up() {
        // No server bound. The seam must say Down, loudly, with a reason — this is
        // the "killing it is the cheapest bypass" case the whole seam exists for.
        let r = tokio::task::spawn_blocking(|| probe("127.0.0.1", 1, Duration::from_millis(200)))
            .await
            .unwrap();
        assert!(!r.is_up());
        assert!(r.down_reason().unwrap().contains("no daemon"));
    }

    #[tokio::test]
    async fn the_query_endpoints_answer_from_the_resident_graph() {
        let dir = tiny_repo();
        let engine = ResidentEngine::build(dir.path(), None).unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            let _ = axum::serve(listener, router(engine)).await;
        });
        tokio::time::sleep(Duration::from_millis(50)).await;

        // /callers?symbol=leaf — `caller` calls `leaf`.
        let callers = get_json(port, "/callers?symbol=leaf").await;
        assert_eq!(callers["found"].as_bool().unwrap(), true);
        let names: Vec<&str> = callers["neighbors"]
            .as_array()
            .unwrap()
            .iter()
            .map(|n| n["name"].as_str().unwrap())
            .collect();
        assert!(names.contains(&"caller"), "got {names:?}");
        assert_eq!(callers["tier"].as_str().unwrap(), "treesitter");

        // /impact?symbol=leaf — the transitive blast radius.
        let impact = get_json(port, "/impact?symbol=leaf&hops=5").await;
        assert_eq!(impact["found"].as_bool().unwrap(), true);
        assert!(impact["count"].as_u64().unwrap() >= 1);

        // An unknown symbol is found=false, not an error.
        let missing = get_json(port, "/callers?symbol=nope").await;
        assert_eq!(missing["found"].as_bool().unwrap(), false);
    }

    #[tokio::test]
    async fn measure_sizes_an_edit_and_confines_to_the_root() {
        let dir = tiny_repo(); // leaf.rs <- caller.rs
        let engine = ResidentEngine::build(dir.path(), None).unwrap();
        let leaf = dir.path().join("leaf.rs").to_string_lossy().to_string();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            let _ = axum::serve(listener, router(engine)).await;
        });
        tokio::time::sleep(Duration::from_millis(50)).await;

        // Editing leaf reaches caller — measured, radius >= 1.
        let (code, body) = post_json(
            port,
            "/measure",
            &serde_json::json!({ "file": leaf, "rel": "leaf.rs", "anchors": ["fn leaf"] })
                .to_string(),
        )
        .await;
        assert_eq!(code, 200);
        let body = body.unwrap();
        assert_eq!(body["measured"].as_bool().unwrap(), true);
        assert!(body["symbols"].as_u64().unwrap() >= 1);

        // A path OUTSIDE the root is refused, not read — the localhost daemon must
        // not be an arbitrary-file reader.
        let (code, _) = post_json(
            port,
            "/measure",
            &serde_json::json!({ "file": "/etc/passwd", "rel": "x" }).to_string(),
        )
        .await;
        assert_eq!(code, 400, "a file outside the root must be refused");
    }

    // Minimal HTTP GET without pulling a client dep into the crate: reuse the
    // probe's raw-socket approach and read the whole body as JSON.
    async fn get_json(port: u16, path: &str) -> serde_json::Value {
        let path = path.to_string();
        tokio::task::spawn_blocking(move || {
            use std::io::{Read, Write};
            let mut s = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
            let req =
                format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n");
            s.write_all(req.as_bytes()).unwrap();
            let mut raw = String::new();
            s.read_to_string(&mut raw).unwrap();
            let body = raw.split("\r\n\r\n").nth(1).unwrap_or("").to_string();
            serde_json::from_str(&body).unwrap()
        })
        .await
        .unwrap()
    }

    // POST a JSON body; return (status_code, parsed_body_if_2xx).
    async fn post_json(port: u16, path: &str, body: &str) -> (u16, Option<serde_json::Value>) {
        let path = path.to_string();
        let body = body.to_string();
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
            let status: u16 = raw
                .split_whitespace()
                .nth(1)
                .and_then(|c| c.parse().ok())
                .unwrap_or(0);
            let payload = raw.split("\r\n\r\n").nth(1).unwrap_or("");
            let parsed = if (200..300).contains(&status) {
                serde_json::from_str(payload).ok()
            } else {
                None
            };
            (status, parsed)
        })
        .await
        .unwrap()
    }
}
