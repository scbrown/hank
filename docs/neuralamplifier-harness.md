# Addendum: Game-State & Policy Harness (NeuralAmplifier-driven)

> **Status: design intent, Phase 4+ and beyond.** This addendum extends
> [hank-spec.md](hank-spec.md) with the net-new capabilities the
> [NeuralAmplifier](https://github.com/scbrown/NeuralAmplifier) project needs from Hank. It
> continues the FR numbering (last core FR is FR-34). Nothing here is built; it is scoped
> honestly so the core spec's flow stays undisturbed.

## Why Hank

NeuralAmplifier is an LLM brain for *Alpha Centauri*. Its knowledge lives in Quipu (a governed
bitemporal graph — the SMAC datalinks and learned strategy). But three needs are a poor fit for
a persisted graph and a natural fit for Hank's **hot, per-tenant, copy-on-write in-memory
graph**:

1. A live board graph rebuilt every turn from the game's fog-limited world view.
2. A fast **policy guardrail** over proposed moves — the strategic analog of `hank_verify` /
   the pre-edit hook (FR-23/24/30), evaluated against game state rather than code.
3. **What-if** analysis over a proposed move — `hank_impact` (FR-11) generalized from the call
   graph to the board.

This is a real widening of Hank's mandate: from a *code* graph analyzer to a **general
in-memory fact graph + policy harness**. It reuses Hank's existing machinery (COW overlays,
multi-tenancy, impact BFS, the `rules::Rule` policy shape, the Quipu policy-projection path) but
requires a non-code ingestion capability Hank does not have today.

## New requirements

### FR-35 — Generic (non-code) fact ingestion

Hank's in-memory graph is built only from tree-sitter-parsed source; nodes are span-anchored
`CodeSymbol`/`CodeModule`. Add a **generic `Node`/`Edge` type not tied to source spans**, plus an
ingestion seam:

- `hank_ingest` (MCP) + `POST /ingest` — `{ entities[], edges[], tenant, provenance }`, mirroring
  the `quipu_episode` node/edge JSON shape so one adapter output can feed both stores.
- A new **tier** value `"engine-state"` alongside `tree-sitter | lsp | cpg` (FR-3). Provenance for
  these facts = the adapter id + turn + faction (not a `file:line`).
- Gated behind a new `game-state` Cargo feature; joins the CI matrix in the same change (the
  "don't ship dark" rule).

### FR-36 — Game-state policy selector/predicate model

Generalize the Quipu-authored policy model (`aegis:Policy`/`Selector`/`Predicate`, whose fields
already map 1:1 to `rules::Rule`) from code to game state. **1:1 reuse:** `matchType`, `gate`,
`effect` (`warn`|`deny`), `claim`/`targets`/`label`. **New:**

- `selectorLang ∈ { "tree-sitter", "graph-pattern", "sparql" }` — a discriminator. Code policies
  keep `"tree-sitter"` (`.scm` over the AST). Game-state policies use `"graph-pattern"`: a compact
  ASK-style pattern over the generic node/edge graph (FR-35). **Hank is not an RDF/SPARQL store** —
  full SPARQL stays Quipu's job; any datalinks a predicate references are projected from Quipu
  first.
- `boundary "order"` (a new value beside `"action"`) — evaluated at pre-apply of proposed orders.
- `tier "game-state"`.

```turtle
aegis:policy_garrison_border a aegis:Policy ;
    rdfs:label "garrison-border-bases" ;
    aegis:targets "BaseState" ;
    aegis:claim "every border base retains >=1 garrison after the proposed orders apply" ;
    aegis:boundary "order" ; aegis:effect "deny" ;
    aegis:selector  [ aegis:selectorLang "graph-pattern" ;
                      aegis:evidenceSource "?b a smac:BaseState ; smac:isBorderBase true" ] ;
    aegis:predicate [ aegis:selectorLang "graph-pattern" ; aegis:matchType "must-match" ;
                      aegis:evidenceSource "?b smac:garrisonCount ?n | ?n >= 1" ] .
```

### FR-37 — `hank_guard`: move/policy verify surface

The `(game_state + proposed_orders)` analog of `hank_verify` (FR-23/24):

- `hank_guard` (MCP) + `POST /guard` — `{ game_state, proposed_orders, tenant } → { violations[],
  advisories[] }`, each `{ policy, tier, claim, offending_order_ids[] }`.
- Evaluation: apply the proposed orders to a **COW overlay** of the hot board graph → run each
  policy's selector → evaluate the gated predicate on the *post-order* overlay → `deny` routes to
  `violations`, `warn` to `advisories`, each carrying `tier "game-state"`.
- **Complements, never replaces** the engine's own legality gate: it can only subtract or annotate
  *legal* moves.

### FR-38 — `hank_whatif`: what-if / impact over state

Generalize `hank_impact` (FR-11, BFS blast-radius over the call graph) to the board:

- `hank_whatif` (MCP) + `POST /whatif` (or a `speculate` flag on `hank_guard`) — speculatively
  apply an order-set to a COW overlay (the analog of Quipu's `speculate()` SAVEPOINT) and return a
  ranked downstream-impact set: bases exposed, own units entering enemy threat range, reachability
  / zone-of-control / supply shifts, opponent next-turn reach — **without committing**.
- Contrast to keep clear: `hank_whatif` = ephemeral live board, fast, this-turn, tactical;
  Quipu `quipu_impact remove=true` = persisted knowledge, durable, cross-game.

### FR-39 — Per-game / per-faction tenancy as an isolation boundary

Hank is already multi-tenant (per-developer COW overlays over a shared base graph). Map that
directly to games:

- Tenant = `(game_id, faction_id)`; per-turn overlays for the guard/what-if speculation.
- **Shared base graph** = public / common-knowledge facts (map size, public treaties, tech known
  to ≥3, observed sightings). **Per-faction COW overlay** = that faction's private intel (own
  units/bases, unexplored fog, plans). A tenant reads the base + its own overlay, **never a
  sibling's** — fog-of-war isolation falls out of the existing architecture. When several factions
  are LLM-driven in one game, this is a **security** boundary, not just organization.

## Honesty / dependencies

- All of FR-35..FR-39 is **net-new engineering**, not integration of existing capability. It is
  gated twice: by Hank **Phase 4** (HTTP-only Quipu promotion; the `quipu` crate dep is still
  commented out; verdict signing is unkeyed → engine-observed/state facts are trusted-advisory,
  not cryptographically trusted) **and** by the non-code ingestion of FR-35, which does not exist
  today. FR-36/37/38 depend on FR-35.
- **Hank is not a SPARQL store.** Game-state selectors are a compact graph-pattern subset over the
  native node/edge index; full SPARQL stays in Quipu.
- **The guard sees an *approximated* post-order board.** Applying proposed orders to a COW overlay
  re-implements a slice of the engine's order semantics outside the engine — a divergence risk. So
  `deny` policies must be conservative; the **game engine remains the sole authority** on legality
  and effects. This is exactly why `hank_guard` complements, and never replaces, that authority.

See NeuralAmplifier's [knowledge-architecture.md](https://github.com/scbrown/NeuralAmplifier/blob/main/docs/knowledge-architecture.md)
for how the brain consumes these surfaces.
