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

## What works today

`analyze`, `refs`, and `status` do real tree-sitter extraction. `serve`,
`callers`, `impact`, `verify`, and `promote` are declared with their final shape
and print a phase notice until their engines land — see the
[Specification](../design/specification.md).
