//! Bodies for the board tools, feature-split on `game-state`.
//!
//! The same shape `hank_promote` uses for the `quipu` arm: the tool is always
//! registered, and without the engine the body is an honest refusal naming the
//! feature. A tool that accepted a board and silently did nothing would be worse
//! than one that is absent — the caller would believe it had guarded.
//!
//! ## The board lives in THIS process
//!
//! The MCP server holds its own [`crate::state::StateRegistry`], as the daemon
//! holds its own. That is what a hot in-memory graph means, and it has a sharp
//! edge worth stating: ingesting over `POST /ingest` and then guarding through
//! `hank_guard` guards an EMPTY board. The refusal on an empty board is what
//! makes that a loud error instead of a clean report.

use super::*;

/// Body of `hank_ingest`.
pub(super) fn ingest(
    server: &HankMcpServer,
    req: &StateIngestRequest,
) -> Result<CallToolResult, McpError> {
    #[cfg(not(feature = "game-state"))]
    {
        let _ = (server, req);
        Err(internal(crate::errors::Error::Config(
            "hank_ingest needs the `game-state` feature; this server was built without it. \
             Nothing was ingested."
                .to_string(),
        )))
    }
    #[cfg(feature = "game-state")]
    {
        let mut board = server.board.write().map_err(|_| {
            internal(crate::errors::Error::Config(
                "the board lock is poisoned by a panic in an earlier request".to_string(),
            ))
        })?;
        let report = board
            .ingest(req)
            .map_err(|reason| internal(crate::errors::Error::Config(reason)))?;
        json_result(&report)
    }
}

/// Body of `hank_guard`.
pub(super) fn guard(
    server: &HankMcpServer,
    req: &StateGuardRequest,
) -> Result<CallToolResult, McpError> {
    #[cfg(not(feature = "game-state"))]
    {
        let _ = (server, req);
        Err(internal(crate::errors::Error::Config(
            "hank_guard needs the `game-state` feature; this server was built without it. These \
             orders were NOT checked — do not read this error as an approval."
                .to_string(),
        )))
    }
    #[cfg(feature = "game-state")]
    {
        let board = server.board.read().map_err(|_| {
            internal(crate::errors::Error::Config(
                "the board lock is poisoned by a panic in an earlier request".to_string(),
            ))
        })?;
        let key = crate::state::TenantKey::new(&req.game_id, &req.faction_id);
        match board.guard(&key, &req.policies, &req.orders) {
            // An ERROR, not a result with an empty violations list. A tool
            // result is something a model reads as an answer; the refusal must
            // not be one of those.
            crate::state::GuardOutcome::Refused { reason } => {
                Err(internal(crate::errors::Error::Config(reason)))
            }
            crate::state::GuardOutcome::Evaluated(report) => json_result(&report),
        }
    }
}

/// Body of `hank_whatif`.
pub(super) fn whatif(
    server: &HankMcpServer,
    req: &StateWhatIfRequest,
) -> Result<CallToolResult, McpError> {
    #[cfg(not(feature = "game-state"))]
    {
        let _ = (server, req);
        Err(internal(crate::errors::Error::Config(
            "hank_whatif needs the `game-state` feature; this server was built without it."
                .to_string(),
        )))
    }
    #[cfg(feature = "game-state")]
    {
        let board = server.board.read().map_err(|_| {
            internal(crate::errors::Error::Config(
                "the board lock is poisoned by a panic in an earlier request".to_string(),
            ))
        })?;
        let key = crate::state::TenantKey::new(&req.game_id, &req.faction_id);
        match board.whatif(&key, &req.orders, req.hops.unwrap_or(3)) {
            crate::state::WhatIfOutcome::Refused { reason } => {
                Err(internal(crate::errors::Error::Config(reason)))
            }
            crate::state::WhatIfOutcome::Evaluated(report) => json_result(&report),
        }
    }
}
