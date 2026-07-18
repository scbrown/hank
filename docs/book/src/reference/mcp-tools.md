# MCP Tools

Hank exposes its capabilities as MCP tools (over stdio and streamable-HTTP) and
a parallel HTTP API. Tools use the `hank_*` naming convention, alongside
Bobbin's `bobbin_*` and Quipu's `quipu_*` on the same agent.

> **Status:** the MCP surface lands in Phase 1 behind the `mcp` feature. The
> table below is the planned surface (Specification §10).

| Tool | Purpose |
|------|---------|
| `hank_definition` | Definition site(s) of a symbol/position |
| `hank_references` | All reference sites of a symbol |
| `hank_callers` / `hank_callees` | Call-graph neighbors |
| `hank_dataflow` | Source→sink dataflow paths |
| `hank_impact` | Blast radius (forward/backward, N hops) |
| `hank_symbols` | Symbol tree for a file/module |
| `hank_verify` | Verdict on a proposed edit buffer |
| `hank_status` | Base commit, tenant overlays, tiers, freshness |
| `hank_promote` | Trigger promotion of a commit to Quipu (write-guarded) |

Every response carries `tier` and `freshness`; every structure-reading request
accepts a `tenant` parameter.
