//! MCP (Model Context Protocol) server for Hank.
//!
//! Exposes Hank's live structural analysis to agents over both stdio and
//! streamable-HTTP, using `rmcp` — the same SDK and registration pattern Bobbin
//! uses. Tools follow the `hank_*` naming convention (see `docs/hank-spec.md`
//! §10). This module is gated behind the `mcp` feature.

mod resident;
mod server;
mod tools;
mod transport;

pub use transport::{run_http, run_stdio};
