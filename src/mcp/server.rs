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

use super::tools::{
    AnalyzeRequest, AnalyzeResponse, RefItem, ReferencesRequest, ReferencesResponse,
    StatusResponse, SymbolItem, SymbolsRequest, SymbolsResponse,
};
use crate::config::HankConfig;
use crate::extract::{extract_symbols, rust_files};

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

/// Serve over stdio (the default agent transport).
pub async fn run_stdio(root: PathBuf, tenant: Option<String>) -> Result<()> {
    use rmcp::transport::stdio;
    use rmcp::ServiceExt;

    let server = HankMcpServer::new(root, tenant);
    let service = server.serve(stdio()).await?;
    service.waiting().await?;
    Ok(())
}

/// Serve over streamable-HTTP (network-accessible; for the broker and remote agents).
pub async fn run_http(
    root: PathBuf,
    tenant: Option<String>,
    bind: String,
    port: u16,
) -> Result<()> {
    use std::sync::Arc;
    use std::time::Duration;

    use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
    use rmcp::transport::streamable_http_server::tower::StreamableHttpService;
    use rmcp::transport::StreamableHttpServerConfig;
    use tokio_util::sync::CancellationToken;

    let ct = CancellationToken::new();
    let service: StreamableHttpService<HankMcpServer, LocalSessionManager> =
        StreamableHttpService::new(
            move || Ok::<_, std::io::Error>(HankMcpServer::new(root.clone(), tenant.clone())),
            Arc::new(LocalSessionManager::default()),
            StreamableHttpServerConfig {
                stateful_mode: true,
                sse_keep_alive: Some(Duration::from_secs(15)),
                cancellation_token: ct.child_token(),
            },
        );

    let router = axum::Router::new().nest_service("/mcp", service);
    let addr = format!("{bind}:{port}");
    eprintln!("Hank MCP server listening on http://{addr}/mcp");

    let listener = tokio::net::TcpListener::bind(&addr).await?;
    axum::serve(listener, router)
        .with_graceful_shutdown(async move {
            tokio::signal::ctrl_c().await.ok();
            ct.cancel();
        })
        .await?;
    Ok(())
}
