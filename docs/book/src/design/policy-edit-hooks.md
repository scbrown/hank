# Policy edit hooks — the hank side

This documents hank's half of the **performant edit hooks for policy** design.
The write-time enforcement half lives in quipu and is already implemented; see
[quipu `docs/design/policy-edit-hooks.md`](https://github.com/scbrown/quipu/blob/main/docs/design/policy-edit-hooks.md)
for the full split. This page records what hank owns, why none of it ships yet,
and the backlog.

## Deciding principle: evidence locality

Evaluate a policy where its evidence is already hot.

- **Governed-fact policies** — the claim reasons over quipu's committed graph
  (provenance, prior verdicts, workflow state). Quipu evaluates these at its
  pre-commit gate.
- **Structural-evidence policies** — the claim reasons over the call graph,
  reachability, blast radius, or symbols. **Hank** holds that hot, per tenant,
  at the `boundary:"action"` edit-time seam (`hank hook pre-edit` + the resident
  daemon). Hank evaluates these against a hot **projection** of quipu's canonical
  policies.

Canonical policy definitions, SHACL validation, verdict signing, and the
human-owned `VerifierRegistration` root of trust stay in quipu regardless. Hank
holds only a projected read cache — a policy is never *defined* in hank.

## What hank owns

- A **hot projection** of quipu's `boundary:"action"` policies whose claims are
  structural, refreshed through hank's existing watch/daemon machinery. Strictly
  one-directional (quipu canonical → hank cache) with invalidation on quipu
  policy writes. If it diverged, hank could allow what quipu would deny — so the
  projection is a cache, never a source of truth.
- **Edit-time evaluation** at `hook pre-edit`: an edit that violates a structural
  `deny` policy is blocked at edit time, against the resident graph, with a
  tier-tagged verdict.
- **Verdict promotion**: hank's edit-time verdicts promote back into quipu as
  signed, bitemporal facts, extending the existing `commit → touched` promote
  path.

## Why none of this ships yet

Everything above is **Phase 4** and deliberately gated:

- The `quipu` dependency is commented out in `Cargo.toml`
  (`rev = "<pin-in-phase-4>"`). Hank builds standalone in CI; there is no quipu
  rev to project from today.
- Edit-time policy verdicts need to declare whether the registry they used was
  fresh or stale — that is FR-3 freshness, which is **not served yet** (Phase 3;
  it lands with the copy-on-write overlays + frontier-bounded incremental
  update). A verdict computed against a stale projection must be tagged stale,
  never silently `fresh`.

Adding a projection registry now — with nothing to populate it and no freshness
to qualify it — would ship a dark feature: a gate that gates nothing, or a
verdict that lies about its freshness. That is exactly the anti-pattern hank has
already corrected once (the `cpg`/`lsp` features were deleted because they gated
no code). So the hank side stays as design + backlog until its Phase-3/4
prerequisites land, rather than shipping scaffolding that advertises a precision
it does not have.

## Sequencing

```text
Phase 3 (hank)          Phase 4 (hank)                 quipu (done)
─────────────────       ─────────────────────          ────────────────────
freshness serving  ──▶  pin quipu dep (H-DEP)   ──┐
overlays + frontier      project action policies    │   write-path gate
                         (H-PROJECTION)              ├─▶ (enforce_on_write)
                         edit-time eval (H-EDIT)     │   already enforces
                         promote verdicts ───────────┘   governed-fact policies
                         (H-PROMOTE-VERDICT)
```

Quipu already enforces governed-fact policies on its write path. Hank adds the
structural, edit-time half once the dependency and freshness prerequisites are
in place.

## Backlog

Acceptance criteria per item. Status: ☐ open (all Phase-3/4-blocked).

- **H-DEP** ☐ Pin and wire the `quipu` dependency (Phase-4 kickoff).
  *AC:* `--features quipu` builds against a real quipu rev; the promote path
  reaches a live `/knot`. *Blocked by:* Phase-4 decision to unpin.
- **H-PROJECTION** ☐ Hot, one-directional projection of quipu
  `boundary:"action"` structural policies, with invalidation on quipu policy
  writes. *AC:* a policy added in quipu appears in hank's registry on the next
  refresh; hank never originates a policy definition. *Blocked by:* H-DEP.
- **H-EDIT-EVAL** ☐ Evaluate projected structural policies at `hook pre-edit`
  against the resident graph. *AC:* an edit violating a structural `deny` policy
  is blocked at edit time with a tier-tagged verdict. *Blocked by:*
  H-PROJECTION.
- **H-FRESHNESS** ☐ Serve FR-3 freshness so a projected-policy verdict declares
  fresh/stale of the registry it used. *AC:* a verdict computed against a stale
  projection is tagged stale, never silently `fresh`. *Blocked by:* Phase-3
  overlays + frontier-bounded incremental update.
- **H-PROMOTE-VERDICT** ☐ Promote hank edit-time verdicts into quipu as signed
  facts, extending the `commit → touched` promote path. *AC:* a hank verdict
  lands as a bitemporal quipu Verdict attributable to the hank verifier
  identity. *Blocked by:* H-DEP.
