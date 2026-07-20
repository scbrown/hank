//! The Hank MCP server and its tools.
//!
//! Registration mirrors Bobbin: a `#[tool_router]` impl of `#[tool]`-annotated
//! async methods taking `Parameters<Req>`, a `#[tool_handler] ServerHandler`
//! providing `get_info`, and stdio + streamable-HTTP transports.

use std::path::PathBuf;

use anyhow::Result;
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{
    CallToolResult, Content, Implementation, ProtocolVersion, ServerCapabilities, ServerInfo,
};
use rmcp::{tool, tool_handler, tool_router, ErrorData as McpError, ServerHandler};
use serde::Serialize;

use std::collections::BTreeSet;

use super::tools::{
    AnalyzeRequest, AnalyzeResponse, CommunitiesRequest, CommunitiesResponse, CommunityItem,
    CommunityMemberItem, DataflowRequest, DataflowResponse, DepEdgeItem, FlowStepItem,
    ImpactRequest, ImpactResponse, NeighborsRequest, NeighborsResponse, ReachItem,
    ReconciliationItem, RefItem, ReferencesRequest, ReferencesResponse, StatusResponse, SymbolItem,
    SymbolsRequest, SymbolsResponse, VerifyRequest, VerifyResponse, ViolationItem,
};
use crate::config::HankConfig;
use crate::dataflow::{Dataflow, FlowDir};
use crate::extract::{extract_symbols, rust_files};
use crate::graph::{CodeGraph, Dir, Reached};
use crate::reconcile::reconcile;

/// Hank's MCP server. Resolves requests against the analysis root for a tenant.
#[derive(Clone)]
pub struct HankMcpServer {
    root: PathBuf,
    tenant: Option<String>,
    tool_router: ToolRouter<Self>,
}

impl HankMcpServer {
    /// Construct a server rooted at `root` for an optional `tenant`.
    #[must_use]
    pub fn new(root: PathBuf, tenant: Option<String>) -> Self {
        Self {
            root,
            tenant,
            tool_router: Self::tool_router(),
        }
    }
}

