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

> **Status:** early Phase-1 scaffold. `analyze`, `refs`, and `status` do real
> tree-sitter work today; `serve`, `callers`, `impact`, `verify`, and `promote`
> are declared with their final shape and land per the [phasing](docs/hank-spec.md#12-milestones--phasing).

## 🤔 Why Hank?

Most code-intelligence tools pick one signal and go deep. The stack already had
two — embedding similarity (Bobbin) and historical co-change (Bobbin). Hank adds
the third, and makes it correct for a team:

|  | **Embeddings / co-change** | **A language server** | **Hank** |
|--|:--:|:--:|:--:|
| Knows call/dataflow structure | ❌ | ✅ | ✅ |
| Explains *why* two files couple | ❌ | ⚠️ | ✅ |
| Blast radius / impact | ❌ | ⚠️ | ✅ |
| Correct while a **team** edits concurrently | ❌ | ❌ | ✅ |
| Feeds a governed, versioned record | ❌ | ❌ | ✅ (→ Quipu) |
| Scopes an agent's sandbox (trust boundary) | ❌ | ❌ | ✅ |

The differentiated move is **fusion + governance + time + tenancy**: a co-change
edge with no structural explanation is a refactoring smell; one backed by a
dataflow path is real coupling. No single signal makes that distinction.

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

## 🚀 Quick Start

```bash
# Build
just build            # or: cargo build

# Analyze a tree and list structure
cargo run -- analyze src
cargo run -- refs <symbol> src
cargo run -- status

# Shell completions
cargo run -- completions bash > hank.bash
```

Hank shares the stack's `.bobbin/config.toml` under a `[hank]` table — see the
[configuration reference](docs/book/src/reference/config.md).

## 🌳 Supported Languages

Tree-sitter structural extraction ships for **Rust** today; the rest of Bobbin's
grammar set (TypeScript, Python, Go, Java, C/C++) is wired behind the
`langs-extra` feature and lands as extractors are filled in.

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
