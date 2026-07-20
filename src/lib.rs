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
mod cli_cmds;
pub mod community;
pub mod config;
pub mod dataflow;
pub mod docref;
pub mod errors;
pub mod export;
pub mod extract;
pub mod git;
pub mod graph;
pub mod hook;
#[cfg(feature = "mcp")]
pub mod mcp;
pub mod policy;
pub mod reconcile;
mod render;
pub mod types;
pub mod verify;
pub mod watch;

pub use errors::{Error, Result};
