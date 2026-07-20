# Changelog

All notable changes to this project will be documented in this file.

## [Unreleased]

### Docs

- **FR-27 (parallel HTTP API) marked phased, matching reality.** Five doc sites
  described a REST HTTP API parallel to MCP that does not exist — the whole axum
  router is one `nest_service("/mcp")`. FR-27 is the resident daemon's shared
  backplane (FR-31, Phase 3), and every capability is already reachable over TCP
  via the streamable-HTTP MCP transport, so the API is now documented as Phase 3
  and added to the §12 checklist rather than presented as present (aegis-ubvd).

## [0.2.0] - 2026-07-20

### Fixed

- **The pre-edit policy guard could block every edit in the fleet.** Absence of
  `hank` failed open (exit `127`), but a `hank` too old to know `hook pre-edit`
  answered it with the argument parser's error and exit `2` — Claude Code's
  fail-*closed* channel. Since parsing precedes any Hank code, the guard's own
  fail-open logic never ran. An unparseable `hank hook …` now degrades to a
  silent allow; other commands keep exit `2` so typos stay loud. The integration
  contract also pins a skew-proof invocation, since older binaries cannot be
  fixed retroactively (#35).
- **A workspace config silently disarmed the guard.** `HankConfig::load`
  documented itself as overlaying user then project config but assigned each
  file wholesale, so a project `.bobbin/config.toml` setting one unrelated key
  reset `[hank.policy]` to `mode = "off"` — enforcement stopped with no warning,
  indistinguishable from finding no violations. Config now merges per-key;
  arrays replace rather than accumulate, so a workspace cannot widen a scope the
  user config narrowed (#36).

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
