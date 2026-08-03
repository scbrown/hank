# SARC Conformance — what hank × quipu still needs

Status: **analysis + build order.** Phases 1–3 are the agreed MVP; Phases 4–6
are scoped but not started. Nothing here is implemented yet.

## Why this document

Besanson's *SARC: A Governance-by-Architecture Framework for Agentic AI Systems*
(arXiv:2605.07728) proposes that constraints become a first-class specification
object alongside state, action space and reward:

```text
c = ⟨src, class, pred, verif, resp⟩  + a declared operating point θ
```

compiled into four named enforcement points in the agent loop — a **Pre-Action
Gate** (PAG), an **Action-Time Monitor** (ATM), a **Post-Action Auditor** (PAA),
and an **Escalation Router** (ER) — under eight runtime invariants (I1–I8) whose
joint effect is *specification-trace correspondence*: given a specification Σ
and a trace T, an auditor can mechanically decide `T ⊨ Σ` without access to the
model, its prompts, or its developers.

The stack is already most of the way there, and on the *specification* side it
is ahead of the paper's own prototype: quipu's `aegis:` governance ontology is
SHACL-validated and bitemporal, and verdicts are ed25519-signed against a
human-owned `VerifierRegistration` root of trust — SARC assumes a JSON spec file
and a Python checker.

What is missing is not the substrate. It is a specific, enumerable set of holes:
the constraint objects are under-declared, three of the four enforcement points
are absent or advisory-only, the trace is not derived from the specification,
and nothing checks correspondence. This document names them and orders the work.

## Where the stack stands

### Already in place

| SARC concept | Stack today | Where |
|---|---|---|
| Constraint specification object | `aegis:Policy` + `aegis:Selector` / `aegis:Predicate` atoms | `quipu/shapes/governance.ttl` |
| `pred` | `aegis:claim` (SPARQL ASK), or selector `.scm` + predicate regex | `quipu/shapes/policies/treesitter.ttl` |
| Pre-Action Gate | `hank hook pre-edit`, `Mode::{Off,Advise,Enforce}` | `src/hook/pre_edit.rs`, `src/policy.rs` |
| Policy-layer reference monitor | quipu pre-commit write gate | `quipu/src/governance/guard.rs` |
| Verdict as attestation, not claim | ed25519-signed, evidence-hash-bound `aegis:Verdict` | `src/verdict.rs`, `quipu/src/signing.rs` |
| Root of trust | `aegis:VerifierRegistration`, human-authored | `quipu/shapes/governance.ttl` |
| Latency budget (SARC §5.1) | `policy.deadline_ms`, fail-open on expiry | `src/policy.rs` |
| One-directional policy projection | quipu canonical → hank read cache | `src/project.rs`, `src/hook/rule_planes.rs` |
| Confidence inputs | `tier ∈ {live,lsp,tree-sitter,committed,attested}` + `freshness` | shapes, FR-3 |
| Layer discipline (SARC I6) | honoured by construction: no rule lives in the prompt | [Governance Plane](governance-plane.md) |

[Governance Plane](governance-plane.md) independently anticipates much of SARC —
risk × confidence adaptive effect, verdict integrity, the out-of-band verifier,
the `prevented`/`observed` enforcement gradient. SARC's marginal contribution on
top of it is **placement discipline** (which constraint class belongs at which
enforcement point) and **checkable correspondence** (a decidable audit).

### The gaps

