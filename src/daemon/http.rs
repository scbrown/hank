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
//! - stage 4: the rest of the FR-27 query surface — `/references` and `/symbols`
//!   from the resident node index, and `/dataflow`, which is NOT resident (no
//!   resident dataflow model exists yet, hank #22) but is served per-request so
//!   the HTTP API is complete rather than silently partial.

use std::path::PathBuf;

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;

use super::wire::{graph_tier, DataflowReply, DepEdgeItem, FlowStepItem};
use super::ResidentEngine;
use super::{Definitions, EngineStatus, FileSymbols, Impact, MeasureReply, Neighbors};
use crate::dataflow::{Dataflow, FlowDir};
use crate::graph::Dir;

/// Build the daemon router over a resident engine.
pub fn router(engine: ResidentEngine) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/status", get(status))
        .route("/callers", get(callers))
        .route("/callees", get(callees))
        .route("/impact", get(impact))
        .route("/references", get(references))
        .route("/symbols", get(symbols))
        .route("/dataflow", get(dataflow))
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

/// Definition sites of a symbol by name, from the resident node index — the
/// `hank_references` answer with no per-call re-extraction.
async fn references(
    State(engine): State<ResidentEngine>,
    Query(q): Query<SymbolQuery>,
) -> Json<Definitions> {
    Json(engine.references(&q.symbol))
}

/// `?file=REL` for `/symbols`.
#[derive(Debug, Deserialize)]
struct FileQuery {
    file: String,
}

/// The symbols one file contributes to the resident graph, at its build
/// snapshot. See [`FileSymbols`] for the `known` semantics.
async fn symbols(
    State(engine): State<ResidentEngine>,
    Query(q): Query<FileQuery>,
) -> Json<FileSymbols> {
    Json(engine.symbols(&q.file))
}

/// Query for `/dataflow`, mirroring the `hank_dataflow` request.
#[derive(Debug, Deserialize)]
struct DataflowQuery {
    /// The function to analyze.
    function: String,
    /// Subtree to build over, relative to the root; omit for the whole root.
    path: Option<String>,
    /// Variable to trace; omit to return all dependence edges.
    var: Option<String>,
    /// Trace what the variable flows into (default: what it depends on).
    forward: Option<bool>,
    /// Maximum hops to follow (default 5).
    hops: Option<u32>,
}

/// Intra-procedural data dependence, mirroring `hank_dataflow`. NOT resident —
/// built per request (see the module doc) — and CONFINED TO THE ROOT like
/// `/measure`: a `path` resolving outside the resident root is refused (400),
/// so the localhost daemon cannot be pointed at arbitrary trees.
async fn dataflow(
    State(engine): State<ResidentEngine>,
    Query(q): Query<DataflowQuery>,
) -> Result<Json<DataflowReply>, StatusCode> {
    let root = engine.root();
    let base = match &q.path {
        Some(p) => {
            let joined = root.join(p);
            // Canonicalize BOTH sides so `..` cannot escape the root.
            let (Ok(canon), Ok(canon_root)) = (joined.canonicalize(), root.canonicalize()) else {
                return Err(StatusCode::BAD_REQUEST);
            };
            if !canon.starts_with(&canon_root) {
                return Err(StatusCode::BAD_REQUEST);
            }
            canon
        }
        None => root.to_path_buf(),
    };
    let flow = Dataflow::build(&base).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let found = flow.has_function(&q.function);
    let (direction, steps, edges) = match &q.var {
        Some(var) => {
            let dir = if q.forward.unwrap_or(false) {
                FlowDir::FlowsInto
            } else {
                FlowDir::DependsOn
            };
            let steps = flow
                .flow(&q.function, var, dir, q.hops.unwrap_or(5))
                .into_iter()
                .map(|s| FlowStepItem {
                    name: s.name,
                    distance: s.distance,
                })
                .collect();
            (Some(dir.as_str().to_string()), steps, Vec::new())
        }
        None => {
            let edges = flow
                .edges(&q.function)
                .iter()
                .map(|e| DepEdgeItem {
                    dependent: e.dependent.clone(),
                    depends_on: e.depends_on.clone(),
                    line: e.line,
                })
                .collect();
            (None, Vec::new(), edges)
        }
    };
    Ok(Json(DataflowReply {
        function: q.function,
        found,
        direction,
        var: q.var,
        flow: steps,
        edges,
        tier: graph_tier(),
    }))
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
#[path = "http_test.rs"]
mod http_test;
