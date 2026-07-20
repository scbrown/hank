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
    hook        Harness hook adapter (post-edit advisory / pre-edit guard)
    verify      Verdict on a proposed edit buffer (FR-23/FR-24)
    promote     Promote a commit's structural facts into Quipu    [Phase 4]
    status      Show base commit, tiers, and configuration
    completions Generate shell completions
    help        Print help

GLOBAL FLAGS:
    --json      Machine-readable output
    --quiet     Suppress non-essential output
    --verbose   Raise the default log level to debug (RUST_LOG still wins)
    --tenant    Tenant/session id (default: single-tenant)
    --config    Path to config file (replaces discovery; must exist)
```

`--config <path>` **replaces** config discovery rather than adding to it: it
loads exactly that file over the compiled defaults, and the ambient
`.bobbin/config.toml` is not consulted (FR-29 ranks a flag above project and
user config). A `--config` path that does not exist is a loud error, never a
silent fall-back to discovery — so pointing the pre-edit guard at a scope file
with a mistyped path fails visibly instead of quietly enforcing the wrong scope.

`--verbose` raises the default tracing level from `info` to `debug`. `RUST_LOG`,
when set, still wins and can target individual modules, so the precedence is
`RUST_LOG` > `--verbose` > the `info` default.

## Examples

```bash
hank analyze src
hank analyze src --at main     # structure of the tree at a baseline commit (FR-13)
hank refs authenticate src --json
hank status                    # resolves base_ref to a commit SHA (in a git repo)
hank impact src/auth.rs::authenticate --hops 5
hank communities src --json         # symbol clusters, largest-first
hank verify --file src/auth.rs --buffer /tmp/edited.rs
hank promote --commit HEAD
```

## `hank verify`

Checks a *proposed* buffer against the graph Hank already holds and returns a
boolean verdict plus violations (FR-23/FR-24). Exits **non-zero** when the buffer
has violations, so scripts and CI can gate on it.

```console
$ hank verify --file src/a.rs --buffer /tmp/proposed.rs
violations src/a.rs [TreeSitter]
  ghost:2 `ghost` is called here but is defined nowhere in this buffer or the
          project graph, and is not brought into scope by a `use`.
  takes_two:2 `takes_two` is called with 1 argument(s) but is defined at line 1
          taking 2.
```

Only violations the edit *introduces* are reported: the file's current contents
are the baseline, so pre-existing breakage is not blamed on this edit.

**Read the `unchecked` list before trusting a clean verdict.** At the tree-sitter
tier there is no type information and no name resolution, so:

| Violation (FR-23) | At this tier |
|---|---|
| `identifier-does-not-exist` | free calls only, and only ones the edit introduces |
| `wrong-arity` | free calls resolving to exactly one known definition |
| `unresolved-import` | bodiless `mod foo;` with no sibling file |
| `type-violation` | **not checked** — needs the LSP tier |

The bias is against false positives throughout: method calls, path-qualified
calls, imports, locals, closures, and function-typed parameters are all left
alone rather than guessed at. `ok: true` means "nothing this tier can see is
wrong", not "this compiles".

`hank status` resolves the configured `base_ref` (default `main`) to a concrete
commit via the system `git`; outside a git repository the base commit shows as
unresolved and Hank falls back to the working tree.

`hank analyze --at <ref>` builds the summary from the **git tree** at a baseline
commit (FR-13) rather than the working copy — the shared read-only base the
Phase-3 resident graph will hold. It reads blob content at the ref (never the
working tree), and degrades to an empty result outside a repo or for an
unresolved ref.

`hank communities` partitions the call graph into densely-connected symbol
clusters using deterministic Louvain (FR-9) — the same partition on every run,
no RNG. Communities are ordered largest-first; members carry a `tier` tag.
Quipu runs community detection over *committed* facts; Hank computes it live
over the hot graph.

Commands marked with a phase print a notice until their engine lands; see the
[Specification](../design/specification.md) §12.
