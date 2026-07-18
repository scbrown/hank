# Hank — Product Requirements & Build Specification

**Version:** 0.1
**Status:** Draft
**Last Updated:** 2026-07-18
**Owning vision:** [`docs/vision.md`](./vision.md) — *Bobbin × Hank × Quipu: A Governed, Multi-Signal, Multi-Tenant Code Intelligence Layer (v0.2)*

---

## 1. Executive Summary

Hank is an **in-memory, multi-tenant code-analysis engine** written in Rust. It
extracts precise structure from a codebase — AST, symbols, call graph, control-
and data-dependence, and LSP-grade type/reference facts — keeps that structure
hot in memory, and serves it over MCP and a local HTTP API. It does so **per
tenant**, so an entire team can edit concurrently without corrupting each
other's view of the graph, using a **shared-base-plus-copy-on-write-overlay**
model in which *blast radius doubles as the incremental-update primitive*.

Hank is the third peer in an existing stack:

- **Bobbin** (`scbrown/bobbin`, v0.6.0) — the fusion/serving layer. Retrieval is
  LanceDB hybrid (vector + keyword) search; coupling is FP-Growth co-change
  mining over git history. Bobbin's mission is unchanged; it gains Hank's
  structural facts as a new signal to fuse and explain.
- **Quipu** (`scbrown/quipu`, v0.3.3) — the governed, bitemporal knowledge graph
  (RDF model over a SQLite EAVT fact log, SPARQL 1.1, SHACL via `rudof`). Quipu
  becomes the settled home for *committed* structural facts under a code
  ontology it already partially defines (`shapes/code-entities.ttl`).
- **Hank** (this spec) — new. Owns the language toolchains, holds the volatile
  per-tenant working graph, and feeds three consumers: Bobbin (fusion), Quipu
  (promotion on commit), and the Gas Town broker/Aegis (per-tenant blast radius
  as a trust boundary).

The north star, restated as an engineering contract: **Hank extracts and serves
live per-tenant structure; Quipu governs and versions the committed record;
Bobbin fuses everything and serves it.**

This document specifies what Hank must do (functional requirements), how well
(non-functional requirements), how it is built (architecture and technology
choices, matched to Bobbin and Quipu), how it integrates (MCP surface, config,
Quipu promotion), and in what order (phasing). It deliberately reconciles the
vision with what the two existing peers *actually* implement today — most
importantly, that Quipu is a **triple store, not a quad store**, so the vision's
"branches as named graphs" needs a concrete design (§9.4), not an assumption.

---

## 2. Problem Statement

### 2.1 What Bobbin cannot answer today

Bobbin answers *"what code is relevant to this?"* using two signals: embedding
similarity and statistical co-change. Both are excellent at surface plausibility
and historical correlation. Neither knows the **actual structure or semantics**
of the code — a call edge, a type, a dataflow path, a definition site. Bobbin
can tell you two files tend to change together; it cannot yet tell you *why*, or
whether the coupling is real or coincidental.

A co-change edge with no structural explanation is a refactoring smell; a
co-change edge backed by a dataflow path is real coupling. **No single signal
makes that distinction** — which is exactly the gap Hank fills.

### 2.2 Why this belongs in a new tool, not in Bobbin

1. **Toolchain quarantine.** LSP servers, tree-sitter grammars, and any
   CPG/dataflow machinery (potentially JVM-flavored, from Joern) are stateful,
   heavy, and must never link into Bobbin's retrieval path.
2. **Different lifecycle.** Bobbin's path is interactive and per-query. Hank's is
   incremental, event-driven, and on-edit — a hot resident graph updated by a
   file-watcher, not rebuilt per request.
3. **Three consumers, not one.** Hank's facts feed Quipu, Bobbin, *and* the
   broker's blast radius. Bury extraction inside Bobbin and the other two must
   route through Bobbin to get structural facts. As a peer, Hank serves all three
   directly.
4. **Precedent.** The stack is already decomposed into peers (Quipu, Aegis,
   polecat). The strongest recent project in this space (codebase-memory) chose
   the standalone-analyzer design deliberately.

### 2.3 The multi-tenant reality

A team means there is **no single "the AST."** Each developer sits at some
branch/commit **plus an uncommitted working delta**, and those deltas diverge.
Rebuilding the whole graph per developer is wasteful (most of it is identical);
sharing one mutable graph is wrong (A's experiment corrupts B's view). Any
credible code-intelligence layer for a *team* of agents and humans must solve
this, and none of the source tools (multilspy, Joern, codebase-memory) do.

### 2.4 The routing rule this implies

Bobbin is on the request path **only when fusion or ranking adds value.**
Multi-signal context retrieval goes through Bobbin. Single-signal, analysis-only
queries — edit verification, blast radius, live structure lookups — go **straight
to Hank**, and policy consumers like the broker read Hank directly. Verification
and blast radius skip Bobbin because there is only one signal, so there is
nothing to fuse.

The boundary is not dogmatic: the lightweight parsing Bobbin already does
(chunking for embeddings, git-history co-change) **stays in Bobbin**. Hank owns
the *heavy, precise, toolchain-bound* analysis, not all parsing.

---

## 3. Relationship to Bobbin and Quipu

| Concern | Bobbin (v0.6.0) | Quipu (v0.3.3) | Hank (this spec) |
|---|---|---|---|
| Mission | Fuse + serve context | Govern + version committed facts | Extract + serve live structure |
| State | Per-query, index on disk | Append-only bitemporal log | Hot in-memory, per-tenant |
| Freshness | Re-index on change | On commit/merge (promotion) | On save / debounced keystroke |
| Primary store | LanceDB (+ SQLite coupling) | SQLite EAVT triple log | In-memory graph (+ overlay cache) |
| Signals | Embeddings, co-change | Committed structure, time | Structure, semantics, dataflow |
| Interface | MCP + HTTP + CLI | MCP handlers + REST + CLI | MCP + HTTP + CLI |
| On request path? | When fusion helps | For governed/temporal queries | For single-signal analysis |

**Data flow (steady state):**

```text
        edit / save / file-watch
                 │
                 ▼
   ┌───────────────────────────┐        promote on commit/merge
   │           HANK            │ ───────────────────────────────► ┌──────────┐
   │  base graph + overlays    │        (SHACL-validated Turtle)   │  QUIPU   │
   │  tree-sitter + LSP + CPG  │ ◄─────────────────────────────── │ EAVT log │
   └────────────┬──────────────┘        SPARQL over committed code │ SPARQL   │
                │                                                   └──────────┘
   structural   │ blast radius                 governed history
   facts +      │ (per tenant)                        ▲
   verdicts     │                                     │ fuse
                ▼                                      │
        ┌───────────────┐   broker/Aegis         ┌────┴─────┐
        │ Bobbin fusion │◄──── (trust boundary) ─│  agents  │
        │ + serving     │────────────────────────►│ (polecat)│
        └───────────────┘   explained context    └──────────┘
```

Where each stolen idea lives (from the vision, made concrete):

