//! The daemon's liveness surface — stage 1 only (aegis-1qze).
//!
//! Two routes: `/health` (a bare 200 for the [`super::client`] probe) and
//! `/status` (the resident graph's real counts). The query endpoints
//! (refs/definition/callers/callees/impact/dataflow/symbols) are stage 2 and land
//! in `src/http/` per hank #1; this module is intentionally just enough to make
//! the resident process observable and reachable, so stage 1 is testable and
//! mergeable on its own.

use axum::extract::State;
use axum::routing::get;
use axum::{Json, Router};

use super::{EngineStatus, ResidentEngine};

/// Build the stage-1 router over a resident engine.
pub fn router(engine: ResidentEngine) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/status", get(status))
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
        let body = get_status_json(port).await;
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

    // Minimal HTTP GET of /status without pulling a client dep into the crate:
    // reuse the probe's raw-socket approach but read the whole body as JSON.
    async fn get_status_json(port: u16) -> serde_json::Value {
        tokio::task::spawn_blocking(move || {
            use std::io::{Read, Write};
            let mut s = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
            s.write_all(b"GET /status HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n")
                .unwrap();
            let mut raw = String::new();
            s.read_to_string(&mut raw).unwrap();
            let body = raw.split("\r\n\r\n").nth(1).unwrap_or("").to_string();
            serde_json::from_str(&body).unwrap()
        })
        .await
        .unwrap()
    }
}
