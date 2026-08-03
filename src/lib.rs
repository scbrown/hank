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

pub mod action;
pub mod audit;
pub mod change;
pub mod cli;
mod cli_cmds;
pub mod community;
pub mod config;
pub mod constraint;
pub mod daemon;
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
pub mod metrics;
pub mod plate;
pub mod policy;
/// Phase-4 projection: a hot, one-directional cache of quipu's structural policies.
#[cfg(feature = "quipu")]
pub mod project;
/// The SPARQL `project` sends. Gated with it: queries for a plane this build
/// does not have are dead weight, and an ungated half of a gated feature is how
/// a `--features` matrix starts lying about what a binary can do.
#[cfg(feature = "quipu")]
pub mod project_decode;
pub mod project_queries;
/// Phase-4 Quipu promotion: SHACL-validate a Turtle projection, then write it.
#[cfg(feature = "quipu")]
pub mod promote;
pub mod reconcile;
mod render;
pub mod rules;
pub mod textrules;
pub mod throttle;
pub mod trace;
pub mod types;
/// Phase-4 verdict signing + promotion (H-PROMOTE-VERDICT).
#[cfg(feature = "quipu")]
pub mod verdict;
#[cfg(feature = "quipu")]
pub mod verdict_spool;
pub mod verify;
pub mod watch;

pub use errors::{Error, Result};