| Idea (source) | Home | Realized as |
|---|---|---|
| LSP defs/refs/types (*multilspy*) | Hank | §5.2 reference/definition resolution |
| CPG + dataflow/taint (*Joern*) | Hank builds → Quipu stores | §5.3 call graph + dataflow |
| Structural graph, community detection (*codebase-memory*) | Hank → Quipu → Bobbin | §5.3, §9, Bobbin fusion |
| Token-efficient structural recall | Bobbin over Hank/Quipu | Bobbin serves structure, not files |
| Convention/decision memory | Quipu | Quipu episodes (out of Hank scope) |
| Monitor-guided verification (*multilspy*) | Hank (served directly) | §5.7 edit verification |
| Blast-radius-as-trust-boundary | Broker/Aegis consumes; Hank computes | §5.4 + §5.9 |

---

## 4. User Personas

### Persona 1: Autonomous Coding Agent (polecat)

- **Needs:** provably-connected references (not "probably relevant"); blast
  radius before it edits; a boolean "will this edit compile / is this identifier
  real" check on its own proposed buffer.
- **Constraints:** limited context window; must not corrupt other tenants; must
  operate inside a capability sandbox scoped by *its own* tenant's live graph.

### Persona 2: Human Developer (direct + via Bobbin)

- **Needs:** "where is this defined / who references it" with ground truth; "what
  will this change break"; explained coupling ("these two files change together
  *because* of this dataflow path").
- **Constraints:** sits at a working copy with uncommitted edits; expects
  sub-second answers; does not want to stand up a language server per query.

### Persona 3: Bobbin (the fusion layer)

- **Needs:** structural facts with confidence/tier tags to fuse with co-change
  and embeddings; a way to flag retrieved code that will not compile in the
  current overlay.
- **Constraints:** async, per-query; consumes Hank as a signal source, not a
  dependency it must route others through.

### Persona 4: The Broker / Aegis (policy consumer)

- **Needs:** per-tenant blast radius to scope the provisioned execution
  environment for a polecat — autonomous edits safe *by construction*, not by
  review.
- **Constraints:** must read the *right* tenant's live graph, never a stale
  shared one.

---

## 5. Functional Requirements

Requirements are grouped by capability and tagged `FR-N`. Each capability maps to
a numbered capability from the vision (§"The concrete capability set").

### 5.1 Extraction engine (tree-sitter + LSP tiers)

**FR-1: Fast structural extraction (tree-sitter).**

- Parse source files with tree-sitter, reusing the exact grammar set Bobbin
  already ships: Rust, TypeScript/TSX, Python, Go, Java, C/C++.
- Extract a symbol tree (functions, methods, classes, structs, enums,
  interfaces, modules, fields, constants, type aliases) and intra-file call
  edges, with byte/line spans.