#[tool_router]
impl HankMcpServer {
    #[tool(
        description = "Show Hank's base ref, tenant, available extraction tiers, and Quipu promotion settings."
    )]
    async fn hank_status(&self) -> Result<CallToolResult, McpError> {
        let config = HankConfig::load(&self.root).map_err(internal)?;
        let response = StatusResponse {
            base_ref: config.base_ref,
            tenant: self
                .tenant
                .clone()
                .unwrap_or_else(|| "(single-tenant)".to_string()),
            tiers: tier_availability(),
            quipu_enabled: config.quipu.enabled,
            branch_model: config.quipu.branch_model,
        };
        json_result(&response)
    }

    #[tool(
        description = "List the symbols (functions, structs, traits, ...) defined in one file. Each symbol carries a tier tag. Best for: 'what's defined in src/auth.rs?'."
    )]
    async fn hank_symbols(
        &self,
        Parameters(req): Parameters<SymbolsRequest>,
    ) -> Result<CallToolResult, McpError> {
        let file = self.root.join(&req.file);
        let source = std::fs::read_to_string(&file).map_err(internal)?;
        let symbols = extract_symbols(&source, "rust").map_err(internal)?;
        let response = SymbolsResponse {
            file: req.file.clone(),
            count: symbols.len(),
            symbols: symbols
                .iter()
                .map(|symbol| SymbolItem {
                    name: symbol.name.clone(),
                    kind: symbol.kind.as_str().to_string(),
                    start_line: symbol.start_line,
                    end_line: symbol.end_line,
                    tier: symbol.tier.as_str().to_string(),
                })
                .collect(),
        };
        json_result(&response)
    }

    #[tool(
        description = "Find the definition site(s) of a symbol by name across a subtree. Best for: 'where is authenticate defined?'."
    )]
    async fn hank_references(
        &self,
        Parameters(req): Parameters<ReferencesRequest>,
    ) -> Result<CallToolResult, McpError> {
        let base = req
            .path
            .as_ref()
            .map_or_else(|| self.root.clone(), |p| self.root.join(p));
        let mut definitions = Vec::new();
        for file in rust_files(&base) {
            let Ok(source) = std::fs::read_to_string(&file) else {
                continue;
            };
            let Ok(symbols) = extract_symbols(&source, "rust") else {
                continue;
            };
            for symbol in symbols {
                if symbol.name == req.symbol {
                    let rel = file.strip_prefix(&self.root).unwrap_or(&file);
                    definitions.push(RefItem {
                        file: rel.display().to_string(),
                        name: symbol.name,
                        kind: symbol.kind.as_str().to_string(),
                        start_line: symbol.start_line,
                        tier: symbol.tier.as_str().to_string(),
                    });
                }
            }
        }
        let response = ReferencesResponse {
            symbol: req.symbol.clone(),
            count: definitions.len(),
            definitions,
        };
        json_result(&response)
    }

    #[tool(
        description = "Summarize the structure of a subtree: how many files and symbols. Best for a quick health check of the base graph."
    )]
    async fn hank_analyze(
        &self,
        Parameters(req): Parameters<AnalyzeRequest>,
    ) -> Result<CallToolResult, McpError> {
        let base = req
            .path
            .as_ref()
            .map_or_else(|| self.root.clone(), |p| self.root.join(p));
        let mut files = 0usize;
        let mut symbols = 0usize;
        for file in rust_files(&base) {
            let Ok(source) = std::fs::read_to_string(&file) else {
                continue;
            };
            if let Ok(found) = extract_symbols(&source, "rust") {
                files += 1;
                symbols += found.len();
            }
        }
        let response = AnalyzeResponse {
            files,
            symbols,
            tier: "treesitter".to_string(),
        };
        json_result(&response)
    }

    #[tool(
        description = "List the direct callers of a symbol (who calls it). Best for: 'who calls authenticate?'."
    )]
    async fn hank_callers(
        &self,
        Parameters(req): Parameters<NeighborsRequest>,
    ) -> Result<CallToolResult, McpError> {
        self.neighbors(&req, Dir::Callers)
    }

    #[tool(
        description = "List the direct callees of a symbol (what it calls). Best for: 'what does authenticate call?'."
    )]
    async fn hank_callees(
        &self,
        Parameters(req): Parameters<NeighborsRequest>,
    ) -> Result<CallToolResult, McpError> {
        self.neighbors(&req, Dir::Callees)
    }

    #[tool(
        description = "Blast radius: the symbols transitively affected by changing a symbol (its callers, up to N hops). Best for: 'what breaks if I change authenticate?'."
    )]
    async fn hank_impact(
        &self,
        Parameters(req): Parameters<ImpactRequest>,
    ) -> Result<CallToolResult, McpError> {
        let base = req
            .path
            .as_ref()
            .map_or_else(|| self.root.clone(), |p| self.root.join(p));
        let hops = req.hops.unwrap_or(5);
        let graph = CodeGraph::build(&base).map_err(internal)?;
        let found = graph.has_symbol(&req.symbol);
        let reachable = graph.reachable(&req.symbol, Dir::Callers, hops);
        let structural_files: BTreeSet<String> = reachable.iter().map(|r| r.file.clone()).collect();
        let reconciliation = req.cochange.as_ref().map(|cochange| {
            let cochange_set: BTreeSet<String> = cochange.iter().cloned().collect();
            let recon = reconcile(&structural_files, &cochange_set);
            ReconciliationItem {
                corroborated: recon.corroborated,
                structural_only: recon.structural_only,
                cochange_only: recon.cochange_only,
            }
        });
        let response = ImpactResponse {
            symbol: req.symbol.clone(),
            found,
            hops,
            count: reachable.len(),
            reachable: reachable.iter().map(reach_item).collect(),
            structural_files: structural_files.into_iter().collect(),
            reconciliation,
        };
        json_result(&response)
    }

    #[tool(
        description = "Detect communities: densely-connected clusters of symbols in the call graph (deterministic Louvain). Best for: 'what are the natural modules/subsystems here?'."
    )]
    async fn hank_communities(
        &self,
        Parameters(req): Parameters<CommunitiesRequest>,
    ) -> Result<CallToolResult, McpError> {
        let base = req
            .path
            .as_ref()
            .map_or_else(|| self.root.clone(), |p| self.root.join(p));
        let graph = CodeGraph::build(&base).map_err(internal)?;
        let comms = graph.communities();
        let communities = comms
            .iter()
            .map(|c| CommunityItem {
                id: c.id,
                size: c.members.len(),
                members: c
                    .members
                    .iter()
                    .map(|m| CommunityMemberItem {
                        name: m.name.clone(),
                        kind: m.kind.clone(),
                        file: m.file.clone(),
                        start_line: m.start_line,
                        tier: m.tier.as_str().to_string(),
                    })
                    .collect(),
            })
            .collect();
        let response = CommunitiesResponse {
            count: comms.len(),
            communities,
            tier: "treesitter".to_string(),
        };
        json_result(&response)
    }

    #[tool(
        description = "Verify a PROPOSED edit buffer before you write it: returns a boolean verdict plus violations (identifier-does-not-exist, wrong-arity, unresolved-import). Best for: 'will this edit break something?'. Note the `unchecked` list — a true verdict is not a compile."
    )]
    async fn hank_verify(
        &self,
        Parameters(req): Parameters<VerifyRequest>,
    ) -> Result<CallToolResult, McpError> {
        let file = self.root.join(&req.file);
        // The file's current contents are the baseline, so violations that
        // already exist are not blamed on the proposed edit.
        let baseline = std::fs::read_to_string(&file).ok();
        let verdict =
            crate::verify::verify_buffer(&self.root, &file, &req.buffer, baseline.as_deref())
                .map_err(internal)?;

        let response = VerifyResponse {
            file: req.file,
            ok: verdict.ok,
            violations: verdict
                .violations
                .iter()
                .map(|v| ViolationItem {
                    kind: serde_json::to_value(v.kind)
                        .ok()
                        .and_then(|k| k.as_str().map(str::to_string))
                        .unwrap_or_default(),
                    symbol: v.symbol.clone(),
                    line: v.line,
                    message: v.message.clone(),
                })
                .collect(),
            unchecked: verdict.unchecked,
            tier: "treesitter".to_string(),
        };
        json_result(&response)
    }

    #[tool(
        description = "Intra-procedural data dependence within a function. With `var`, trace what it depends on (or, with forward=true, what it flows into); without `var`, list all dependence edges. Best for: 'where does this value come from?'."
    )]
    async fn hank_dataflow(
        &self,
        Parameters(req): Parameters<DataflowRequest>,
    ) -> Result<CallToolResult, McpError> {
        let base = req
            .path
            .as_ref()
            .map_or_else(|| self.root.clone(), |p| self.root.join(p));
        let flow = Dataflow::build(&base).map_err(internal)?;
        let found = flow.has_function(&req.function);

        let (direction, steps, edges) = match &req.var {
            Some(var) => {
                let dir = if req.forward.unwrap_or(false) {
                    FlowDir::FlowsInto
                } else {
                    FlowDir::DependsOn
                };
                let steps = flow
                    .flow(&req.function, var, dir, req.hops.unwrap_or(5))
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
                    .edges(&req.function)
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

        let response = DataflowResponse {
            function: req.function.clone(),
            found,
            direction,
            var: req.var.clone(),
            flow: steps,
            edges,
        };
        json_result(&response)
    }
}

impl HankMcpServer {
    /// Shared body for `hank_callers` / `hank_callees`.
    fn neighbors(&self, req: &NeighborsRequest, dir: Dir) -> Result<CallToolResult, McpError> {
        let base = req
            .path
            .as_ref()
            .map_or_else(|| self.root.clone(), |p| self.root.join(p));
        let graph = CodeGraph::build(&base).map_err(internal)?;
        let found = graph.has_symbol(&req.symbol);
        let neighbors = graph.direct(&req.symbol, dir);
        let response = NeighborsResponse {
            symbol: req.symbol.clone(),
            found,
            count: neighbors.len(),
            neighbors: neighbors.iter().map(reach_item).collect(),
        };
        json_result(&response)
    }
}

#[tool_handler]
impl ServerHandler for HankMcpServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            protocol_version: ProtocolVersion::V_2024_11_05,
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            server_info: Implementation {
                name: "hank".to_string(),
                title: Some("Hank Code Structure".to_string()),
                version: env!("CARGO_PKG_VERSION").to_string(),
                icons: None,
                website_url: None,
            },
            instructions: Some(
                "Hank serves live, per-tenant code structure. Use hank_symbols to list a \
                 file's symbols, hank_references to find where a symbol is defined, \
                 hank_analyze for a subtree summary, and hank_status for base ref and tiers. \
                 Every fact is tagged with its tier (treesitter/lsp/cpg)."
                    .to_string(),
            ),
        }
    }
}

/// Serialize a response into a successful tool result.
fn json_result<T: Serialize>(value: &T) -> Result<CallToolResult, McpError> {
    let json = serde_json::to_string_pretty(value).map_err(internal)?;
    Ok(CallToolResult::success(vec![Content::text(json)]))
}

/// Map any error into an MCP internal error.
fn internal<E: std::fmt::Display>(err: E) -> McpError {
    McpError::internal_error(err.to_string(), None)
}

/// Convert a graph `Reached` into the wire DTO.
fn reach_item(reached: &Reached) -> ReachItem {
    ReachItem {
        name: reached.name.clone(),
        file: reached.file.clone(),
        start_line: reached.start_line,
        distance: reached.distance,
        via: reached.via.to_string(),
    }
}

/// The extraction tiers this build can serve.
fn tier_availability() -> Vec<String> {
    let mut tiers = vec!["treesitter".to_string()];
    if cfg!(feature = "lsp") {
        tiers.push("lsp".to_string());
    }
    if cfg!(feature = "cpg") {
        tiers.push("cpg".to_string());
    }
    tiers
}
