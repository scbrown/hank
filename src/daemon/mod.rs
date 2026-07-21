//! The resident engine — Phase 3, stage 1 (FR-31, hank #1 / aegis-1qze).
//!
//! Today the hook and one-shot commands build the whole `CodeGraph` transiently,
//! per invocation. FR-31 makes a resident process that holds the base graph in
//! memory the foundation for the sub-100ms guard budget and for per-tenant
//! overlays. This module is that process's core: it builds the graph ONCE, holds
//! it, and (stage 1) exposes only a liveness/status surface. The query endpoints
//! are stage 2; the hook/MCP thin-client cutover is stage 3. Landing in stages is
//! deliberate — a half-built resident guard is a footgun (see below).
//!
//! ## Two invariants this stage exists to establish before any query lands
//!
//! 1. **Daemon-absent must be a DISTINCT, LOUD signal — never a silent allow.**
//!    Once the guard is a thin client (stage 3), a down daemon is the cheapest
//!    possible bypass: kill one process and every edit sails through. So the
//!    client seam ([`client`]) reports "not reachable" as its own variant that a
//!    caller cannot fold into a default — the compiler makes you handle it. This
//!    is built now, with the process, so the cutover cannot forget it.
//!
//! 2. **The resident policy state is loaded ONCE, at a single trust point.** The
//!    engine holds a config snapshot ([`ResidentEngine::policy`]) taken at build
//!    time, not re-read per request. That single load site is where the
//!    aegis-hac0 signed rule cache will verify-and-trust: sign/verify wraps this
//!    one boundary rather than being scattered across per-invocation disk reads.
//!    The seam is here; the signing is that issue's job, not this stage's.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;

use serde::Serialize;

use crate::config::HankConfig;
use crate::graph::CodeGraph;
use crate::policy::PolicyConfig;

pub mod client;
#[cfg(feature = "mcp")]
mod http;

/// The base graph plus its policy snapshot, built once and held for the process
/// lifetime. Cheap to clone (`Arc`), so the HTTP layer shares one instance.
#[derive(Clone)]
pub struct ResidentEngine {
    inner: Arc<Engine>,
}

struct Engine {
    root: PathBuf,
    graph: CodeGraph,
    /// The config resolved at startup. NOT re-read per request — this is the
    /// single trust point the aegis-hac0 signed cache will guard (see module docs).
    config: HankConfig,
    built_at: SystemTime,
    nodes: usize,
    edges: usize,
}

impl ResidentEngine {
    /// Build the base graph for `root` and hold it resident. Runs once, at
    /// startup; a failure here means the daemon refuses to start rather than
    /// serving a graph it could not build.
    ///
    /// `config_override` mirrors the `--config` flag so the daemon honours the
    /// same config resolution as every other entry point.
    pub fn build(root: &Path, config_override: Option<&Path>) -> anyhow::Result<Self> {
        let config = HankConfig::resolve(config_override, root)?;
        let graph = CodeGraph::build(root)?;
        let (nodes, edges) = graph.stats();
        Ok(Self {
            inner: Arc::new(Engine {
                root: root.to_path_buf(),
                graph,
                config,
                built_at: SystemTime::now(),
                nodes,
                edges,
            }),
        })
    }

    /// The resident graph. Query endpoints (stage 2) borrow this; nothing mutates
    /// it — a rebuild replaces the whole engine, it does not patch in place.
    #[must_use]
    pub fn graph(&self) -> &CodeGraph {
        &self.inner.graph
    }

    /// The resident policy, from the config snapshot taken at build time. The
    /// aegis-hac0 signed cache will verify the source of this at load; callers
    /// read it from here rather than re-reading config from disk per request.
    #[must_use]
    pub fn policy(&self) -> &PolicyConfig {
        &self.inner.config.policy
    }

    /// A machine-readable liveness/status snapshot — real facts about what is
    /// resident, so a probe distinguishes "up and holding a graph" from "up but
    /// empty" as well as from "not reachable at all" (the last is the client's
    /// job, in [`client`]).
    #[must_use]
    pub fn status(&self) -> EngineStatus {
        let uptime_secs = self.inner.built_at.elapsed().map_or(0, |d| d.as_secs());
        EngineStatus {
            status: "ok",
            root: self.inner.root.display().to_string(),
            nodes: self.inner.nodes,
            edges: self.inner.edges,
            uptime_secs,
            tier: crate::types::Tier::served(),
        }
    }
}

/// The status payload served at `/status` and returned by a successful probe.
/// `status: "ok"` is a constant liveness marker a client greps for; the counts
/// let an operator see the daemon is holding a real graph, not an empty one.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct EngineStatus {
    /// Constant `"ok"` — presence of a parseable status body with this field is
    /// the liveness signal.
    pub status: &'static str,
    /// The analysis root the resident graph was built from.
    pub root: String,
    /// Nodes (symbols) in the resident graph.
    pub nodes: usize,
    /// Edges (relations) in the resident graph.
    pub edges: usize,
    /// Seconds since the graph was built.
    pub uptime_secs: u64,
    /// Precision tiers this build actually serves.
    pub tier: Vec<String>,
}

/// Build the resident engine and serve its liveness surface on `bind`.
///
/// Stage 1 serves only `/health` and `/status`; the query endpoints are stage 2.
/// Runs until the process is signalled.
#[cfg(feature = "mcp")]
pub async fn serve(
    root: &Path,
    config_override: Option<&Path>,
    bind: &str,
    port: u16,
) -> anyhow::Result<()> {
    let engine = ResidentEngine::build(root, config_override)?;
    let status = engine.status();
    eprintln!(
        "hank daemon: resident graph built — {} nodes, {} edges from {}",
        status.nodes, status.edges, status.root
    );
    http::serve(engine, bind, port).await
}
