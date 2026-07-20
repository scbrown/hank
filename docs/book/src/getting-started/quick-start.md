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

## Call graph and blast radius

```bash
# Direct callers and callees of a symbol
hank callers authenticate src

# Blast radius: what changing a symbol transitively affects
hank impact authenticate src --hops 5 --json

# Reconcile the structural impact against a co-change set (from Bobbin):
# corroborated = real coupling; co-change-only = possible refactoring smell
hank impact authenticate src --cochange cochange.json

# Intra-procedural data dependence: where a value comes from / flows to
hank dataflow authenticate src --var token           # what `token` depends on
hank dataflow authenticate src --var token --forward # what `token` flows into

# Export the referential structure (modules, symbols, calls, imports) as governed RDF
hank export src --repo myrepo --format turtle
```

The export is the governed projection of the live graph — precise, typed
referential structure in the `bobbin:` code ontology, **not** embedding chunks.
It is the substrate under Phase-4 promotion into Quipu; see the
[Specification](../design/specification.md) §5.10 and §9.

## The MCP server

Built with the `mcp` feature, `hank serve` exposes ten `hank_*` tools over MCP —
starting with `hank_status`, `hank_symbols`, `hank_references`, and `hank_analyze`
(the full set is in the [MCP Tools reference](../reference/mcp-tools.md)):

```bash
cargo run --features mcp -- serve         # stdio, for a local agent
cargo run --features mcp -- serve --http  # streamable-HTTP at :3040/mcp
```

See the [MCP Tools reference](../reference/mcp-tools.md).

## What works today

`analyze`, `refs`, `status`, the call-graph commands `callers`/`impact`,
`dataflow`, `verify` (the FR-23/FR-24 edit-buffer verdict), and the ten MCP tools
are live. Only `promote` is still declared with its final shape and prints a phase
notice until its engine lands — see the
[Specification](../design/specification.md).
