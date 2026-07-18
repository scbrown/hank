//! Request/response DTOs for Hank's MCP tools.
//!
//! Requests derive `Deserialize + schemars::JsonSchema` (the schema is served to
//! clients); responses derive `Serialize + schemars::JsonSchema`. Every response
//! that carries facts includes the `tier` tag (FR-3).

use serde::{Deserialize, Serialize};

/// Request for `hank_symbols` — the symbol tree of one file.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct SymbolsRequest {
    /// File path (relative to the analysis root) to list symbols for.
    #[schemars(description = "File path relative to the root, e.g. 'src/main.rs'")]
    pub file: String,
}

/// Request for `hank_references` — definition sites of a symbol by name.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ReferencesRequest {
    /// The symbol name to locate.
    #[schemars(description = "Symbol name to locate, e.g. 'authenticate'")]
    pub symbol: String,

    /// Directory to search (relative to the root; defaults to the whole root).
    #[schemars(
        description = "Directory to search, relative to the root. Omit to search everything."
    )]
    pub path: Option<String>,
}

/// Request for `hank_analyze` — a structural summary of a subtree.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct AnalyzeRequest {
    /// Directory to analyze (relative to the root; defaults to the whole root).
    #[schemars(
        description = "Directory to analyze, relative to the root. Omit for the whole root."
    )]
    pub path: Option<String>,
}

/// One extracted symbol in a response.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct SymbolItem {
    /// The symbol's name.
    pub name: String,
    /// The kind of symbol (`function`, `struct`, ...).
    pub kind: String,
    /// 1-based start line.
    pub start_line: usize,
    /// 1-based end line.
    pub end_line: usize,
    /// Provenance tier (`treesitter`, `lsp`, `cpg`).
    pub tier: String,
}

/// Response for `hank_symbols`.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct SymbolsResponse {
    /// The file that was analyzed.
    pub file: String,
    /// Number of symbols found.
    pub count: usize,
    /// The extracted symbols.
    pub symbols: Vec<SymbolItem>,
}

/// One definition site in a `hank_references` response.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct RefItem {
    /// File the definition is in (relative to the root).
    pub file: String,
    /// The symbol's name.
    pub name: String,
    /// The kind of symbol.
    pub kind: String,
    /// 1-based start line.
    pub start_line: usize,
    /// Provenance tier.
    pub tier: String,
}

/// Response for `hank_references`.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct ReferencesResponse {
    /// The symbol that was searched for.
    pub symbol: String,
    /// Number of definition sites found.
    pub count: usize,
    /// The definition sites.
    pub definitions: Vec<RefItem>,
}

/// Response for `hank_analyze`.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct AnalyzeResponse {
    /// Number of files analyzed.
    pub files: usize,
    /// Total symbols found.
    pub symbols: usize,
    /// Provenance tier of the summary.
    pub tier: String,
}

/// Request for `hank_callers` / `hank_callees` — call-graph neighbors.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct NeighborsRequest {
    /// The symbol name.
    #[schemars(description = "Symbol name, e.g. 'authenticate'")]
    pub symbol: String,

    /// Directory to build the call graph over (relative to the root).
    #[schemars(description = "Directory relative to the root. Omit for the whole root.")]
    pub path: Option<String>,
}

/// Request for `hank_impact` — the blast radius of changing a symbol.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ImpactRequest {
    /// The seed symbol.
    #[schemars(description = "Seed symbol name")]
    pub symbol: String,

    /// Directory to build the call graph over (relative to the root).
    #[schemars(description = "Directory relative to the root. Omit for the whole root.")]
    pub path: Option<String>,

    /// Maximum hops to follow (default 5).
    #[schemars(description = "Maximum hops to follow (default 5)")]
    pub hops: Option<u32>,
}

/// One reached symbol in a call-graph response.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct ReachItem {
    /// Symbol name.
    pub name: String,
    /// File (relative to the root).
    pub file: String,
    /// 1-based definition line.
    pub start_line: usize,
    /// Hop distance from the seed.
    pub distance: u32,
    /// Relationship to the seed (`calls` / `called_by`).
    pub via: String,
}

/// Response for `hank_callers` / `hank_callees`.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct NeighborsResponse {
    /// The seed symbol.
    pub symbol: String,
    /// Whether the symbol exists in the graph.
    pub found: bool,
    /// Number of neighbors.
    pub count: usize,
    /// The direct neighbors.
    pub neighbors: Vec<ReachItem>,
}

/// Response for `hank_impact`.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct ImpactResponse {
    /// The seed symbol.
    pub symbol: String,
    /// Whether the symbol exists in the graph.
    pub found: bool,
    /// Maximum hops followed.
    pub hops: u32,
    /// Number of affected symbols.
    pub count: usize,
    /// The transitively affected symbols (callers).
    pub reachable: Vec<ReachItem>,
}

/// Response for `hank_status`.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct StatusResponse {
    /// Baseline ref the graph is built at.
    pub base_ref: String,
    /// Tenant/session id.
    pub tenant: String,
    /// Extraction tiers this build can serve.
    pub tiers: Vec<String>,
    /// Whether Quipu promotion is enabled.
    pub quipu_enabled: bool,
    /// The configured branch model.
    pub branch_model: String,
}