**G1 — Constraint objects are incomplete.** Violates I2 ("a constraint missing
any field is not a constraint; it is a comment").

`aegis:Policy` carries `targets`, `claim`, `boundary ∈ {action,transition}` and
`effect ∈ {allow,warn,require-approval,deny,escalate,record}`. It does not carry:

- **`class ∈ {hard, soft, escalation}`.** `effect` conflates class with
  response, so "what kind of constraint is this" is not declarable and SARC's
  class→placement rules (Table 3) cannot be checked.
- **an operating point θ** — no false-positive / false-negative tolerance.
- **a reversibility window τ_rev** and on-timeout behaviour (needed by I4).
- **a per-constraint latency budget** at its verification point.
- **a hosting layer** (`orchestration|tool|policy`), needed to check I6.
- **`src.type`** — `aegis:Directive` supplies optional `authority`/`issuedBy`,
  and the shipped catalog sets neither.

**G2 — The verdict path is built but not wired.** Violates I3 and I8.

`src/verdict.rs` implements signing and `promote_verdict`, and it is correct —
it mirrors quipu's scheme exactly so a hank-signed verdict verifies under
quipu's root of trust. It has **no caller** outside `hank verdict-key` in
`src/cli.rs`. A pre-edit guard decision — the exact moment a constraint fires —
never becomes a governed fact. Symmetrically, quipu's own write-gate decision is
not persisted (`Q-VERDICT-PERSIST`, open). Today the only enforcement record is
a local, fail-silent JSONL spool.

**G3 — The trace is not derived from Σ.** Violates I3, and I8 by consequence.

`src/metrics.rs` emits `{kind, ts, agent, tenant, item, …}` per event. There is
no pre/post state, no `constraints_evaluated` set with outcomes, no attribution
tuple. `docs/work-scoped-governance.md` names this precisely — "records and
rules share one vocabulary" — and phases it. Its phase 1 *is* SARC's I3. It is
designed, not built.

**G4 — The Post-Action Auditor is advisory context, not a constraint site.**

`src/hook/post_edit.rs` injects blast-radius context after an edit. It evaluates
no constraint, emits no verdict, and cannot prevent the *next* action — which is
what a PAA is for. SARC's soft class has nowhere to live, and `throttle` — the
declared PAA response responsible for the paper's entire 89.5% soft-overage
result — is not in quipu's `effect` enum.

**G5 — There is no Escalation Router.** Violates I4.

`require-approval` and `escalate` currently fail closed at the quipu write gate
*with no channel to grant approval* (`guard.rs::effect_blocks`, and the design
doc says so plainly). `aegis:Decision` and `aegis:assignsWorkflow` shapes exist;
there is no runtime router, no operator group, no queue, no τ_rev, no
default-deny-on-timeout, no capacity model. SARC I4: "escalation without a bound
is not human oversight; it is deferred autonomy."

**G6 — There is no Action-Time Monitor.**

Nothing observes an action mid-flight. Long-running Bash, MCP tool calls and
sub-agent runs have no cumulative-budget monitor and no interrupt.
`src/hook/pre_bash.rs` is deliberately record-only and prints nothing.

**G7 — No attribution tuple, no authority intersection.** Violates I5.

SARC's `α = ⟨P, planner, executor, tool, auth, C_eval⟩` has no counterpart. The
spool carries one flat `agent` + `tenant` + `item`. There is no
principal-and-agent chain, no authority composition (`all-of` / `any-of`), no
monotonic narrowing under delegation — and the trace is a **sequence, not a
tree**, so orchestrator/worker runs are exposed to exactly SARC's
constraint-laundering and attribution-dilution failure modes. quipu's `group_id`
is documented as provenance-only, and [Governance Plane](governance-plane.md)
scopes v1 to a single trust domain. This is the deepest gap and the one with a
real prerequisite.

**G8 — Enforcement completeness is unmeasured.** Violates I7.

`docs/work-scoped-governance.md` §"What this cannot reach" is an honest,
explicit list of bypass surfaces: CI pipelines, cron, the far side of a remote
shell, a sibling session's VCS index, a hostile agent. I7 is a property of the
*dispatch graph*, verified by inspection against Σ — and there is no inventory
of governed vs ungoverned tool-call classes, so "which actions traverse an
enforcement point" is not answerable mechanically.

**G9 — No audit checker, no replay, no calibration.**

Nothing computes `T ⊨ Σ`. There is no spool reader, no replay harness, no
measured false-positive rate — so the advise→enforce promotion ladder in
`work-scoped-governance.md` §Evals cannot be walked, and θ (G1) would be
undeclarable-in-practice even once the field exists.

### Position on SARC's adoption ladder

- **Level 1** (PAG, hard constraints at the tool/policy layer, structured trace
  emission) — substantially met; I3 and I8 outstanding.
- **Level 2** (PAA, soft constraints with calibrated operating points) — not started.
- **Level 3** (ATM, ER with declared τ_rev) — not started.
- **Level 4** (multi-agent) — blocked on quipu multi-tenancy.

## Decisions

- **Vocabulary**: extend `aegis:` in place in `quipu/shapes/governance.ttl`
  rather than layering a separate `sarc:` overlay. One vocabulary; the existing
  SHACL validation and projection decode carry the new fields for free. Cost:
  the shipped `treesitter.ttl` catalog needs backfilling in the same change.
- **Escalation Router owner**: **quipu**. The engine of record already models
  `Decision`, `assignsWorkflow` and the bitemporal audit trail, and
  `require-approval` already fails closed there. Hank gets a thin client. This
  matches the settled "the engine lives in Quipu; hank never originates policy"
  rule.
- **MVP scope**: Phases 1–3 — SARC-conformant for a single agent in a single
  trust domain, which is exactly the v1 boundary
  [Governance Plane](governance-plane.md) already declared.

## Build order

### Phase 1 — Complete the constraint object

*Closes G1. Prerequisite for everything else: you cannot place a constraint by
class before class exists.*

**quipu — `shapes/governance.ttl`:**

- `aegis:constraintClass`, `sh:in ("hard" "soft" "escalation")`, required on
  `boundary "action"` policies.
- `aegis:verificationPoint`, `sh:in ("PAG" "ATM" "PAA" "tool_layer" "policy_layer")`.
  This replaces nothing — `boundary` stays as the coarse action/transition
  split; `verificationPoint` is the fine placement SARC needs.
  `tool_layer`/`policy_layer` already appear in hank's projected policies.
- `aegis:hostedAtLayer`, `sh:in ("orchestration" "tool" "policy")`. Deliberately
  **no `"prompt"` value**, so I6 is unrepresentable by construction rather than
  checked-and-rejected.
- `aegis:OperatingPoint` node shape + `aegis:operatingPoint` on `Policy`:
  `falsePositiveTolerance`, `falseNegativeTolerance`, `threshold`,
  `calibrationBasis`.
- `aegis:reversibilityWindowSeconds` and `aegis:onTimeout` — the latter
  `sh:in ("deny")`, one value only, so default-allow-under-load is not
  expressible. Required on escalation-class policies.
- `aegis:latencyBudgetMs` on `Policy`.
- `"throttle"` added to the `aegis:effect` enum, plus `aegis:backoffFormula`.
- `aegis:sourceType`, `sh:in ("regulatory" "contractual" "ethical" "operational")`,
  and `aegis:authority` required on action-boundary policies.

**quipu — `src/governance/placement.rs` (new):** a class↔placement conformance
pass, run at definition time alongside SHACL. `hard ⇒ verificationPoint ∈ {PAG,
ATM, tool_layer, policy_layer}`; `soft ⇒ {ATM, PAA}`; `escalation ⇒ {PAG, PAA}`
and must declare τ_rev. This is SARC Table 3 made mechanical, and it is what
"placement discipline" means in practice. Backfill `shapes/policies/treesitter.ttl`
(`no-ticket-in-comment` → hard/PAG, `todo-needs-ticket` → soft/PAA).

**hank:**

- `project_queries::POLICY_QUERY` gains `?constraintClass ?verificationPoint
  ?latencyBudgetMs ?fpTolerance ?fnTolerance ?reversibilityWindowSeconds` as
  **OPTIONAL**s. Not required — a projection that hard-required a field quipu
  had not yet backfilled would return zero rows, which is the exact
  both-sides-shipped-and-the-seam-returned-nothing failure `project_queries.rs`
  already documents in its own comments.
- `project::ProjectedPolicy` gains those fields alongside `effect`;
  `decode_policies` reads them through the existing `required`/`optional`
  closures. An unrecognised `constraintClass` is an `Error::Projection`,
  matching how an unknown `matchType` is handled — never a silent drop.
- `rules::Rule` gains `class` and `verification_point`, so a locally-configured
  `[[hank.policy.rules]]` and a projected policy stay one type.
- `hook/rule_planes.rs::governed_check` selects its response by declared class
  (hard ⇒ block under `Enforce`; soft ⇒ never block; escalation ⇒ route) rather
  than by `project::effect_blocks` alone. `Mode::Advise` keeps its ceiling: it
  never blocks, whatever the class.

### Phase 2 — Wire the verdict, derive the trace

*Closes G2 and G3 — the two gaps that make I8 impossible.*

1. Call `verdict::promote_verdict` from the pre-edit decision path. Every fired
   constraint emits one signed, evidence-hash-bound `aegis:Verdict`.
2. Carry the projection's real freshness into `aegis:freshness`.
   `rule_verdict_message` already threads it into the model-facing text;
   `verdict_turtle` currently hardcodes `"fresh"`, which is precisely the
   silent-fresh-tag the tier discipline exists to prevent.
3. Batch and buffer. Promotion must not sit on the edit's critical path —
   `deadline_ms` is 100 ms and a `/knot` round-trip is not. Spool locally with
   the existing fail-silent discipline; drain from the resident daemon.
4. Persist quipu's write-gate verdict too (`Q-VERDICT-PERSIST`), so the
   policy-layer monitor is as auditable as the orchestration-layer one.
5. Restructure the spool record to be **derived from Σ**:

   ```text
   { pre_state_ref, action, post_state_ref,
     constraints_evaluated: [{ id, class, verification_point, outcome,
                               response_taken }],
     attribution, reward_components }
   ```

   This is `work-scoped-governance.md` phase 1 with SARC's `E_i` and `α_i`
   named explicitly. Records and policies then share one vocabulary — the
   precondition for derive/test/explain in that document *and* for the checker
   in Phase 5.

### Phase 3 — Make the PAA a real enforcement point

*Closes G4.*

Give `hook/post_edit.rs` a constraint-evaluation path alongside its advisory
context: evaluate `verificationPoint "PAA"` policies against
`(pre, post, action, obs)`, emit verdicts, and implement the `throttle`
response — a declared backoff applied to *subsequent* actions once a soft window
is crossed. This is the single highest-leverage mechanism in SARC's evaluation
and the stack has no equivalent.

Soft constraints stay non-blocking by construction. The PAA's "prevents the
next, not the just-completed" semantics must be explicit in the module doc,
because presenting it otherwise is the false-`prevented` claim the enforcement
gradient exists to stop.

### Phase 4 — Escalation Router (quipu, hank client)

*Closes G5. Turns `require-approval` from fail-closed-with-no-channel into
bounded human oversight.*

- `aegis:OperatorGroup` shape: capacity model (`M/M/c` — `c`, `mean_service_s`),
  hours, after-hours mode, `fallback_if_unavailable "deny"`.
- Router service in quipu: accept a suspended action, dispatch a
  `DecisionRequest` to the group's queue, hold until τ_rev, **default-deny** on
  timeout. A ruling that *modifies* the action must re-enter the gate (SARC
  Algorithm 1's `goto PagCheck`) — re-validation, not trust.
- Decisions stay content-bound to the evidence hash (already the shape's
  contract), so approve-then-change goes stale automatically.
- Hank gets a thin ER client at the pre-edit seam. When the router is
  unreachable the escalation-class response is deny, and the fail-open notice
  says so loudly.
- Emit queue-depth / wait / utilisation metrics, because SARC's operative claim
  is that `W_q < τ_rev` is a *measurable* property, not an assertion.

### Phase 5 — Audit checker and enforcement inventory

*Closes G8 and G9 — the "auditable by construction" claim itself.*

- `quipu_audit_check(Σ, T)`: four passes per SARC Definition 2 — coverage,
  class-placement compatibility, outcome consistency, attribution completeness —
  returning a structured discrepancy report. Deterministic,
  predicate-language-agnostic, and explicitly **not** an LLM call: SARC §5.1's
  design rule, which is the same `O(ℓ_tool)` budget discipline hank already
  applies to its own guard.
- A dispatch-graph inventory for I7: enumerate every tool-call class the harness
  exposes, mark governed/ungoverned, fail the check when an executable class
  traverses no compatible enforcement point. Seed it from
  `work-scoped-governance.md` §"What this cannot reach", so the known-unreachable
  surfaces become **data** rather than prose.
- The replay/eval harness from `work-scoped-governance.md` §Evals — liveness,
  both-outcomes, non-vacuity, recoverability, replay. This is what makes the
  Phase-1 operating point θ an honest number rather than a declared one, and it
  gates advise→enforce promotion per rule.

### Phase 6 — Multi-agent

*Closes G7. Gated on quipu enforceable multi-tenancy — do not start before it.*

- Attribution tuple α on every trace record; the trace stored as a **tree**,
  worker subtrees attached to their dispatch node, never summarised.
- Authority intersection along the call chain, monotonically non-increasing;
  an empty intersection fails safe.
- Constraint inheritance with **decidability rescue**: evaluate an inherited
  constraint at the deepest layer where it remains decidable, or escalate —
  never silently drop it, which is the constraint-laundering path.
- Trust-boundary tagging on imported state (Bobbin retrieval results, MCP tool
  output, sub-agent responses) with a PAA trust predicate. SARC's zero-trust
  agent gateway, expressed in the existing constraint vocabulary rather than as
  a separate perimeter layer.

## Files this touches

**quipu**

- `shapes/governance.ttl` — the constraint-object extensions (Phase 1)
- `shapes/policies/treesitter.ttl` — backfill the shipped catalog (Phase 1)
- `src/governance/guard.rs` — class-aware effects, verdict persistence
- `src/governance/placement.rs` *(new)* — class↔placement conformance pass
- `src/governance/router.rs` *(new)* — escalation router (Phase 4)
- `src/governance/audit.rs` *(new)* — the `T ⊨ Σ` checker (Phase 5)
- `src/signing.rs` — reused unchanged; it is already the root of trust
- `docs/design/policy-edit-hooks.md` — the SARC gaps land in its backlog table

**hank**

- `src/project_queries.rs`, `src/project.rs` — projection of the new fields
- `src/rules.rs` — `Rule` gains class + verification point
- `src/hook/rule_planes.rs` — response selection by declared class
- `src/hook/pre_edit.rs` — verdict emission on decision
- `src/hook/post_edit.rs` — PAA constraint evaluation + throttle (Phase 3)
- `src/metrics.rs`, `src/audit.rs` — Σ-derived trace record (Phase 2)
- `src/verdict.rs` — real freshness and batching; the signing scheme is correct
- `src/daemon/` — verdict drain, ER client (Phase 4)
- `docs/work-scoped-governance.md` — reconcile its phasing with this one

## Verification

Beyond each repo's normal gate:

- **quipu** — `cargo test governance`: the extended catalog must still conform;
  new tests that a class↔placement mismatch is *rejected at write* (the
  definition-time half of the discipline), and that an escalation-class policy
  without τ_rev fails validation.
- **hank** — `just check && just test`, plus two-sided fixtures through the real
  hook binary per `work-scoped-governance.md` §Evals: a RED case and a GREEN
  case per new constraint class, and a non-vacuity mutation check.
- **End to end** — a policy authored in quipu, projected into hank, fired at
  pre-edit, promoted back as a signed verdict, and accepted by
  `quipu_audit_check` against the same Σ. That round trip *is* SARC's
  decidable-audit property, and it is the acceptance test for Phases 1–2.

**One caveat travels with any number this produces:** replay measures false
positives only on traffic that actually happened, and measures no false
negatives at all. A seeded adversarial corpus gives coverage against known
attacks and none against novel ones.

## Out of scope

The other papers in the source set — the Deloitte/Informatica semantic-layer
whitepaper, Agent-OM, the LLM-driven KG-construction framework, the
ontological-grounding work, and the agentic-governance literature review — bear
on how Σ gets *authored* and kept aligned with its sources, not on runtime
enforcement. They are relevant to the authoring surface in
[Governance Plane](governance-plane.md) §Authoring (particularly the "mined"
modality and the translation layer SARC §6 presupposes but does not specify).
They are deliberately not addressed here.
