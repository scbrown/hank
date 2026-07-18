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
- Project tooling matched to Bobbin and Quipu: `just` recipes, pre-commit,
  clippy lint policy, markdownlint/Vale/Prettier, mdBook, CI, and release-plz.
