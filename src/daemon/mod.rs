//! The resident engine — Phase 3, stage 1 (FR-31, hank #1 / aegis-1qze).
//!
//! Today the hook and one-shot commands build the whole `CodeGraph` transiently,
//! per invocation. FR-31 makes a resident process that holds the base graph in
//! memory the foundation for the sub-100ms guard budget and for per-tenant
//! overlays. This module is that process's core: it builds the graph ONCE, holds
//! it, and exposes a liveness/status surface (stage 1) plus graph-backed query
//! endpoints (stage 2). The hook/MCP thin-client cutover is stage 3. Landing in stages is
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

use serde::{Deserialize, Serialize};

use crate::config::HankConfig;
use crate::graph::{CodeGraph, Dir, Reached};
use crate::hook::Sizing;
use crate::policy::PolicyConfig;
use crate::types::Tier;

pub mod client;
#[cfg(feature = "mcp")]
pub(crate) mod http;

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

    /// The analysis root the resident graph was built from. Used to confine the
    /// `/measure` endpoint to files under this root, and to check a client is
    /// talking to a daemon serving the repo it means to measure.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.inner.root
    }

    /// The resident policy, from the config snapshot taken at build time. The
    /// aegis-hac0 signed cache will verify the source of this at load; callers
    /// read it from here rather than re-reading config from disk per request.
    #[must_use]
    pub fn policy(&self) -> &PolicyConfig {
        &self.inner.config.policy
    }

    /// Direct callers or callees of `symbol`, from the RESIDENT graph — no
    /// per-call rebuild, which is the daemon's whole point. This is the shared
    /// query layer: the HTTP surface (stage 2) calls it now, and the hook/MCP thin
    /// clients (stage 3) will call the same method instead of building transiently.
    #[must_use]
    pub fn neighbors(&self, symbol: &str, dir: Dir) -> Neighbors {
        let graph = self.graph();
        Neighbors {
            symbol: symbol.to_string(),
            found: graph.has_symbol(symbol),
            neighbors: graph.direct(symbol, dir).iter().map(reached_item).collect(),
            tier: graph_tier(),
        }
    }

    /// Blast radius: symbols transitively affected by changing `symbol`, up to
    /// `hops`. Resident-graph, no rebuild.
    #[must_use]
    pub fn impact(&self, symbol: &str, hops: u32) -> Impact {
        let graph = self.graph();
        let reachable = graph.reachable(symbol, Dir::Callers, hops);
        let files: std::collections::BTreeSet<String> =
            reachable.iter().map(|r| r.file.clone()).collect();
        Impact {
            symbol: symbol.to_string(),
            found: graph.has_symbol(symbol),
            hops,
            count: reachable.len(),
            reachable: reachable.iter().map(reached_item).collect(),
            files: files.into_iter().collect(),
            tier: graph_tier(),
        }
    }

    /// Size an edit against the RESIDENT graph — the exact question the pre-edit
    /// guard asks, answered without the per-invocation `CodeGraph::build`. The
    /// edited file is still read fresh (its content is what changed), so the answer
    /// matches the transient path on the same tree; only the graph build is saved.
    /// This is what the hook becomes a thin client of in the cutover (stage 3b).
    #[must_use]
    pub fn measure_edit(
        &self,
        file: &Path,
        rel: &str,
        anchors: &[String],
        max_hops: u32,
    ) -> Sizing {
        crate::hook::measure_with_graph(self.graph(), file, rel, anchors, max_hops)
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

/// The provenance tier of every reachability fact the resident graph serves.
/// One source of truth for the string, mirroring `mcp::server::graph_tier`; the
/// place to propagate a real per-node tier from when the LSP/CPG tiers land.
fn graph_tier() -> String {
    Tier::TreeSitter.as_str().to_string()
}

/// One reached symbol in a neighbors/impact reply. Owned + `Serialize` so the
/// daemon layer does not depend on the `mcp` feature's response types, and
/// `Deserialize` so a thin client (the MCP cutover, stage 3c) parses the same
/// type off the wire that the daemon serialized — no shadow DTO to drift.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReachedItem {
    /// Symbol name.
    pub name: String,
    /// File, relative to the analysis root.
    pub file: String,
    /// 1-based definition line.
    pub start_line: usize,
    /// Hop distance from the seed (1 = direct).
    pub distance: u32,
    /// Relationship to the seed (`calls` or `called_by`). Owned (not
    /// `&'static str`) so the type round-trips through a client's parse.
    pub via: String,
}

