# Changelog

All notable changes to this project will be documented in this file.

## [Unreleased]

### Added

- Vision document and full build specification (`docs/vision.md`,
  `docs/hank-spec.md`).
- Phase-1 scaffold: `hank` CLI (`analyze`, `refs`, `status`, `completions`,
  plus phase-gated `callers`/`impact`/`verify`/`promote`), tree-sitter Rust
  extraction, the tiered fact model, and the shared `[hank]` config table.
- Phase-1 MCP server (`mcp` feature): `hank serve` over stdio and
  streamable-HTTP, exposing `hank_status`, `hank_symbols`, `hank_references`,
  and `hank_analyze` via `rmcp` (Bobbin's registration pattern). The `mcp` arm
  is in the CI matrix.
- Phase-2 call graph and blast radius: tree-sitter call-site extraction, an
  in-memory `CodeGraph` (petgraph) with the `reachable()` primitive (FR-12),
  the `hank callers` and `hank impact` CLI commands, and the `hank_callers`,
  `hank_callees`, and `hank_impact` MCP tools.
- Phase-2 intra-procedural dataflow (Rust-native, resolving §14.1): per-function
  data-dependence extraction, `DependsOn`/`FlowsInto` flow queries, the
  `hank dataflow` CLI command, and the `hank_dataflow` MCP tool (eight tools
  total).
- Phase-2 exit (FR-11): reconcile the structural blast radius against a
  caller-supplied co-change set (`src/reconcile.rs`), partitioning files into
  corroborated / structural-only / co-change-only. Exposed via
  `hank impact --cochange` (CLI) and the `cochange` parameter on `hank_impact`
  (MCP). Co-change mining stays in Bobbin. **Phase 2 complete.**
- Edit-reactive harness integration (spec §5.9 / FR-30, prototype): `hank hook
  post-edit` reads a Claude Code `PostToolUse` payload on stdin and returns a
  synchronous cross-file blast-radius advisory as injected context — the agent's
  edit tool call *is* the change event. Spec §5.9 reworked into an interface
  model (hook / MCP / HTTP / CLI / optional LSP) with the resident-daemon latency
  requirement (FR-31).
- Unified code+docs referential structure (spec §5.10): the concept crystallized
  — code and docs are one referential graph (real-time for code, asynchronous for
  docs), distinct from Bobbin's chunking. Adds FR-33 (doc→code reference
  extraction) and FR-34 (`hank export`).
- Referential-structure export (FR-34, code side): `hank export --format turtle`
  emits `CodeModule`/`CodeSymbol` + `definedIn`/`calls` edges as RDF Turtle in the
  `bobbin:` code ontology (validating against `shapes/code-entities.ttl`) — the
  governed projection that is the substrate under Phase-4 promotion.
- Docs published to the `gh-pages` branch via `peaceiris/actions-gh-pages`.
- Project tooling matched to Bobbin and Quipu: `just` recipes, pre-commit,
  clippy lint policy, markdownlint/Vale/Prettier, mdBook, CI, and release-plz.
