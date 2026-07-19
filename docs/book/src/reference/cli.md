# CLI Commands

```text
USAGE:
    hank <COMMAND>

COMMANDS:
    serve       Run the MCP server (stdio; --http for streamable-HTTP)
    analyze     Build the base graph for a path and print a summary
    refs        Find the definition sites of a symbol by name
    callers     Direct callers and callees of a symbol
    communities Densely-connected symbol clusters (deterministic Louvain, FR-9)
    impact      Blast radius; --cochange reconciles against history (FR-11)
    dataflow    Intra-procedural data dependence within a function
    export      Emit the referential structure as Turtle (bobbin: ontology)
    hook        Harness hook adapter (reads hook payload on stdin)
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
hank status                    # resolves base_ref to a commit SHA (in a git repo)
hank impact src/auth.rs::authenticate --hops 5
hank communities src --json         # symbol clusters, largest-first
hank verify --file src/auth.rs --buffer /tmp/edited.rs
hank promote --commit HEAD
```

`hank status` resolves the configured `base_ref` (default `main`) to a concrete
commit via the system `git`; outside a git repository the base commit shows as
unresolved and Hank falls back to the working tree.

`hank communities` partitions the call graph into densely-connected symbol
clusters using deterministic Louvain (FR-9) — the same partition on every run,
no RNG. Communities are ordered largest-first; members carry a `tier` tag.
Quipu runs community detection over *committed* facts; Hank computes it live
over the hot graph.

Commands marked with a phase print a notice until their engine lands; see the
[Specification](../design/specification.md) §12.