fn reached_item(r: &Reached) -> ReachedItem {
    ReachedItem {
        name: r.name.clone(),
        file: r.file.clone(),
        start_line: r.start_line,
        distance: r.distance,
        via: r.via.to_string(),
    }
}

/// Reply for `/callers` and `/callees`. `found` is separate from an empty
/// `neighbors` on purpose: "the symbol is not in the graph" and "the symbol has
/// no callers" are different answers, and collapsing them is the fact-vs-absence
/// bug this project keeps paying for.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Neighbors {
    /// The queried symbol.
    pub symbol: String,
    /// Whether the symbol exists in the resident graph at all.
    pub found: bool,
    /// Direct neighbors in the requested direction.
    pub neighbors: Vec<ReachedItem>,
    /// Provenance tier of these facts.
    pub tier: String,
}

/// Reply for `/impact` — the transitive blast radius of changing a symbol.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Impact {
    /// The queried symbol.
    pub symbol: String,
    /// Whether the symbol exists in the resident graph at all.
    pub found: bool,
    /// Hops followed.
    pub hops: u32,
    /// Number of transitively affected symbols.
    pub count: usize,
    /// The affected symbols (callers).
    pub reachable: Vec<ReachedItem>,
    /// Distinct files in the impact set.
    pub files: Vec<String>,
    /// Provenance tier of these facts.
    pub tier: String,
}

/// The wire form of a [`Sizing`] — what `/measure` returns and a thin-client hook
/// parses. `measured` is separate from a zero radius on purpose: an UNMEASURED edit
/// (no grammar, unreadable, deadline) is NOT a radius of zero, and the client must
/// treat it as "not evaluated", never "within limits" (the fail-open/loud contract).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MeasureReply {
    /// Whether a blast radius was actually computed.
    pub measured: bool,
    /// Symbols transitively affected (0 when not measured).
    pub symbols: usize,
    /// Files transitively affected (0 when not measured).
    pub files: usize,
    /// The `Sizing` variant tag: `measured`, `deadline`, `no-grammar`, …. Lets the
    /// client key its once-per-session loud notice by kind, exactly as the in-process
    /// guard does.
    pub kind: String,
    /// The operator-facing reason it was not measured; `None` when measured.
    pub reason: Option<String>,
}

impl MeasureReply {
    /// Map a `Sizing` to its wire form. A measured radius carries its counts; every
    /// unmeasured variant carries its kind tag and reason and a zero radius that the
    /// `measured: false` flag forbids reading as "within limits".
    #[must_use]
    pub fn from_sizing(sizing: &Sizing) -> Self {
        match sizing {
            Sizing::Measured(radius) => Self {
                measured: true,
                symbols: radius.symbols,
                files: radius.files,
                kind: "measured".to_string(),
                reason: None,
            },
            other => Self {
                measured: false,
                symbols: 0,
                files: 0,
                kind: other.kind_tag().to_string(),
                reason: other.unmeasured_reason(),
            },
        }
    }
}

/// Build the resident engine and serve its liveness surface on `bind`.
///
/// Serves `/health`, `/status` (stage 1) and the graph-backed query endpoints
/// `/callers`, `/callees`, `/impact` (stage 2). Runs until the process is signalled.
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

#[cfg(test)]
mod tests {
    use super::*;

