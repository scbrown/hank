//! `POST /ingest`, `/guard` and `/whatif` — the FR-35/37/38 HTTP surface.
//!
//! A sibling of [`super::http`] rather than more routes inside it: the board
//! endpoints are gated on `game-state` and the code endpoints are not, and
//! interleaving two feature gates through one router function is how a route
//! ends up mounted on a build that cannot serve it.
//!
//! ## Status codes, and the one that matters
//!
//! A guard over an EMPTY board answers **409 Conflict**, not 200-with-no-
//! violations. That is the whole safety property of this surface: ingest and
//! guard are separate calls, so "nothing was ingested into THIS process" and
//! "these orders are fine" would otherwise be the same 200. A caller that gates
//! on HTTP status alone still cannot mistake one for the other.

use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::post;
use axum::{Json, Router};
use serde::Deserialize;

use super::ResidentEngine;
use crate::state::{
    GuardOutcome, IngestReport, IngestRequest, Order, StatePolicy, TenantKey, WhatIfOutcome,
};

/// The board endpoints, merged into the daemon router.
pub fn routes() -> Router<ResidentEngine> {
    Router::new()
        .route("/ingest", post(ingest))
        .route("/guard", post(guard))
        .route("/whatif", post(whatif))
}

/// A board could not be read or written because the lock was poisoned by a
/// panic in another request. An explicit 503, never a silent empty board — an
/// empty board is exactly what the guard exists to refuse.
const BOARD_UNAVAILABLE: StatusCode = StatusCode::SERVICE_UNAVAILABLE;

/// The refusal status for a guard or what-if that could not be answered. 409:
/// the request is well-formed, the SERVER's state makes it unanswerable.
const CANNOT_GUARD: StatusCode = StatusCode::CONFLICT;

/// `POST /ingest` — FR-35. The request carries its own `(game_id, faction_id)`
/// and `visibility`; routing between the shared base and this tenant's overlay
/// happens inside the registry, not here.
async fn ingest(
    State(engine): State<ResidentEngine>,
    Json(req): Json<IngestRequest>,
) -> Result<Json<IngestReport>, (StatusCode, String)> {
    let mut board = engine
        .board()
        .write()
        .map_err(|_| (BOARD_UNAVAILABLE, "board lock poisoned".to_string()))?;
    board
        .ingest(&req)
        .map(Json)
        .map_err(|reason| (StatusCode::TOO_MANY_REQUESTS, reason))
}

/// `POST /guard` and `/whatif` name the tenant explicitly — a board query has no
/// ambient game the way a code query has an ambient repo.
#[derive(Debug, Deserialize)]
struct GuardBody {
    game_id: String,
    faction_id: String,
    /// The policies to evaluate. Supplied per call rather than held resident:
    /// they are authored in Quipu and projected, and a stale resident copy would
    /// enforce yesterday's governance while looking current.
    #[serde(default)]
    policies: Vec<StatePolicy>,
    /// The proposed orders.
    #[serde(default)]
    orders: Vec<Order>,
}

/// `POST /guard` — FR-37.
async fn guard(
    State(engine): State<ResidentEngine>,
    Json(req): Json<GuardBody>,
) -> Result<Json<GuardOutcome>, (StatusCode, String)> {
    let board = engine
        .board()
        .read()
        .map_err(|_| (BOARD_UNAVAILABLE, "board lock poisoned".to_string()))?;
    let key = TenantKey::new(&req.game_id, &req.faction_id);
    match board.guard(&key, &req.policies, &req.orders) {
        // The refusal is a STATUS, not a field a caller has to remember to read.
        GuardOutcome::Refused { reason } => Err((CANNOT_GUARD, reason)),
        evaluated @ GuardOutcome::Evaluated(_) => Ok(Json(evaluated)),
    }
}

/// `POST /whatif` body — the guard body plus a hop ceiling.
#[derive(Debug, Deserialize)]
struct WhatIfBody {
    game_id: String,
    faction_id: String,
    #[serde(default)]
    orders: Vec<Order>,
    /// Hops to follow; defaults to 3. Lower than the code plane's 5 on purpose —
    /// a board is far denser than a call graph, and this is the hot, this-turn
    /// path.
    #[serde(default)]
    hops: Option<u32>,
}

/// `POST /whatif` — FR-38.
async fn whatif(
    State(engine): State<ResidentEngine>,
    Json(req): Json<WhatIfBody>,
) -> Result<Json<WhatIfOutcome>, (StatusCode, String)> {
    let board = engine
        .board()
        .read()
        .map_err(|_| (BOARD_UNAVAILABLE, "board lock poisoned".to_string()))?;
    let key = TenantKey::new(&req.game_id, &req.faction_id);
    match board.whatif(&key, &req.orders, req.hops.unwrap_or(3)) {
        WhatIfOutcome::Refused { reason } => Err((CANNOT_GUARD, reason)),
        evaluated @ WhatIfOutcome::Evaluated(_) => Ok(Json(evaluated)),
    }
}

#[cfg(test)]
#[path = "state_http_test.rs"]
mod state_http_test;
