# Quick Start

Build the binary, then point it at a source tree.

```bash
# Build the base graph for a directory and print a summary
hank analyze src

# Find the definition sites of a symbol by name
hank refs authenticate src

# Show the base ref, active tiers, and configuration
hank status

# Generate shell completions
hank completions bash > hank.bash
```

Every command accepts the global flags `--json`, `--quiet`, `--verbose`,
`--tenant <id>`, and `--config <path>`.

## The MCP server

Built with the `mcp` feature, `hank serve` exposes `hank_status`,
`hank_symbols`, `hank_references`, and `hank_analyze` over MCP:

```bash
cargo run --features mcp -- serve         # stdio, for a local agent
cargo run --features mcp -- serve --http  # streamable-HTTP at :3040/mcp
```

See the [MCP Tools reference](../reference/mcp-tools.md).

## What works today

`analyze`, `refs`, and `status` do real tree-sitter extraction, and the four
MCP tools above are live. `callers`, `impact`, `verify`, and `promote` are
declared with their final shape and print a phase notice until their engines
land — see the [Specification](../design/specification.md).
