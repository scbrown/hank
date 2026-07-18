//! Hank — an in-memory, multi-tenant code-analysis engine.
//!
//! Hank extracts precise structure from a codebase (AST, symbols, call graph,
//! and — in later phases — control/data dependence and LSP facts), keeps it hot
//! in memory, and serves it per tenant so a whole team can edit concurrently
//! without corrupting each other's view. It is the third peer in the
//! Bobbin × Hank × Quipu stack; see `docs/hank-spec.md` for the full design.
//!
//! This crate is an early Phase-1 skeleton: tree-sitter structural extraction,
//! a config model, a typed fact model, and a CLI. The MCP/HTTP serving layer
//! (`mcp` feature), CPG/dataflow (`cpg`), LSP precision (`lsp`), and Quipu
//! promotion (`quipu`) land in subsequent phases.

pub mod cli;
pub mod config;
pub mod errors;
pub mod extract;
pub mod graph;
#[cfg(feature = "mcp")]
pub mod mcp;
mod render;
pub mod types;

pub use errors::{Error, Result};
