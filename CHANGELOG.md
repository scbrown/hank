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
- Docs published to the `gh-pages` branch via `peaceiris/actions-gh-pages`.
- Project tooling matched to Bobbin and Quipu: `just` recipes, pre-commit,
  clippy lint policy, markdownlint/Vale/Prettier, mdBook, CI, and release-plz.