- Tree-sitter extraction is **always-on breadth**: it must work build-free, on a
  syntactically-broken buffer, incrementally (tree-sitter's incremental reparse).

**FR-2: Precise semantic extraction (LSP).**

- Run a language server per supported language behind one language-agnostic
  client interface (the multilspy idea), yielding defs, refs, types, hover,
  document/workspace symbols.
- LSP facts are **precision where a build exists**; they are computed on save or
  on-demand when a query needs them, never on every keystroke.
- Absence of a resolvable build must degrade to tree-sitter facts, not fail.

**FR-3: Confidence / freshness tier tags (crux — see risk §14.5).**

- Every served fact MUST carry a `tier` ∈ {`treesitter`, `lsp`, `cpg`} and a
  `freshness` ∈ {`fresh`, `stale`, `recomputing`}. Agents must be able to tell a
  tree-sitter-fast-but-approximate fact from an LSP-precise one.

### 5.2 Ground-truth reference & definition resolution *(multilspy → Hank; cap. 1)*

**FR-4:** Given a symbol or a `(file, line, col)` position, return its definition
site(s) and all reference sites, each with span, tier, and tenant-resolved
truth (base + overlay, see §5.5).

**FR-5:** Resolution must be served to Bobbin so it can turn "probably relevant"
into "provably connected," and to agents directly for navigation.

### 5.3 Call-graph & dataflow extraction *(Joern + codebase-memory → Hank; cap. 2)*

**FR-6: Call graph.** Build inter-procedural call edges (caller → callee) with
multi-strategy resolution (direct, method, dynamic/virtual best-effort), matching
codebase-memory's approach. Tag each edge with the resolution strategy and tier.

**FR-7: Code Property Graph.** Construct AST + control-flow + data/program-
dependence merged into one queryable graph (the Joern CPG idea). See §14.1 for
the JVM-vs-Rust build decision this forces (resolve in Phase 2).

**FR-8: Dataflow / taint.** Support source→sink reachability over the CPG so a
dataflow path can corroborate (or refute) a co-change edge.

**FR-9: Community detection.** Run deterministic Louvain community detection over
the structural graph (Quipu already exposes this via `quipu_project`; Hank
computes it live over the in-memory graph for the hot path).

### 5.4 Blast-radius / impact analysis *(Joern reachability + co-change → Hank; cap. 3)*

**FR-10:** Given a symbol/file/change set, compute the structurally-reachable
impacted set (forward: dependents; backward: dependencies) over the call/dataflow
graph, bounded by max hops and optional predicate filters — the same shape as
Quipu's `quipu_impact` but over Hank's live per-tenant graph.

**FR-11:** Reconcile the structural reachable set with Bobbin's historical
co-change set; surface edges that appear in one but not the other (structural-
only = new/unexercised coupling; co-change-only = a refactoring smell).

**FR-12 (crux):** The blast-radius reachability query MUST be implemented as a
single primitive reused for two purposes: (a) answering *"what does this change
affect?"* for a consumer, and (b) answering *"what must I recompute?"* for the
incremental updater (§5.5). **One primitive, two uses — build it once.**

### 5.5 Per-tenant live graph *(the tenancy model → Hank; cap. 4)*

**FR-13: Shared base.** Compute the full structural graph once at a baseline
commit (e.g. `main`), held **read-only** in memory.

**FR-14: Copy-on-write overlays.** Each tenant (developer/agent session) gets a
lightweight overlay: only touched files are re-parsed, only affected edges are
recomputed, layered over the shared base. Queries resolve against `base +
overlay`. An overlay MUST be invisible to other tenants (isolation is automatic).

**FR-15: Content-hash structural sharing.** Use content-hash keys (the codebase-
memory trick) so that N developers cost *one base + N small deltas*, not N full
graphs. Identical subtrees across tenants share storage.

**FR-16: Frontier-bounded incremental update.** Updating an overlay is **not** just
the edited file — it is the edited file *plus its frontier*. On edit: re-parse X
(cheap) → find changed symbols → **query the base graph for references/dependents
of those symbols** (this is FR-12's primitive) → recompute facts for that bounded
frontier → store as overlay. Naive per-file incremental update is wrong because
the consequences are non-local.

**FR-17: Tiered freshness.** Tree-sitter structure updates on save or debounced
keystroke; LSP/dataflow facts update on save or on-demand. (This is exactly the
tree-sitter-everywhere + LSP-for-a-subset split codebase-memory ships.)

**FR-18: Overlay lifecycle.** Overlays are created per session, evicted on session
close, and support explicit reset to base. Very-high-fan-in symbols (widely-
referenced signatures) may cascade the frontier; §14.2 requires an eviction and
special-handling policy.

### 5.6 Promotion to Quipu *(codebase-memory → Quipu; caps. 5, 6, 7 — see §9)*

**FR-19:** When changes land on a shared branch (commit/merge), promote the
corresponding structural facts into Quipu as a new bitemporal state
(valid-time = commit time; transaction-time = when learned).

**FR-20:** Promoted facts MUST be emitted as Turtle in the **existing `bobbin:`
code ontology** (`https://bobbin.dev/ontology#`, namespace constructors in
Quipu's `src/namespace.rs`) and validated against `shapes/code-entities.ttl`
(extended per §9.2) before write. Hank never writes to Quipu without passing
SHACL.

**FR-21:** Promotion writes via Quipu's existing surface — `quipu_knot` (MCP) /
`POST /knot` (REST) / `Store::transact` (in-process) — honoring
`valid_from`/`valid_to`, `transactions.actor` (= the promoting identity), and
`source` (= the commit SHA). Hank does **not** stand up its own triple store
(§14.4).

**FR-22:** Uncommitted overlay churn MUST NOT be promoted. Hank holds the
in-flight reality; Quipu holds only the settled record.

### 5.7 Monitor-guided edit verification *(multilspy monitors → Hank, served directly; cap. 8)*

**FR-23:** Given a proposed edit (an edited buffer), re-run analysis on that
buffer against the base graph Hank already holds and return a boolean verdict
plus violations: `identifier-does-not-exist`, `wrong-arity`, `type-violation`,
`unresolved-import`.

**FR-24:** Verification is single-signal and boolean — agents call Hank directly,
**not** through Bobbin. Bobbin may still *consume* verdicts like any other Hank
fact (e.g. to flag retrieved code that will not compile in the current overlay);
that is the normal Hank→Bobbin flow, not verification living in Bobbin.

### 5.8 Static-analysis-as-trust-boundary *(Hank blast radius → Broker/Aegis; cap. 9)*

**FR-25:** Expose per-tenant blast radius in a form the Gas Town broker/Aegis can
consume to scope a polecat's provisioned execution environment. Capability
scoping MUST be computed against the *requesting tenant's* live graph, never a
stale shared one (this is why the live per-tenant state must live in Hank).

### 5.9 Interfaces

**FR-26: MCP server.** Expose Hank's capabilities as MCP tools (§12) over both
stdio and streamable-HTTP transports, using `rmcp` exactly as Bobbin does
(`#[tool_router]` / `#[tool]` / `Parameters<T>` / `schemars`).

**FR-27: HTTP API.** Expose the same capabilities over a local Axum HTTP server
for the broker and non-MCP consumers, mirroring Quipu's REST-parallel-to-MCP
pattern.

**FR-28: CLI.** Provide a `hank` binary (clap, like Bobbin) with subcommands for
serving, one-shot analysis, and inspection (§Appendix A).

**FR-29: Config.** Read from the shared `.bobbin/config.toml` under a new `[hank]`
table (§11), with the same resolution order Quipu uses (flags > project toml >
user toml > defaults).

---

## 6. Non-Functional Requirements

### 6.1 Performance

| Metric | Target |
|---|---|
| Base graph build (tree-sitter tier), 100K LOC | < 30 s cold |
| Overlay update on single-file save (tree-sitter) | < 150 ms p95 |
| Frontier recompute, typical (non-hot symbol) | < 500 ms p95 |
| Reference/definition lookup (served) | < 50 ms p95 (base+overlay hit) |
| Blast radius, 5 hops, live graph | < 300 ms p95 |
| Edit verification verdict | < 200 ms p95 |
| LSP-precise fact (on-demand, warm server) | < 1 s p95 |

### 6.2 Scalability & memory

| Metric | Target |
|---|---|
| Codebase size | up to 1M LOC base graph |
| Concurrent tenants | ≥ 32 overlays on one base |
| Overlay cost | O(touched files + frontier), not O(repo) |
| Memory | base + Σ overlays within a configurable budget; content-hash sharing (FR-15) is the primary lever |

Overlay memory and hot-symbol churn are the top scaling risk (§14.2): the spec
requires an eviction policy and a high-fan-in special case, and requires Hank to
`log` when it bounds or truncates coverage rather than silently degrading.

### 6.3 Correctness & staleness semantics

- Every fact carries a tier and freshness tag (FR-3). A served fact must never
  present a tree-sitter approximation as LSP-precise.
- Tenant isolation is absolute: no overlay is ever observable by another tenant.
- Promotion to Quipu is all-or-nothing per commit and must pass SHACL; a
  validation failure blocks the write and surfaces the violations (it does not
  write partial facts).

### 6.4 Reliability & portability

- Graceful handling of unparseable files, missing language servers, and
  build-free repos (degrade tier, never crash).
- Same platform matrix as Bobbin: macOS (ARM64/x86_64), Linux (x86_64/ARM64).
- Single binary for the Rust core; language servers and any JVM extractor are
  external processes managed behind a boundary (§14.1).

### 6.5 Security & privacy

- Local-first, matching Bobbin/Quipu: no code leaves the machine during normal
  operation. Language servers run locally.
- The HTTP surface honors the same read-only / bearer-token guards Quipu uses
  (`http_auth.rs` pattern) for any write-ish endpoint (e.g. promotion trigger).

---

## 7. Technical Architecture

### 7.1 High-level components

```text
┌────────────────────────────────────────────────────────────────────┐
│                        MCP (rmcp)  ·  HTTP (axum)  ·  CLI (clap)     │
├────────────────────────────────────────────────────────────────────┤
│                            Query / Serve layer                       │
│   refs · defs · callgraph · dataflow · blast-radius · verify         │
│   (all resolve against base + tenant overlay, tier/freshness tagged) │
├────────────────────────────────────────────────────────────────────┤
│                        Tenancy layer (the hard part)                 │
│  ┌────────────────┐   ┌──────────────────────────────────────────┐  │
│  │  Shared base   │   │  Per-tenant overlays (copy-on-write)      │  │
│  │  graph (RO)    │◄──│  touched files + frontier, content-hashed │  │
│  └────────────────┘   └──────────────────────────────────────────┘  │
│        ▲   blast-radius primitive (FR-12): one query, two callers    │
├────────┼─────────────────────────────────────────────────────────────┤
│        │                 Extraction layer                            │
│  ┌───────────┐  ┌───────────────┐  ┌────────────────────────────┐    │
│  │ tree-sitter│  │  LSP client   │  │  CPG / dataflow (Phase 2)  │    │
│  │  (breadth) │  │ (multilspy-ish)│  │  Rust traversals or Joern  │    │
│  └───────────┘  └───────────────┘  │  behind a process boundary │    │
│                                     └────────────────────────────┘    │
├────────────────────────────────────────────────────────────────────┤
│   File-watch (notify)   ·   Git baseline (gix/git2)   ·   Overlay    │
│   cache (in-mem + optional rusqlite spill)                           │
├────────────────────────────────────────────────────────────────────┤
│         Promotion boundary  →  Quipu (quipu_knot / REST / in-proc)   │
│         emits bobbin: Turtle, SHACL-validated before write           │
└────────────────────────────────────────────────────────────────────┘
```

### 7.2 Proposed source layout (`src/`)

Mirrors Bobbin's module-per-concern style (one file/dir per responsibility, a
thin `main.rs` that inits tracing + parses the CLI):

```text
src/
  main.rs            # tracing init, CLI parse+dispatch (#[tokio::main])
  cli/               # one module per subcommand (serve, analyze, refs, impact, verify, promote, status)
  config.rs          # [hank] table, load_merged (defaults < user < project < flags)
  errors.rs          # thiserror error type + Result alias
  extract/
    treesitter.rs    # grammar registry, symbol tree, intra-file calls
    lsp/             # language-agnostic LSP client (multilspy idea), per-language servers
    cpg.rs           # CPG construction + dataflow (Phase 2; Joern boundary or Rust traversals)
    resolve.rs       # multi-strategy call resolution, import resolvers
  graph/
    base.rs          # shared read-only base graph (petgraph-backed)
    overlay.rs       # copy-on-write overlay, content-hash sharing
    tenant.rs        # tenant/session registry, base+overlay resolution
    blast.rs         # FR-12 reachability primitive (impact + frontier)
    community.rs     # Louvain over the live graph
  serve/
    refs.rs · impact.rs · verify.rs · callgraph.rs · dataflow.rs
  watch.rs           # notify-based file-watch, debounce, tier scheduling
  promote/
    ontology.rs      # bobbin: IRI minting (reuse Quipu namespace constructors)
    turtle.rs        # emit facts as Turtle (oxrdf/oxttl)
    quipu.rs         # #[cfg(feature="quipu")] promotion via quipu_knot / Store::transact
  mcp/               # rmcp server (server.rs handlers, tools.rs DTOs) — Bobbin pattern
  http/              # axum server + handlers (broker + REST)
  types.rs           # Fact, Tier, Freshness, Symbol, Edge, Tenant, Overlay
```

### 7.3 Core data model (`types.rs`)

```rust
enum Tier { TreeSitter, Lsp, Cpg }
enum Freshness { Fresh, Stale, Recomputing }

enum SymbolKind { // matches shapes/code-entities.ttl sh:in enumeration
    Function, Method, Class, Interface, Enum, Struct,
    Variable, Constant, Module, Property, Field, Constructor, TypeAlias,
}

enum EdgeKind {   // §9.2 predicates
    Calls, References, DefinedIn, Imports,
    DataDependsOn, ControlDependsOn,
}

struct Fact { subject: Iri, edge: EdgeKind, object: Iri, tier: Tier, freshness: Freshness }
struct Overlay { tenant: TenantId, base_commit: Oid, touched: HashMap<PathBuf, FileFacts>, frontier: HashSet<SymbolId> }
```

### 7.4 The blast-radius primitive (FR-12), made concrete

```text
fn reachable(seed: &[SymbolId], dir: Direction, hops: u32, view: &TenantView) -> ReachSet
    // dir = Forward  → dependents  → "what does this change affect?"  (consumer)
    // dir = Backward → dependencies → context for recompute
    // Called by serve/impact.rs AND by graph/overlay.rs::update_frontier.
    // Same traversal, same code, two callers.
```

### 7.5 Data flow

**Baseline build:** walk repo (respect `.gitignore` via `ignore`) → tree-sitter
parse each file → symbol tree + intra-file calls → resolve inter-procedural calls
→ (Phase 2) CPG/dataflow → hold read-only base keyed by content hash.

**Overlay update (on save):** notify event → debounce → re-parse touched file →
diff symbols vs base → `reachable(changed, Backward+Forward)` to bound the
frontier → recompute frontier facts (tree-sitter now, LSP on demand) → write
overlay delta.

**Serve:** request carries a `tenant` (and optionally a position) → resolve
`base + overlay` → return tier/freshness-tagged facts.

**Promote (on commit/merge):** diff committed change vs base → emit `bobbin:`
Turtle for the affected facts → SHACL-validate → `quipu_knot` with valid-time =
commit time, source = SHA → advance the base to the new commit.

---

## 8. Technology Choices

Hank most resembles **Bobbin** on the serving side (async, MCP, tree-sitter,
file-watch) and borrows **Quipu's** graph and RDF crates for the analysis and
promotion sides. Versions below are pinned to what the two peers already use, so
the three build against a coherent dependency set.

| Concern | Choice | Version | Matches |
|---|---|---|---|
| Language / edition | Rust, **edition 2021** | — | Bobbin (Hank is closest to Bobbin's rmcp serving core; see note) |
| Async runtime | `tokio` (full) | `1` | Bobbin |
| MCP SDK | `rmcp` (server, transport-io, streamable-http, axum) | `0.12` | Bobbin |
| JSON schema | `schemars` | `1.0` | Bobbin |
| CLI | `clap` (derive, env) + `clap_complete` | `4` | Bobbin |
| Tree-sitter | `tree-sitter` + rust/ts/python/go/java/cpp grammars | `0.25` / `0.24`/`0.23`/`0.25`/`0.23`/`0.23`/`0.23` | Bobbin (identical grammar set) |
| Graph algorithms | `petgraph` | `0.7` | Quipu |
| Datalog (optional, for derived edges) | `datafrog` | `2` | Quipu |
| RDF model / Turtle | `oxrdf` / `oxttl` / `oxrdfio` | `0.3` / `0.2` / `0.2` | Quipu |
| SPARQL (if Hank ever parses queries) | `spargebra` | `0.4` | Quipu |
| SHACL (validate before promotion) | `rudof_lib` (behind `shacl`/`quipu` feature) | `0.2.8` | Quipu |
| Overlay spill / cache (optional) | `rusqlite` (bundled) | `0.33` | Both |
| HTTP server | `axum` + `tower-http` (cors, trace) | `0.8` / `0.6` | Both |
| File-watch | `notify` | `6` | Bobbin |
| Git baseline | `gix` or `git2` | (TBD Phase 1) | — (Bobbin shells to git in `index/git.rs`; pick one, see §16) |
| Error handling | `thiserror` (+ `anyhow` in bins only) | `2` / `1` | Both (Quipu is thiserror-only; Bobbin uses both) |
| Serialization | `serde` / `serde_json` / `toml` | `1` / `1` / `0.8` | Both |
| Logging | `tracing` + `tracing-subscriber` | `0.1` / `0.3` | Bobbin |
| Hashing | `sha2` / `hex` | `0.10` / `0.4` | Bobbin (content-hash sharing) |
| Quipu integration | `quipu` git dep, pinned by rev, `default-features = false`, optional | rev-pinned | Bobbin's exact pattern |

**Edition note.** Bobbin is edition 2021; Quipu is edition 2024. Hank sits on
Bobbin's serving stack (`rmcp`, async, `notify`, `tracing`) and shares Bobbin's
request-path role, so **edition 2021** is the default choice for compatibility
with that surface. This is a reversible decision; revisit if a 2024-only
dependency becomes compelling (§16, open question 1).

**Feature flags** (mirroring both peers' feature discipline):

- `quipu` — gates the entire promotion path (`dep:quipu`, `oxttl`, `rudof_lib`).
  Off by default so Hank compiles and serves without the promotion toolchain, and
  — critically — **CI builds and tests both with and without it**, the single
  most-emphasized convention in Bobbin (the "don't let a feature ship dark" rule).
- `cpg` — gates the Phase-2 CPG/dataflow extractor (and any JVM boundary).
- `lsp` — gates the LSP tier if we want tree-sitter-only builds for constrained
  environments.

**Lints.** Adopt Quipu's in-manifest `[lints.rust]` / `[lints.clippy]` block
verbatim (`unsafe_code = "deny"`, `unused_must_use = "deny"`, `missing_docs =
"warn"`, plus the ~25 clippy warns) so Hank matches house style from commit one.

**The `quipu` dependency**, following Bobbin's Cargo.toml comment discipline
exactly: pin by `rev` (not `branch`, because `Cargo.lock` is gitignored and a
branch dep would float to tip on a fresh CI checkout), use `default-features =
false` to keep Quipu's `onnx`/`shacl` off unless Hank explicitly needs them, and
document the chosen rev and why bumping it is a migration, not a version bump.

---

## 9. The Code Ontology & Quipu Promotion

This is where Hank meets Quipu, and where the vision needs the most reconciliation
with reality.

### 9.1 What already exists (build on it, don't reinvent)

Quipu already ships a code ontology and SHACL contract:

- **Namespace:** `bobbin: <https://bobbin.dev/ontology#>` (and the SHACL file's
  `bobbin: <http://aegis.gastown.local/ontology/>` target class prefix). IRI
  constructors live in Quipu `src/namespace.rs`: `code_module_iri`,
  `code_symbol_iri`, etc., minting IRIs like `bobbin:code/{repo}/{path}::{symbol}`.
- **Classes (in `shapes/code-entities.ttl`):** `CodeModule` (requires `filePath`,
  `repo`, `language`), `CodeSymbol` (requires `name`, `definedIn` → CodeModule;
  `symbolKind` enumerated), `Document`, `Section`, `Bundle`.
- **Bobbin↔Quipu type mapping** (`bobbin-quipu-mapping.toml`): `CodeSymbol` →
  `aegis:SoftwareComponent`, `CodeModule` → `aegis:CodeRepository`, etc., surfaced
  predicates `aegis:dependsOn`, `aegis:ownedBy`, `aegis:runsOn`.

Hank promotes into **this** model. It mints the **same** IRIs so Bobbin's and
Hank's facts about the same symbol reconcile on a shared identifier.

### 9.2 What Hank adds (ontology extension)

The existing shapes cover *entities* (modules, symbols) but not the *structural
edges* Hank exists to produce. Hank contributes new predicates and their SHACL
shapes (to be added to `code-entities.ttl`, or a sibling `code-edges.ttl`):

| Predicate | Domain → Range | Meaning | Source tier |
|---|---|---|---|
| `bobbin:calls` | CodeSymbol → CodeSymbol | caller invokes callee | tree-sitter / cpg |
| `bobbin:references` | CodeSymbol → CodeSymbol | use site of a definition | lsp |
| `bobbin:imports` | CodeModule → CodeModule | module dependency | tree-sitter |
| `bobbin:dataDependsOn` | CodeSymbol → CodeSymbol | data-dependence edge | cpg |
| `bobbin:controlDependsOn` | CodeSymbol → CodeSymbol | control-dependence edge | cpg |
| `bobbin:hasTier` | Fact → literal | provenance/confidence tag | (all) |

Following the vision's guidance — *"start permissive, tighten deliberately"* (a
good code ontology over-constrained will reject legitimate facts from messy real
code) — these shapes begin with minimal cardinality/datatype constraints and add
`sh:class` domain/range checks only once real promoted data validates cleanly.

**Sample shape (new edge, in the existing SHACL style):**

```turtle
@prefix sh:     <http://www.w3.org/ns/shacl#> .
@prefix bobbin: <http://aegis.gastown.local/ontology/> .

bobbin:CallsShape a sh:NodeShape ;
    sh:targetSubjectsOf bobbin:calls ;
    sh:property [
        sh:path bobbin:calls ;
        sh:class bobbin:CodeSymbol ;   # range: callee is a CodeSymbol
        sh:minCount 1 ;
    ] .
```

### 9.3 Bitemporal promotion

Promotion uses Quipu's bitemporal model directly (Quipu `concepts/temporal-model`):

- **valid-time** (`--timestamp` / `valid_from`) = the commit's author/commit time.
- **transaction-time** (`transactions.timestamp`, monotonic tx id) = when Hank
  learned/promoted the fact.
- A signature change that removes an edge is a **retraction** (close `valid_to`),
  not a delete — Quipu's log is append-only, so code archaeology ("what called
  this function as of last March?") is answerable via `--valid-at`.

This gives capability 6 (bitemporal code archaeology) and capability 7
(SPARQL-over-code) for free, because they are Quipu features once the facts are
in the graph. **Sample SPARQL over promoted code:**

```sparql
# Who called authenticate() as of 2026-03-01?  (valid-time travel)
SELECT ?caller WHERE {
  ?caller <http://aegis.gastown.local/ontology/calls>
          <http://aegis.gastown.local/ontology/code/hank/src%2Fauth.rs::authenticate> .
}
# executed with valid_at = 2026-03-01
```

### 9.4 Branches as named graphs (make Quipu a quad store)

The vision proposes modeling each branch's committed facts as an **RDF named
graph**, bitemporally versioned within. **Quipu today is a triple store, not a
quad store** — there is no `GRAPH` / quad handling in its SPARQL engine or EAVT
schema. The recommended resolution is to **add quad support to Quipu** and make
named graphs the branch axis, rather than reifying a branch qualifier onto every
promoted edge.

This is the right call because a quad store is a **strict superset** of a triple
store, so the change is *additive* and can be made non-breaking:

- Add a graph term `g` to Quipu's `facts` identity. Existing facts migrate into
  the **default graph** (`g = NULL`/sentinel); nothing is deleted or rewritten.
- SPARQL without a `GRAPH` clause keeps hitting the default graph; `spargebra`
  already parses `GRAPH` / `FROM` / `FROM NAMED`, so the evaluator in
  `src/sparql/` gains graph-scoped BGP matching without a new query language.
- Bobbin (pinned to an old Quipu rev, `default-features = false`) is insulated
  during the transition.

**Why it's worth a Quipu-core change, not just a Hank convenience:** named graphs
pay off well beyond branches. Quipu already has a `docs/design/group-
isolation.md`, per-source provenance (`transactions.source`, episode
`prov:wasGeneratedBy`), and a `FederatedProvider` — all of which want the same
primitive: a first-class way to partition the graph. Branches are simply the
first customer. One quad column serves branch scoping, group isolation, and
provenance/federation at once, which is *less* total complexity than solving each
separately (a branch-qualifier hack in Hank *plus* group isolation *plus* source
scoping).

**Where the design care goes:** the interaction of three axes — `graph ×
valid-time × transaction-time`. Each fact already carries two time dimensions;
adding a graph dimension means the index permutations (`idx_eavt/aevt/vaet`),
retraction semantics (does closing `valid_to` scope to a graph?), the `datafrog`
reasoner (which graphs does a rule range over?), and SHACL targeting (which graph
do shapes validate?) each grow a graph-awareness question. None are individually
hard; together they are the surface to design deliberately. **Decide
default-graph-is-union vs. default-graph-is-distinct early** — it is the dataset
semantics choice that is painful to reverse later.

**Sequencing (does not block Hank).** Hank Phases 1–3 (extraction, dataflow,
tenancy) never touch Quipu. Only Phase 4 (promotion) cares. So the quad work is a
**Phase 4 enabler tracked on the Quipu side** (see §9.5 for the RFC sketch), not a
Hank dependency. If quads land first, Hank promotes each branch's committed facts
directly into a named graph named for the branch (bitemporally versioned within).
If they are not ready when Phase 4 starts, Hank falls back to **branch-as-
qualifier** (a reified `bobbin:onBranch` term on each edge, queries adding a
`?fact bobbin:onBranch "main"` constraint) — heavier queries, no Quipu change —
and migrates to named graphs when they arrive. The config `branch_model` key
(§11) selects between them.

### 9.5 Quipu quad-store RFC (sketch, Quipu-side follow-up)

> **Tracked as [scbrown/quipu#36](https://github.com/scbrown/quipu/issues/36)** —
> *"store: add named-graph (quad) support — additive, default-graph-preserving."*

A short design note to raise in `scbrown/quipu` (natural home:
`docs/design/group-isolation.md` or a new `docs/design/named-graphs.md`):

- **Schema:** add `g INTEGER` (interned graph IRI, nullable = default graph) to
  `facts`; extend the primary key and the EAVT/AEVT/VAET index permutations to be
  graph-aware (or add a `GEAVT`-style permutation). Keep it nullable so the
  migration is a column-add, not a rewrite.
- **SPARQL dataset semantics:** define the active dataset (default graph = union
  of all graphs, or a distinct empty default) and wire `GRAPH ?g { … }`,
  `FROM`, and `FROM NAMED` through the evaluator. Pick union-vs-distinct once.
- **Bitemporality:** `valid_from`/`valid_to`/`tx` stay per-fact; retraction and
  time-travel scope *within* a graph. Confirm `Store::speculate` savepoints and
  contradiction detection are graph-local.
- **SHACL / reasoner:** decide the graph a shape targets by default (all graphs,
  or the default graph) and the graphs a `datafrog` rule ranges over.
- **MCP/REST:** `quipu_knot` / `POST /knot` gain an optional `graph` parameter;
  `quipu_query` honors `GRAPH`. Backward compatible when omitted.
- **Migration:** existing `data/quipu.db` facts move to the default graph in
  place; no downstream break for Bobbin's pinned rev.

### 9.6 Two graph engines — keep the split honest

Hank's in-memory graph serves interactive dataflow/reachability queries that are
genuinely painful over RDF/SPARQL. Quipu serves governed/temporal/cross-domain
queries. The rule (from the vision's risks): **Hank's transient store must never
become a second source of truth for committed facts.** Committed truth lives in
Quipu; Hank holds only what is in flight plus a read-only projection of the base.

---

## 10. MCP & HTTP Tool Surface

Tool naming mirrors the peers: Bobbin uses bare snake_case function names that
clients namespace as `bobbin_*`; Quipu uses explicit `quipu_*`. Hank uses
**`hank_*`** for clarity alongside both on the same agent.

| Tool | Purpose | Routes to |
|---|---|---|
| `hank_definition` | Definition site(s) of a symbol/position | §5.2 |
| `hank_references` | All reference sites of a symbol | §5.2 |
| `hank_callers` / `hank_callees` | Call-graph neighbors | §5.3 |
| `hank_dataflow` | Source→sink dataflow paths | §5.3 |
| `hank_impact` | Blast radius (forward/backward, N hops) | §5.4 |
| `hank_symbols` | Symbol tree for a file/module | §5.1 |
| `hank_verify` | Verdict on a proposed edit buffer | §5.7 |
| `hank_status` | Base commit, tenant overlays, tiers, freshness | §5.5 |
| `hank_promote` | Trigger promotion of a commit to Quipu (write-guarded) | §5.6 |

Every tool response carries `tier` and `freshness` per FR-3, and every request
that reads structure accepts a `tenant` parameter (defaulting to a single-tenant
session in Phase 1). Registration follows Bobbin's `rmcp` pattern exactly:
`#[tool_router]` impl, `#[tool(description = …)]` async fns taking
`Parameters<Req>` where `Req: Deserialize + schemars::JsonSchema`, responses
serialized with `serde_json::to_string_pretty` into `CallToolResult::success`.
The HTTP API exposes a parallel endpoint per tool (Quipu's REST-mirrors-MCP
pattern) for the broker.

---

## 11. Configuration

Hank shares Bobbin/Quipu's `.bobbin/config.toml` under a new `[hank]` table, with
the same resolution order (compiled defaults < `~/.config/bobbin/config.toml` <
`.bobbin/config.toml` < CLI flags). No new environment variables beyond what
Bobbin defines (e.g. `BOBBIN_ROLE` for tenant identity, reused).

```toml
[hank]
# Baseline the shared read-only graph is built at.
base_ref = "main"

# Which extraction tiers to run.
enable_lsp = true          # LSP precision where a build resolves
enable_cpg = false         # Phase 2: CPG/dataflow

# Languages (default = Bobbin's grammar set).
languages = ["rust", "typescript", "python", "go", "java", "cpp"]

[hank.freshness]
# Debounce keystroke-driven tree-sitter updates (ms); LSP/CPG on save/on-demand.
debounce_ms = 300
lsp_on = "save"            # "save" | "on_demand"

[hank.tenancy]
max_overlays = 32
# Symbols with fan-in above this get special frontier handling (§14.2).
high_fanin_threshold = 200
overlay_eviction = "on_session_close"   # "on_session_close" | "lru"

[hank.serve]
bind_address = "127.0.0.1"
mcp_http_port = 3040       # distinct from Bobbin's server and Quipu's 3030
read_only = false          # broker/promotion write guard (http_auth pattern)

[hank.quipu]               # promotion target (feature = "quipu")
enabled = false
promote_on = "merge"       # "commit" | "merge" | "manual"
branch_model = "named_graph" # §9.4: "named_graph" (preferred, needs Quipu quads) | "qualifier" (fallback)
shapes_path = "shapes/"    # code-entities.ttl (+ code-edges.ttl)
```

---

## 12. Milestones / Phasing

Phasing follows the vision's five phases. Each is a checklist with an exit
criterion; every phase must keep the `quipu` feature compiling both on and off
(Bobbin's dark-feature rule) and must land docs + tests per §13.

### Phase 1 — Hank, single-tenant *(explained retrieval, no new store)*

- [ ] Project scaffold: Cargo (edition 2021), `[lints]` block, `just` + pre-commit + CI (both feature arms), mdBook skeleton.
- [ ] Tree-sitter extraction (Bobbin's grammar set): symbol tree + intra-file calls.
- [ ] LSP client (multilspy-style) for ≥ Rust + one more language: defs/refs/types.
- [ ] Tier/freshness tagging (FR-3) from day one.
- [ ] Single-tenant in-memory graph; `hank_definition` / `hank_references` / `hank_symbols` / `hank_callers` over MCP (stdio + HTTP).
- [ ] CLI: `serve`, `analyze`, `refs`, `status`.
- **Exit:** Bobbin fuses Hank's precise references with its co-change/embeddings; "probably relevant" becomes "provably connected."

### Phase 2 — Dataflow & blast radius

- [x] Call graph (FR-6): tree-sitter call-site extraction, by-name resolution, in-memory `CodeGraph`.
- [x] Blast-radius primitive (FR-10, FR-12) with forward/backward reachability (`reachable()`, one primitive).
- [x] `hank_impact`, `hank_callers`, `hank_callees` (MCP) and `hank callers` / `hank impact` (CLI).
- [x] Resolve the JVM/Rust CPG decision (§14.1): **Rust-native traversals** (Joern not adopted).
- [x] Intra-procedural data dependence (FR-8, first slice): `src/dataflow.rs`, `hank dataflow` (CLI) and `hank_dataflow` (MCP).
- [ ] Deeper CPG: control dependence + inter-procedural taint (FR-7, remainder of FR-8), behind the `cpg` feature.
- [ ] Reconcile structural reachable set with Bobbin co-change (FR-11).
- **Exit:** structural blast radius, reconciled with history, served to agents and Bobbin.

### Phase 3 — Multi-tenancy *(the hard phase)*

- [ ] Shared base + copy-on-write overlays (FR-13, FR-14).
- [ ] Content-hash structural sharing (FR-15).
- [ ] Frontier-bounded incremental update reusing the Phase-2 blast primitive (FR-16).
- [ ] File-watch (`notify`) + debounce + tiered scheduling (FR-17).
- [ ] Overlay lifecycle + high-fan-in handling + eviction (FR-18, §14.2).
- [ ] `tenant` parameter across the MCP/HTTP surface; `hank_status` shows overlays.
- **Exit:** N developers edit concurrently; each sees a correct, isolated `base + overlay`; overlays cost O(touched + frontier).

### Phase 4 — Promote to Quipu

- [ ] Extend the code ontology with edge shapes (§9.2); start permissive.
- [ ] Turtle emission (`oxttl`) with the existing `bobbin:` IRI constructors.
- [ ] SHACL-validate (`rudof`) before every write (FR-20).
- [ ] Promote on commit/merge via `quipu_knot` / `Store::transact`, bitemporal (FR-19, FR-21, FR-22).
- [ ] Branch modeling per §9.4: promote each branch into a named graph if Quipu
      quad support (§9.5) has landed; else branch-as-qualifier fallback. SPARQL-
      over-code recipes.
- **Exit:** committed structure lives in Quipu, SHACL-validated, bitemporally queryable; uncommitted churn never pollutes it.

### Phase 5 — Consumption & guardrails

- [ ] Per-tenant blast radius wired into the broker/Aegis capability-scoping path (FR-25).
- [ ] `hank_verify` monitor-guided edit verification as a direct surface (FR-23, FR-24).
- [ ] Bobbin consumes verdicts to flag won't-compile retrieved code.
- **Exit:** structure defines the polecat sandbox, per tenant; agents get a boolean guard on their own edits.

---

## 13. Testing & Dev Tooling

Adopt both peers' conventions so Hank is a first-class citizen of the stack from
commit one:

- **`just` is the only entrypoint** (never raw `cargo`); justfile quiet by
  default with `verbose=true` to override; group related ops under subcommands
  (`just docs build`).
- **`just check` is the pre-push gate** — pre-commit hooks (trailing-whitespace,
  EOF, yaml/json, merge-conflict, large-files, markdownlint-cli2) + `cargo fmt
  --check` + clippy `-D warnings`. **Do not push if it fails.**
- **CI matrix builds/tests/clippies both feature arms** (`--features quipu` and
  `--no-default-features`) — Bobbin's hardest-won lesson; dropping either arm
  re-creates the dark-feature bug.
- **Tests:** inline `#[cfg(test)]` unit tests colocated with modules (Quipu
  style) + `tests/` integration tests via `assert_cmd`/`predicates`/`tempfile`
  driving the `hank` binary (Bobbin style). New functionality ships with tests;
  tests are part of `just check`. Integration tests must **skip gracefully** when
  a language server or optional toolchain is unavailable (Bobbin's
  `try_indexed_project` pattern).
- **Docs:** mdBook under `docs/book/` with the peers' IA (getting-started /
  concepts / architecture / reference / tutorials); user-facing changes MUST
  update docs and README; `just docs build` must be clean; Vale + markdownlint +
  prettier for prose.
- **Release:** conventional commits + `release-plz` + `git-cliff` (Quipu's
  automated versioning/changelog).
- **"Landing the plane":** work is not complete until `git push` succeeds.

---

## 14. Risks & Mitigations

| # | Risk | Impact | Mitigation |
|---|---|---|---|
| 14.1 | **JVM/Rust fork for CPG.** Joern is JVM/Scala; the stack is Rust. | High | **Decided (Phase 2): Rust-native traversals.** Rather than embed Joern (a heavy JVM dep + serialization seam), Hank reimplements the traversals it needs, keeping the stack coherent. Started with intra-procedural data dependence (`src/dataflow.rs`, tree-sitter tier); a deeper CPG with inter-procedural taint can grow behind the `cpg` feature. Joern is not adopted. |
| 14.2 | **Overlay memory & churn.** Per-tenant overlays + a large base must stay in budget; frontier recompute on hot (high-fan-in) symbols can cascade. | High | Content-hash sharing (FR-15) as the primary lever; `high_fanin_threshold` special-casing; explicit overlay eviction policy (`on_session_close`/`lru`); `log` any bounded/truncated coverage — never degrade silently. |
| 14.3 | **When to promote to Quipu.** Every commit? Only merges to tracked branches? Promotion cost vs. history completeness. | Medium | `promote_on = commit\|merge\|manual` config; default `merge`. Bitemporality lets promotion be lazy but not free. |
| 14.4 | **Two graph engines drift.** Hank's transient store could become a second source of truth for committed facts. | Medium | Hard rule (§9.6): committed truth lives in Quipu; Hank holds in-flight + a read-only base projection only. Promotion is the one-way boundary. |
| 14.5 | **Freshness/staleness semantics.** Agents must know if a fact is tree-sitter-approximate or LSP-precise. | Medium | Mandatory `tier` + `freshness` tag on every fact (FR-3), surfaced in every MCP/HTTP response. |
| 14.6 | **Build-free vs build-required.** Joern's fuzzy parser needs no build; LSP needs a resolvable build for precise types. | Medium | Serve both: tree-sitter always-on breadth, LSP precision where a build exists; degrade tier, never fail; the ontology carries facts of differing confidence. |
| 14.7 | **Ontology design cost.** Over-constrained SHACL rejects legitimate facts from messy real code. | Medium | Start permissive (§9.2), tighten deliberately once real promoted data validates cleanly. |
| 14.8 | **Named-graph gap → Quipu quad-store work.** Quipu is a triple store; branches want named graphs. The fix is a Quipu-core change (add a graph column, graph-aware SPARQL), whose real cost is the `graph × valid-time × tx-time` interaction. | Medium | §9.4/§9.5: add quads *additively* (default-graph-preserving, non-breaking); sequence as a Phase-4 enabler tracked on the Quipu side, **not** on Hank's critical path; `branch_model = "qualifier"` is the zero-Quipu-change fallback if quads aren't ready. Decide default-graph union-vs-distinct early. |
| 14.9 | **Query-surface sprawl.** Resist standing up CPGQL *and* SPARQL *and* many MCP tools as permanent interfaces. | Low | Consolidate on SPARQL-over-Quipu for committed queries + Hank's `hank_*` MCP surface for live analysis. No second query language. |
| 14.10 | **`quipu` dep instability.** Quipu is pre-1.0; API drifts (Bobbin is pinned to a rev a full minor behind tip). | Medium | Pin `quipu` by `rev`, `default-features = false`, document the rev and why bumping it is a migration; CI compiles the `quipu` feature so drift can't ship dark. |

---

## 15. Open Questions

1. **Edition.** Default is 2021 (Bobbin's serving stack). Adopt 2024 (Quipu) only
   if a 2024-only dependency becomes compelling.
2. **Git access.** `gix` (pure-Rust, in-process) vs `git2` (libgit2) vs shelling
   out like Bobbin's `index/git.rs`. Pick in Phase 1; affects the baseline build
   and commit-diff path.
3. **CPG realization.** Joern-as-subprocess vs Rust-native traversals (§14.1) —
   the single biggest architectural fork; resolve early in Phase 2.
4. **Branch model.** Named graphs (via Quipu quad support, §9.4/§9.5) are the
   preferred path; branch-as-qualifier is the fallback. The open item is
   *sequencing*: does the Quipu quad work land before Hank Phase 4, and what are
   the default-graph dataset semantics (union vs. distinct)? Freeze before the
   promotion schema is.
5. **Promotion trigger.** On every commit vs only merges to tracked branches
   (§14.3) — trades promotion cost against history completeness.
6. **Tenant identity.** Reuse `BOBBIN_ROLE`/Gas Town crew identity as the tenant
   key, or mint a Hank-native session id? Affects broker capability scoping.
7. **Overlay persistence.** Pure in-memory vs `rusqlite` spill for large overlays
   / crash recovery — do we need durability for in-flight state at all?
8. **LSP server management.** Bundle/vendor language servers, or discover
   system-installed ones? Affects portability and the build-free story.

---

## 16. Glossary

| Term | Definition |
|---|---|
| **Base graph** | The full structural graph at a baseline commit, held read-only in memory and shared across tenants. |
| **Overlay** | A per-tenant copy-on-write delta over the base: touched files + recomputed frontier facts. |
| **Frontier** | The bounded set of symbols whose facts must be recomputed after an edit — the edited symbols plus their references/dependents. |
| **Blast radius** | The reachable set answering "what does this change affect?" — and, reused, "what must I recompute?" (FR-12). |
| **Tier** | Provenance/precision of a fact: `treesitter` (fast, approximate), `lsp` (precise, build-required), `cpg` (dataflow). |
| **Freshness** | Whether a served fact is `fresh`, `stale`, or `recomputing`. |
| **CPG** | Code Property Graph — AST + control-flow + data/program-dependence merged into one queryable graph (Joern's idea). |
| **Promotion** | Writing committed structural facts from Hank into Quipu as a new bitemporal state, SHACL-validated. |
| **Tenant** | A developer/agent session sitting at the base commit plus its own uncommitted working delta. |
| **Bitemporal** | Two time axes: valid-time (when true in the world = commit time) and transaction-time (when Quipu learned it). |
| **Named graph** | An RDF quad's graph component; the preferred branch axis. Not supported by Quipu today — §9.4/§9.5 propose adding quad support additively. |
| **LSP** | Language Server Protocol — the source of precise defs/refs/types. |
| **Monitor-guided verification** | Re-running analysis on an edited buffer to return a boolean "is this edit valid" verdict (multilspy monitors). |

---

## Appendix A: CLI Reference (Draft)

```text
USAGE:
    hank <COMMAND>

COMMANDS:
    serve       Run the MCP (stdio + HTTP) and HTTP API servers
    analyze     One-shot: build the base graph and print stats
    refs        Definitions and references for a symbol/position
    callers     Callers / callees of a symbol
    impact      Blast radius (forward/backward, N hops) for a change
    verify      Verdict on a proposed edit buffer
    promote     Promote a commit's structural facts into Quipu
    status      Base commit, tenant overlays, tiers, freshness
    completions Generate shell completions
    help        Print help

GLOBAL FLAGS:
    --json      Machine-readable output
    --quiet     Suppress non-essential output
    --verbose   Detailed progress
    --tenant    Tenant/session id (default: single-tenant)
    --config    Path to config file

EXAMPLES:
    hank serve
    hank analyze
    hank refs src/auth.rs::authenticate
    hank impact src/auth.rs::authenticate --hops 5 --forward
    hank verify --file src/auth.rs --buffer /tmp/edited.rs
    hank promote --commit HEAD
```

## Appendix B: Sample promoted Turtle (facts Hank emits into Quipu)

```turtle
@prefix bobbin: <http://aegis.gastown.local/ontology/> .
@prefix xsd:    <http://www.w3.org/2001/XMLSchema#> .

bobbin:code/hank/src%2Fauth.rs::authenticate
    a bobbin:CodeSymbol ;
    bobbin:name "authenticate" ;
    bobbin:symbolKind "function" ;
    bobbin:definedIn bobbin:code/hank/src%2Fauth.rs ;
    bobbin:calls bobbin:code/hank/src%2Fdb.rs::lookup_user ;
    bobbin:dataDependsOn bobbin:code/hank/src%2Ftoken.rs::verify .

bobbin:code/hank/src%2Fauth.rs
    a bobbin:CodeModule ;
    bobbin:filePath "src/auth.rs" ;
    bobbin:repo "hank" ;
    bobbin:language "rust" .
```

(Promoted with `valid_at` = commit time, `source` = commit SHA, via `quipu_knot`
/ `Store::transact`, after SHACL validation against `code-entities.ttl` +
`code-edges.ttl`. Under `branch_model = "named_graph"` (§9.4) these facts are
written into the branch's named graph — e.g. `GRAPH bobbin:branch/main { … }` —
once Quipu quad support (§9.5) has landed; under the `qualifier` fallback each
edge instead carries a `bobbin:onBranch "main"` term.)

## Appendix C: Sample `hank_impact` response (MCP)

```json
{
  "tenant": "strider",
  "seed": "src/auth.rs::authenticate",
  "direction": "forward",
  "hops": 5,
  "reachable": [
    { "symbol": "src/api/login.rs::handler", "distance": 1, "via": "calls", "tier": "lsp", "freshness": "fresh" },
    { "symbol": "src/api/session.rs::refresh", "distance": 2, "via": "dataDependsOn", "tier": "cpg", "freshness": "fresh" }
  ],
  "cochange_only": [ "docs/auth.md" ],
  "structural_only": [ "src/api/session.rs::refresh" ]
}
```

---

*Hank: live per-tenant code structure — the missing structural signal for the
Bobbin × Quipu stack.*
