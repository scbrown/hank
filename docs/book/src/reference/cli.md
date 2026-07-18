# CLI Commands

```text
USAGE:
    hank <COMMAND>

COMMANDS:
    serve       Run the MCP server (stdio; --http for streamable-HTTP)
    analyze     Build the base graph for a path and print a summary
    refs        Find the definition sites of a symbol by name
    callers     Direct callers and callees of a symbol
    impact      Blast radius: symbols affected by changing a symbol
    dataflow    Intra-procedural data dependence within a function
    verify      Verdict on a proposed edit buffer                 [Phase 5]
    promote     Promote a commit's structural facts into Quipu    [Phase 4]
    status      Show base commit, tiers, and configuration
    completions Generate shell completions
    help        Print help

GLOBAL FLAGS:
    --json      Machine-readable output
    --quiet     Suppress non-essential output
    --verbose   Detailed progress
    --tenant    Tenant/session id (default: single-tenant)
    --config    Path to config file
```

## Examples

```bash
hank analyze src
hank refs authenticate src --json
hank status
hank impact src/auth.rs::authenticate --hops 5
hank verify --file src/auth.rs --buffer /tmp/edited.rs
hank promote --commit HEAD
```

Commands marked with a phase print a notice until their engine lands; see the
[Specification](../design/specification.md) §12.