    // `leaf` is called by `caller`, which is called by `top` — a 2-hop chain so
    // impact can be tested past a single hop.
    fn chain_repo() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("leaf.rs"), "fn leaf() {}\n").unwrap();
        std::fs::write(dir.path().join("mid.rs"), "fn caller() { leaf(); }\n").unwrap();
        std::fs::write(dir.path().join("top.rs"), "fn top() { caller(); }\n").unwrap();
        dir
    }

    #[test]
    fn callers_of_a_known_symbol_come_from_the_resident_graph() {
        let dir = chain_repo();
        let engine = ResidentEngine::build(dir.path(), None).unwrap();
        let n = engine.neighbors("leaf", Dir::Callers);
        assert!(n.found, "leaf is in the graph");
        assert!(
            n.neighbors.iter().any(|r| r.name == "caller"),
            "leaf's direct caller is `caller`, got {:?}",
            n.neighbors
        );
        assert_eq!(n.tier, "treesitter");
    }

    #[test]
    fn an_unknown_symbol_is_NOT_FOUND_distinct_from_no_neighbors() {
        // found=false and an empty list are different answers; a symbol that IS in
        // the graph but has no callers would be found=true + empty.
        let dir = chain_repo();
        let engine = ResidentEngine::build(dir.path(), None).unwrap();
        let missing = engine.neighbors("does_not_exist", Dir::Callers);
        assert!(!missing.found);
        assert!(missing.neighbors.is_empty());

        let top = engine.neighbors("top", Dir::Callers);
        assert!(top.found, "top exists");
        assert!(top.neighbors.is_empty(), "nothing calls top");
    }

    #[test]
    fn measure_edit_sizes_against_the_resident_graph() {
        // The exact question the pre-edit guard asks, answered from the resident
        // graph. Editing `leaf` reaches `caller` (mid.rs) and `top` (top.rs) — two
        // symbols across two files; the edited file itself is excluded. `measure_edit`
        // shares `edit_touch` + `walk_blast` with the transient `measure`, differing
        // only in graph source, so this radius is the transient path's radius too.
        let dir = chain_repo();
        let engine = ResidentEngine::build(dir.path(), None).unwrap();
        let file = dir.path().join("leaf.rs");
        let sizing = engine.measure_edit(&file, "leaf.rs", &["fn leaf".to_string()], 5);
        match sizing {
            Sizing::Measured(radius) => {
                assert_eq!(radius.symbols, 2, "leaf reaches caller and top");
                assert_eq!(radius.files, 2, "in mid.rs and top.rs");
            }
            other => panic!("expected a measured radius, got {other:?}"),
        }
    }

    #[test]
    fn measure_edit_reports_UNMEASURED_for_an_unparseable_file_never_a_silent_zero() {
        // The fail-open/loud contract flows through unchanged: a file the graph
        // cannot parse is NOT a radius of zero (which would read as "within limits").
        let dir = chain_repo();
        std::fs::write(dir.path().join("notes.md"), "# hi\n").unwrap();
        let engine = ResidentEngine::build(dir.path(), None).unwrap();
        let file = dir.path().join("notes.md");
        let sizing = engine.measure_edit(&file, "notes.md", &[], 5);
        assert!(
            !matches!(sizing, Sizing::Measured(_)),
            "an unparseable file must be UNMEASURED, not a measured zero: {sizing:?}"
        );
    }

    #[test]
    fn impact_follows_the_chain_transitively() {
        let dir = chain_repo();
        let engine = ResidentEngine::build(dir.path(), None).unwrap();
        let imp = engine.impact("leaf", 5);
        assert!(imp.found);
        // Changing leaf transitively affects caller AND top.
        let names: Vec<&str> = imp.reachable.iter().map(|r| r.name.as_str()).collect();
        assert!(names.contains(&"caller"), "got {names:?}");
        assert!(names.contains(&"top"), "got {names:?}");
    }

    #[test]
    fn a_resident_impact_query_is_far_under_the_SLO() {
        // The daemon's reason for being: the query runs against the RESIDENT graph,
        // with NO rebuild. hank #1's SLO is blast-radius 5 hops < 300ms p95. Against
        // a resident graph a single query is microseconds; this pins that the query
        // path itself carries no rebuild cost. (Build time is paid once, at startup,
        // and is excluded here on purpose — that is exactly what the daemon moves off
        // the per-query path.)
        let dir = chain_repo();
        let engine = ResidentEngine::build(dir.path(), None).unwrap();
        let start = std::time::Instant::now();
        for _ in 0..100 {
            let _ = engine.impact("leaf", 5);
        }
        let per_query = start.elapsed() / 100;
        assert!(
            per_query < std::time::Duration::from_millis(50),
            "a resident-graph impact query took {per_query:?} — the SLO win is that \
             this path has no rebuild cost"
        );
    }
}
