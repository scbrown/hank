# MCP Tools

Hank exposes its capabilities as MCP tools (over stdio and streamable-HTTP) and
a parallel HTTP API. Tools use the `hank_*` naming convention, alongside
Bobbin's `bobbin_*` and Quipu's `quipu_*` on the same agent.

Build with the `mcp` feature and run `hank serve` (stdio) or `hank serve --http`
(streamable-HTTP on `[hank.serve] mcp_http_port`, default 3040).

```bash
cargo build --features mcp
hank serve            # stdio, for a local agent
hank serve --http     # streamable-HTTP at http://127.0.0.1:3040/mcp
```

## Live tools

| Tool | Purpose |
|------|---------|
| `hank_status` | Base ref, tenant, available tiers, Quipu settings |
| `hank_symbols` | Symbol tree for a file |
| `hank_references` | Definition site(s) of a symbol by name |
| `hank_analyze` | Files/symbols summary for a subtree |

## Planned (later phases)

| Tool | Purpose | Phase |
|------|---------|-------|
| `hank_callers` / `hank_callees` | Call-graph neighbors | 2 |
| `hank_dataflow` | Source→sink dataflow paths | 2 |
| `hank_impact` | Blast radius (forward/backward, N hops) | 2 |
| `hank_verify` | Verdict on a proposed edit buffer | 5 |
| `hank_promote` | Trigger promotion of a commit to Quipu | 4 |

Every response carries a `tier` tag; structure-reading requests will accept a
`tenant` parameter as multi-tenancy lands (Phase 3).
