<p align="center">
  <img src="assets/logo.svg" width="200" alt="Hank logo — a coiled skein of yarn with graph nodes"/>
</p>

<h1 align="center">hank</h1>

<p align="center">
  <em>🧵 Live, per-tenant code structure — the missing structural signal for the Bobbin × Quipu stack</em>
</p>

<p align="center">
  <a href="LICENSE"><img src="https://img.shields.io/badge/License-MIT-blue.svg" alt="License: MIT"/></a>
  <a href="https://www.rust-lang.org"><img src="https://img.shields.io/badge/Rust-2021-orange.svg" alt="Rust 2021"/></a>
  <a href="docs/book/src/SUMMARY.md"><img src="https://img.shields.io/badge/docs-mdbook-green.svg" alt="Documentation"/></a>
  <a href="docs/hank-spec.md"><img src="https://img.shields.io/badge/spec-v0.1-blueviolet.svg" alt="Specification"/></a>
</p>

> *Bobbin holds the thread. Quipu ties the knots. **Hank** keeps the working coil — live, per-tenant, ready.* 🧶

A [hank](https://en.wikipedia.org/wiki/Hank_(textile)) is a coiled skein of yarn
kept ready while you work. **Hank** keeps a codebase's live structural graph the
same way: extracted once at a baseline, then layered with a lightweight
per-developer overlay so a whole team of humans and agents can edit at the same
time without corrupting each other's view. It answers the questions embeddings
and git-history can't — *what calls this, what does this flow into, what will
this change break* — and it answers them **per tenant**, correctly, while the
code is still in flight.

## 🧶 See It In Action

```text
$ hank analyze src
analyzed 7 file(s), 47 symbol(s) [tree-sitter]

$ hank refs authenticate src
src/auth.rs:18 authenticate (Function) [TreeSitter]

$ hank status
hank status
  base ref  : main
  tenant    : (single-tenant)
  tiers     : treesitter
  quipu     : enabled=false branch_model=named_graph
```

> **Status:** Phases 1 and 2 complete. `analyze`, `refs`, `status`, the
> call-graph commands `callers`/`impact` (with `--cochange` reconciliation), and
> intra-procedural `dataflow` do real work, and an MCP server (`hank serve`,
> `--features mcp`) exposes eight `hank_*` tools. `verify` and `promote` land per
> the [phasing](docs/hank-spec.md#12-milestones--phasing); Phase 3 (multi-tenancy)
> is next.

## 🤔 Why Hank? — and how it's different

Structural code intelligence isn't new; the strongest tools each prove out **one**
signal class. Hank deliberately takes the best idea from each — then adds the axes
none of them have: **a whole team editing at once, governance, and time.**

### Key selling points

- 🧵 **Correct under concurrency** — the only structural engine that stays right
  while a whole team of humans *and agents* edit the same base at once (shared base
  graph + per-tenant copy-on-write overlays).
- 🔀 **Fusion, not one signal** — call/dataflow structure *plus* historical
  co-change *plus* embeddings. A coupling backed by a dataflow path is real; one
  without is a refactoring smell — only fusion tells them apart.
- 🪢 **Governed & time-travelable** — committed facts promote into
  [Quipu](https://github.com/scbrown/quipu) as SHACL-validated, bitemporal RDF: a
  versioned source of truth, not a best-effort cache.
- 💥 **Blast radius as a primitive** — *"what will this change break,"* per tenant —
  and it doubles as the incremental-update engine.
- ⚡ **Two-tier freshness** — tree-sitter-fast breadth + LSP-precise depth, every
  fact confidence-tagged so an agent knows what it's trusting.
- 🛡️ **Structure scopes the sandbox** — per-tenant blast radius bounds what an
  autonomous agent may touch, and can act as *generation guardrails*, not just context.
- 🪙 **Token-cheap** — structural answers instead of dumping files into context.

### How it compares

| | **codebase-memory** | **Joern (CPG)** | **LSP / multilspy** | **Embeddings / co-change** | **Hank** |
|---|:--:|:--:|:--:|:--:|:--:|
| Fast structural graph, low token cost | ✅ | ⚠️ | ❌ | ✅ | ✅ |
| Call graph + **dataflow / taint** | ⚠️ | ✅ | ⚠️ | ❌ | ✅ |
| Precise LSP-grade types | tiered | ❌ | ✅ | ❌ | tiered |
| Incremental freshness on edit | ✅ | ❌ | ✅ | ❌ | ✅ *(frontier-bounded)* |
| **Correct while a team edits concurrently** | ❌ | ❌ | ❌ | ❌ | ✅ *(per-tenant overlays)* |
| **Governed, versioned, time-travel record** | ❌ | ❌ | ❌ | ❌ | ✅ *(→ Quipu)* |
| Blast radius scopes an **agent trust boundary** | ❌ | ❌ | ❌ | ❌ | ✅ |

Each proves one piece — **[multilspy](https://github.com/microsoft/multilspy)** that
LSP facts can also be *generation guardrails*, **[Joern](https://joern.io)** the Code
Property Graph and dataflow, **codebase-memory** a lean standalone analyzer with
content-hash incremental freshness. Hank is spiritually closest to codebase-memory,
extended with Joern-style dataflow, LSP precision, **tenancy**, and a governed
projection into Quipu.

> The moat isn't any single signal — it's **fusion + governance + time + tenancy**,
> kept correct while a whole team edits. No off-the-shelf tool does that.

## 🧩 The Stack — three tools, one job each

```text
        edit / save / file-watch
                 │
                 ▼
   ┌──────────────────────────┐   promote on commit/merge   ┌──────────┐
   │           HANK           │ ───────────────────────────► │  QUIPU   │
   │  base graph + overlays   │   (SHACL-validated Turtle)   │ EAVT log │
   │  tree-sitter + LSP + CPG │ ◄─────────────────────────── │ SPARQL   │
   └────────────┬─────────────┘   SPARQL over committed code └──────────┘
                │ blast radius (per tenant)
                ▼
        ┌───────────────┐   broker/Aegis        ┌──────────┐
        │ Bobbin fusion │◄──(trust boundary)────│  agents  │
        │ + serving     │───────────────────────►│ (polecat)│
        └───────────────┘   explained context   └──────────┘
```

- **[Hank](https://github.com/scbrown/hank)** (this repo) — extracts and serves
  live per-tenant structure.
- **[Quipu](https://github.com/scbrown/quipu)** — governs and versions the
  committed record (bitemporal RDF / SPARQL / SHACL).
- **[Bobbin](https://github.com/scbrown/bobbin)** — fuses everything with its
  statistical and embedding signals and serves explained context over MCP.

See [`docs/vision.md`](docs/vision.md) for the north star and
[`docs/hank-spec.md`](docs/hank-spec.md) for the full build spec.

## 🪢 Hank + Quipu — what the pair unlocks

Hank holds the *live* structure; [Quipu](https://github.com/scbrown/quipu) governs
the *committed* record (bitemporal RDF, SHACL-validated, SPARQL-queryable). Together
they do things neither does alone:

- **Governed SPARQL-over-code.** Query committed structure as typed, validated facts
  — *"every public function with no test," "modules that violate the layering," "who
  still calls this deprecated API"* — not a cache you hope is fresh.
- **Impact over history.** Bitemporal facts answer *what did this change break, and
  when did that coupling first appear* — blast radius that accounts for how the code
  got here, replayable at any point in time.
- **Ontology rules that block or influence changes.** Author architectural
  constraints as ontology rules in Quipu (SHACL over the code graph); Hank evaluates
  a proposed edit against them **live and per tenant**, and warns or blocks a
  violation *before it lands*. Policy-as-ontology — a new rule is a graph assertion,
  not a new bespoke linter.
- **Per-tenant parallel worlds.** A shared base plus copy-on-write overlays (Hank)
  map onto Quipu named graphs, so a whole team edits concurrently without corrupting
  each other's view — over a single **source-of-truth root** that's always queryable.
- **Agent trust boundaries.** Per-tenant blast radius scopes what an autonomous agent
  may touch — structure *defines the sandbox* — via the Aegis/broker machinery.
- **Code ↔ intent, linked.** Quipu provenance ties structural facts to the decisions
  and work-items that produced them — *"which decision does this module implement,"
  "what tickets co-occur with this code path."*

## 🚀 Quick Start

### Install

```bash
# From source — puts the `hank` binary on your PATH
cargo install --path .                              # or: just install
# with the MCP server and the extra language grammars:
cargo install --path . --features "mcp langs-extra"
```

### Use

```bash
# Analyze a tree and list its structure
hank analyze src
hank refs <symbol> src
hank status

# Call graph: callers/callees and blast radius
hank callers <symbol> src
hank impact <symbol> src --hops 5

# Data dependence within a function
hank dataflow <function> src --var <variable>

# Export the referential structure (code + docs) as governed RDF Turtle
hank export src --repo myrepo --format turtle

# Serve over MCP (stdio) for an agent
hank serve

# Edit-reactive: wire `hank hook post-edit` into a Claude Code PostToolUse hook
# for synchronous blast-radius advisories on every edit (see docs).

# Shell completions
hank completions bash > hank.bash
```

Hank shares the stack's `.bobbin/config.toml` under a `[hank]` table — see the
[configuration reference](docs/book/src/reference/config.md).

## 🌳 Supported Languages

Tree-sitter structural extraction (symbols, intra-file call edges, import
references — all tagged `TreeSitter`) is wired for Bobbin's full grammar set.
**Rust** is always built; the rest land behind the `langs-extra` feature
(`cargo build --features langs-extra`).

| Language       | Feature       | Extensions                                     |
| -------------- | ------------- | ---------------------------------------------- |
| Rust           | _(always on)_ | `.rs`                                           |
| TypeScript     | `langs-extra` | `.ts` `.mts` `.cts` `.js` `.mjs` `.cjs`         |
| TSX / JSX      | `langs-extra` | `.tsx` `.jsx`                                   |
| Python         | `langs-extra` | `.py` `.pyi`                                     |
| Go             | `langs-extra` | `.go`                                           |
| Java           | `langs-extra` | `.java`                                          |
| C / C++        | `langs-extra` | `.c` `.h` `.cc` `.cpp` `.cxx` `.hpp` `.hh` `.hxx` |

Each grammar contributes a per-language `GrammarSpec` (grammar + node-kind →
`SymbolKind` mapping + call/import extraction) to a shared, language-agnostic
walker in `src/extract/`; `language_for_extension` selects the grammar by file
extension. See [FR-1](docs/hank-spec.md) for the extraction-tier contract.

## 🛠️ Development

```bash
just setup            # install pre-commit hooks
just build            # cargo build
just test             # cargo test
just lint             # clippy -D warnings
just check            # full pre-push gate (fmt, clippy, markdownlint, file size)
just docs build       # build the mdBook
```

Conventions live in [`AGENTS.md`](AGENTS.md); contribution guidance in
[`CONTRIBUTING.md`](CONTRIBUTING.md). Always use `just`, never raw `cargo`.

## 📚 Documentation

- [Specification](docs/hank-spec.md) — the full PRD-style build spec.
- [Vision](docs/vision.md) — Bobbin × Hank × Quipu.
- [mdBook](docs/book/src/SUMMARY.md) — guides, concepts, and reference.

## License

[MIT](LICENSE) © 2026 Steve Brown
