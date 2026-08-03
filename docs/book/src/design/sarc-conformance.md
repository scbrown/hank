# SARC Conformance — what hank × quipu still needs

Status: **Phase 1 landed** (quipu `Q-SARC-CLASS` + `Q-SARC-PLACEMENT`, and the
hank projection that consumes them). Phases 2–3 complete the MVP; Phases 4–6
are scoped but not started. See [Build order](#build-order) for what each
phase covers and [Phase 1, as built](#phase-1-as-built) for what shipped.

## Why this document

Besanson [\[SARC\]](#sources) proposes that constraints become a first-class
specification object alongside state, action space and reward (§3.1):

```text
c = ⟨src, class, pred, verif, resp⟩  + a declared operating point θ
```

compiled into four named enforcement points in the agent loop (§4.1) — a
**Pre-Action Gate** (PAG), an **Action-Time Monitor** (ATM), a **Post-Action
Auditor** (PAA), and an **Escalation Router** (ER) — under eight runtime
invariants I1–I8 (§3.5) whose joint effect is *specification-trace
correspondence* (Definition 2, §3.6): given a specification Σ and a trace T, an
auditor can mechanically decide `T ⊨ Σ` in `O(|T|·|C|)` without access to the
model, its prompts, or its developers.

The stack is already most of the way there, and on the *specification* side it
is ahead of the paper's own prototype: quipu's `aegis:` governance ontology is
SHACL-validated and bitemporal, and verdicts are ed25519-signed against a
human-owned `VerifierRegistration` root of trust — SARC's reference artifact is
a JSON spec file and a Python checker (§3.6, §13.4).

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
| Latency budget ([SARC] §5.1) | `policy.deadline_ms`, fail-open on expiry | `src/policy.rs` |
| One-directional policy projection | quipu canonical → hank read cache | `src/project.rs`, `src/hook/rule_planes.rs` |
| Confidence inputs | `tier ∈ {live,lsp,tree-sitter,committed,attested}` + `freshness` | shapes, FR-3 |
| Layer discipline (SARC I6) | honoured by construction: no rule lives in the prompt | [Governance Plane](governance-plane.md) |

[Governance Plane](governance-plane.md) independently anticipates much of SARC —
risk × confidence adaptive effect, verdict integrity, the out-of-band verifier,
the `prevented`/`observed` enforcement gradient. SARC's marginal contribution on
top of it is **placement discipline** ([SARC] §4.2, Table 3: which constraint
class belongs at which enforcement point) and **checkable correspondence**
([SARC] §3.6: a decidable audit). SARC is explicit that this is a *specification
discipline* layered over a policy-as-code substrate rather than a replacement
for one (§2.1) — which is exactly the relationship quipu's write gate already
has to hank's projection.

### The gaps

**G1 — Constraint objects are incomplete.** Violates I2 ([SARC] §3.5: "a
constraint missing any field is not a constraint; it is a comment").

`aegis:Policy` carries `targets`, `claim`, `boundary ∈ {action,transition}` and
`effect ∈ {allow,warn,require-approval,deny,escalate,record}`. It does not carry:

- **`class ∈ {hard, soft, escalation}`.** `effect` conflates class with
  response, so "what kind of constraint is this" is not declarable and the
  class→placement rules ([SARC] Table 3) cannot be checked.
- **an operating point θ** — no false-positive / false-negative tolerance.
- **a reversibility window τ_rev** and on-timeout behaviour (needed by I4).
- **a per-constraint latency budget** at its verification point.
- **a hosting layer** (`orchestration|tool|policy`), needed to check I6.
- **`src.type`** — `aegis:Directive` supplies optional `authority`/`issuedBy`,
  and the shipped catalog sets neither.

**G2 — The verdict path is built but not wired.** Violates I3 and I8 ([SARC]
§3.5).

`src/verdict.rs` implements signing and `promote_verdict`, and it is correct —
it mirrors quipu's scheme exactly so a hank-signed verdict verifies under
quipu's root of trust. It has **no caller** outside `hank verdict-key` in
`src/cli.rs`. A pre-edit guard decision — the exact moment a constraint fires —
never becomes a governed fact. Symmetrically, quipu's own write-gate decision is
not persisted (`Q-VERDICT-PERSIST`, open). Today the only enforcement record is
a local, fail-silent JSONL spool.

**G3 — The trace is not derived from Σ.** Violates I3 ([SARC] §3.5: "the trace
is generated; it is not reconstructed"), and I8 by consequence.

`src/metrics.rs` emits `{kind, ts, agent, tenant, item, …}` per event. There is
no pre/post state, no `constraints_evaluated` set with outcomes, no attribution
tuple. `docs/work-scoped-governance.md` names this precisely — "records and
rules share one vocabulary" — and phases it. Its phase 1 *is* SARC's I3. It is
designed, not built.

**G4 — The Post-Action Auditor is advisory context, not a constraint site.**

`src/hook/post_edit.rs` injects blast-radius context after an edit. It evaluates
no constraint, emits no verdict, and cannot prevent the *next* action — which is
what a PAA is for ([SARC] §4.1). SARC's soft class `C_s` has nowhere to live,
and `throttle` — the declared PAA response responsible for the paper's entire
89.5% soft-overage reduction ([SARC] §10.3, Table 6) — is not in quipu's
`effect` enum.

**G5 — There is no Escalation Router.** Violates I4 ([SARC] §3.5, §5.3).

`require-approval` and `escalate` currently fail closed at the quipu write gate
*with no channel to grant approval* (`guard.rs::effect_blocks`, and the design
doc says so plainly). `aegis:Decision` and `aegis:assignsWorkflow` shapes exist;
there is no runtime router, no operator group, no queue, no τ_rev, no
default-deny-on-timeout, no capacity model. [SARC] I4: "escalation without a
bound is not human oversight; it is deferred autonomy." The queueing model that
makes `W_q < τ_rev` measurable rather than asserted is §5.3.

**G6 — There is no Action-Time Monitor.**

Nothing observes an action mid-flight. Long-running Bash, MCP tool calls and
sub-agent runs have no cumulative-budget monitor and no interrupt.
`src/hook/pre_bash.rs` is deliberately record-only and prints nothing.

**G7 — No attribution tuple, no authority intersection.** Violates I5 ([SARC]
§9.3, §9.6).

SARC's `α = ⟨P, planner, executor, tool, auth, C_eval⟩` has no counterpart. The
spool carries one flat `agent` + `tenant` + `item`. There is no
principal-and-agent chain, no authority composition (`all-of` / `any-of`), no
monotonic narrowing under delegation — and the trace is a **sequence, not a
tree**, so orchestrator/worker runs are exposed to exactly the
constraint-laundering and attribution-dilution failure modes of [SARC] §9.5.
quipu's `group_id` is documented as provenance-only, and
[Governance Plane](governance-plane.md) scopes v1 to a single trust domain. This is the deepest gap and the one with a
real prerequisite.

**G8 — Enforcement completeness is unmeasured.** Violates I7 ([SARC] §3.5).

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

### Position on the adoption ladder ([SARC] §13.3)

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
and must declare τ_rev. This is [SARC] Table 3 made mechanical, and it is what
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

### Phase 1, as built

Shipped in quipu (`shapes/governance.ttl`, `src/governance/placement.rs`) and
hank (`src/constraint.rs`, `src/project_decode.rs`,
`src/hook/rule_planes.rs`). Three things came out differently from the plan
above, each because building it surfaced something the analysis had not:

**Two fields are unrepresentable rather than validated.** `aegis:hostedAtLayer`
has no `"prompt"` value and `aegis:onTimeout` has only `"deny"`. Both started as
rules the placement pass would enforce; both are better as gaps in the
vocabulary, because a check that can be configured wrong eventually is, and
neither of these has a legitimate second value to preserve.

**Multi-valued fields are refused, not resolved.** Asserting
`constraintClass "hard"` over an existing `"soft"` leaves *both* facts active —
assertion is not replacement. The first implementation read the last SPARQL row
and silently picked one, so a re-class would have landed while the old placement
still validated. This was caught by the write-path test, not by the unit tests
over the rule table, which is the argument for having both. A policy with two
classes is now refused as ambiguous, with the retract-in-the-same-transaction
remedy in the message, and `a_clean_re_placement_retracting_the_old_value_lands`
is the recoverability half — refusing ambiguity is only safe if there is a way
to legitimately move a policy.

**The projection decoder got the same collapse the text decoder already had.**
`POLICY_QUERY` gained three OPTIONALs, and SPARQL returns the cross product of
them: a policy carrying two `rdfs:label`s comes back as two rows and became two
identical rules. `decode_text_rules` already carried a comment recording this
exact failure on the live catalogue — 7 entities projecting as 11 rules, 4
duplicates, each reported twice to the model with conflicting rationales.
`decode_policies` was exposed to it the whole time and the new fields made it
likelier, so it now collapses on the policy IRI and refuses rows that disagree
on a required field. Identity is the IRI, not the label: an unlabelled policy
falls back to a row-indexed name, so keying on the name would give every row a
distinct identity and collapse nothing.

Two behaviour changes worth knowing about when reading a verdict:

- **The declared class outranks the governed effect.** A `soft` policy never
  blocks, even with `effect "deny"` — that combination is contradictory, quipu's
  placement check now refuses to define it, and honouring what the author
  declared it to *be* is the only reading that is not a guess. A policy with no
  class (projected from a catalog predating the field) still behaves exactly as
  before: the effect decides.
- **A policy declared at the PAA does not fire at the pre-edit gate.** It is
  skipped, not evaluated-and-ignored. Evaluating it there would tell the model
  to fix something its author scoped to after the fact — and until Phase 3 lands
  the post-edit auditor, such a policy is *not evaluated at all*. That is the
  honest state, and it is visible in the projection rather than hidden.

`Mode::Advise` remains a ceiling over all of it: an advise-mode deployment never
blocks, whatever class a projected constraint declares. That is what makes
staging a new hard constraint safe before anyone has measured its
false-positive rate.

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

   This is `work-scoped-governance.md` phase 1 with [SARC] §3.6's `E_i` and
   §9.6's `α_i`
   named explicitly. Records and policies then share one vocabulary — the
   precondition for derive/test/explain in that document *and* for the checker
   in Phase 5.

### Phase 3 — Make the PAA a real enforcement point

*Closes G4.*

Give `hook/post_edit.rs` a constraint-evaluation path alongside its advisory
context: evaluate `verificationPoint "PAA"` policies against
`(pre, post, action, obs)`, emit verdicts, and implement the `throttle`
response — a declared backoff applied to *subsequent* actions once a soft window
is crossed. This is the single highest-leverage mechanism in the [SARC]
evaluation
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
- Emit queue-depth / wait / utilisation metrics, because the operative claim of
  [SARC] §5.3 is that `W_q < τ_rev` is a *measurable* property of an M/M/c
  queue, not an assertion.

### Phase 5 — Audit checker and enforcement inventory

*Closes G8 and G9 — the "auditable by construction" claim itself.*

- `quipu_audit_check(Σ, T)`: four passes per [SARC] Definition 2 (§3.6) —
  coverage,
  class-placement compatibility, outcome consistency, attribution completeness —
  returning a structured discrepancy report. Deterministic,
  predicate-language-agnostic, and explicitly **not** an LLM call: [SARC] §5.1's
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
  output, sub-agent responses) with a PAA trust predicate. The zero-trust agent
  gateway of [SARC] §9.5, expressed in the existing constraint vocabulary rather
  than as a separate perimeter layer.

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
  `quipu_audit_check` against the same Σ. That round trip *is* the
  decidable-audit property of [SARC] Property 1, and it is the acceptance test
  for Phases 1–2.

**One caveat travels with any number this produces:** replay measures false
positives only on traffic that actually happened, and measures no false
negatives at all. A seeded adversarial corpus gives coverage against known
attacks and none against novel ones.

## Out of scope — and why

Five of the six sources this analysis was drawn from bear on how Σ gets
*authored* and kept aligned with its sources, not on how it is enforced at
runtime. That distinction matters: [SARC] §6 is explicit that the arrow from
obligation to predicate is an **institutional process, not a technical step**,
and that the framework "presupposes rather than resolves" it. So the authoring
work is real, and it is a different piece of work.

Where each one lands, should we pick that thread up:

- **[Raji & Bashir]** surveys the governance requirements Σ has to encode, and
  is the clearest statement of the principal-agent problem behind G7 — who is
  the principal, and what is the agent authorized to do on their behalf. Its
  Singapore MGF summary ("assess and bound the risks upfront", "make humans
  meaningfully accountable") maps onto the risk map and the ER respectively.
- **[Informatica/Deloitte]** argues the semantic-layer case Quipu already
  embodies. Useful as external corroboration for why the governed graph is the
  right home for policy, not as a source of requirements.
- **[Agent-OM]** is directly applicable to keeping `aegis:` aligned with
  external vocabularies as they drift — the maintenance half of the translation
  layer, and the thing that stops predicates silently decaying away from their
  source obligations ([SARC] §6, "the translation layer as a governed control
  surface").
- **[Peshevski et al.]** is the agent-driven ontology-construction pattern
  behind the "mined" authoring modality in
  [Governance Plane](governance-plane.md) §Authoring.
- **[Olivares-Alarcos et al.]** grounds *explanation* generation in an ontology
  while keeping the reasoning sound — relevant to making a refusal legible, which
  is a stated requirement of the recoverability eval in
  `docs/work-scoped-governance.md` §Evals ("every refusal names the command that
  satisfies it").

None of them changes the runtime gap list above, which is why they are named
here rather than folded into it.

## Sources

- **[SARC]** — Besanson, G. (2026). *SARC: A Governance-by-Architecture
  Framework for Agentic AI Systems: Compiling Regulatory Obligations into
  Runtime Constraints*. Working paper, Universidad Torcuato Di Tella.
  [arXiv:2605.07728v1](https://arxiv.org/abs/2605.07728) [cs.SE].
  Reference artifacts: <https://github.com/besanson/sarc-governance>.
  All section, table, definition and invariant references in this document are
  to this paper.
- **[Raji & Bashir]** — Raji, M. & Bashir, M. (2026). *Towards Agentic AI
  Governance: A Preliminary Assessment*. AIR-RES 2026, Springer Nature.
  [arXiv:2607.07612v1](https://arxiv.org/abs/2607.07612).
- **[Informatica/Deloitte]** — Beierschoder, M., Andrensek, J. & Rebele, T.
  *Building the Semantic Data Layer for Agentic AI*. Informatica / Deloitte
  whitepaper (5340en).
- **[Agent-OM]** — Qiang, Z., Wang, W. & Taylor, K. (2024). *Agent-OM:
  Leveraging LLM Agents for Ontology Matching*. PVLDB 18(3), 516–529.
  [doi:10.14778/3712221.3712222](https://doi.org/10.14778/3712221.3712222).
- **[Peshevski et al.]** — Peshevski, D., Stojanov, R. & Trajanov, D. (2025).
  *AI Agent-Driven Framework for Automated Product Knowledge Graph Construction
  in E-Commerce*. [arXiv:2511.11017v1](https://arxiv.org/abs/2511.11017)
  [cs.AI].
- **[Olivares-Alarcos et al.]** — Olivares-Alarcos, A., Ahsan, M., Sanjaya, S.,
  Lin, H.-I. & Alenyà, G. *Ontological grounding for sound and natural robot
  explanations via large language models*.
  [arXiv:2602.13800v1](https://arxiv.org/abs/2602.13800).

Internal design documents this analysis builds on, all in-tree:

- [Governance Plane](governance-plane.md) — the verification spine, verdict
  integrity, risk × confidence, the Hank↔Quipu integration contract.
- [Policy edit hooks](policy-edit-hooks.md) — evidence locality, the quipu
  pre-commit gate, the hank projection, and the `Q-*` / `H-*` backlog the
  `Q-SARC-*` beads extend.
- [Tiers and Freshness](../concepts/tiers-and-freshness.md) — the confidence
  inputs SARC's operating point composes over.
- `docs/work-scoped-governance.md` — the trace taxonomy, the five eval
  properties, and the per-rule promotion ladder. Out of the book by design;
  cited by path.
