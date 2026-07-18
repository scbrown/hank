# Tiers and Freshness

Full LSP-grade resolution on every keystroke is too expensive, so Hank serves
tiered facts — and **every fact it serves is tagged** so a consumer never
mistakes an approximation for ground truth.

## Tier — how a fact was derived

- `treesitter` — fast, build-free, approximate. Always-on breadth; works on a
  syntactically-broken buffer.
- `lsp` — precise defs/refs/types where a build resolves.
- `cpg` — control/data dependence from the code property graph (Phase 2).

## Freshness — how current a fact is

- `fresh` — reflects the latest observed edit.
- `stale` — known to be behind a pending edit.
- `recomputing` — a recompute is in flight.

Tree-sitter structure updates on save or debounced keystroke; LSP/CPG facts
update on save or on demand. Agents that need certainty ask for `lsp`/`cpg`
tiers; agents that need breadth take `treesitter` and know it.
