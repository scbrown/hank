# Governance Plane — Policy, Workflows & Verification

Status: **design / exploratory.** This describes a capability that layers on the
Hank × Quipu × Bobbin stack. It is not yet built. It fixes the primitives and
the trust model so implementation can proceed without re-litigating them.

## Motivation

The stack today is a **knowledge plane** — Hank (live code structure), Quipu
(governed bitemporal record), Bobbin (search / fusion / RAG). Its job is to
*know things about the code and feed context to agents*.

Missing is a **governance plane**: the layer that decides *what a fleet of
agents should be doing, and what they are allowed to do*. Today that intent
lives in skills — advisory checklists an agent may freely ignore. The goal is to
make it **load-bearing**:

- **Workflows propel** the agent — soft but pervasive, always driving toward the
  next step.
- **Policy constrains** the agent — hard but targeted, a wall at the boundaries
  that matter.

The governance plane *uses* the knowledge plane; it is not part of it. Policies
query Hank/Quipu/Bobbin for the facts they evaluate over.

## The verification spine

"How is an artifact verified?" and "what is a policy?" are the same question. A
policy **is** a verification predicate. So the plane is not three sibling
concepts (artifact, workflow, policy) — it is **one recursive primitive,
verification**, with policy as the verifier at each node.

Verification composes bottom-up:

```text
Workflow    — verified iff its Steps are (per its order / dependencies)
  └ Step     — verified iff its required Artifacts are + its gate policy holds
     └ Artifact — verified iff its Reference, dereferenced, satisfies its Claim   ← leaf
```

Only the **leaf** touches reality — it dereferences to concrete evidence from
the knowledge plane. Everything above is composition of leaf verdicts.

## Primitives

The minimal vocabulary. `Artifact` / `Step` / `Workflow` / `Policy` are *types*
of `Entity`; the rest are base primitives.

- **Entity** — a governed, typed node in Quipu (bitemporal, provenanced).
- **Reference** — an entity's handle to a *concrete target*: a commit, a file, a
  Hank code-symbol IRI, a URL. The entity is the governed abstraction; the
  reference points at the real thing.
- **Claim** — the verifiable assertion an entity carries (e.g. *"exists and is
  approved"*).
- **Evidence** — facts that bear on a claim, drawn from the knowledge plane
  (Hank), the record (Quipu), or external systems (git, CI). Evidence is either
  **canonical** (read directly from a source of truth) or an **attestation** (a
  signed statement from the system that owns the fact).
- **Verifier (= Policy)** — a **pure predicate** mapping evidence → verdict
  `{satisfied | unsatisfied | unknown}`. `unknown` (no evidence yet) is distinct
  from `unsatisfied` (evidence says no). Policies are **external**: a policy
  *targets an entity type*, the way a SHACL shape targets a class. One policy
  governs all entities of that type.
- **Verdict** — the result, **stored** as a governed bitemporal fact. Carries a
  `tier` and `freshness` (see [Tiers and Freshness](../concepts/tiers-and-freshness.md)),
  is signed, is bound to the evidence it saw, and is reproducible (see
  [Verdict integrity](#verdict-integrity)).
- **Actor / Role** — who performs a step. Enables fleet-level assignment and
  separation-of-duties policies (*"the reviewer must differ from the author"*).
- **Binding / Gate** — attaches a policy to an *enforcement point* with an
  *effect* (`block` / `warn` / `require-approval`). The policy stays pure; the
  effect is contextual — `tests-green` may `block` at a workflow exit and merely
  `warn` elsewhere.
- **Instance (Run)** — a definition gone live for a specific run (this bead, this
  session), holding live state + verdicts. Distinct from the definition it
  instantiates.
- **Catalog leaf** — the composable atoms policies are built from, of two kinds
  (see [Catalog](#catalog-initial)): a **selector** (evidence → a *set*, e.g.
  `changed-symbols`) and a **predicate** (element → bool, e.g. `has-test`).
  Registered, evidence-grounded, tier-tagged.
- **Parameterization** — definitions are *templates*. References and actors are
  *patterns* (`repo://{repo}/docs/design/{bead}.md`, role `reviewer`) resolved
  when an Instance binds its parameters (bead, repo, agent).
- **Temporal trigger** — a scheduled re-verification for verdicts that depend on
  *elapsed time* (SLAs, deadlines), which reactive fact-change invalidation does
  not cover.

### Composition types

- **Artifact** — an entity standing for a thing produced or required (design doc,
  test result, ADR, commit, review). Verified at the leaf.
- **Step** — one unit of work: `{ produces: [Artifact], requires: [Artifact],
  gated-by: [Policy], actor: Role }`.
- **Transition** — a step → step edge, fired when the source step's verdicts are
  satisfied.
- **Workflow** — an ordered / dependency graph of Steps.

## Verdict integrity

A bare "satisfied ✓" written into the record is forgeable by anyone who can
write a fact. The plane defends against the **agent** (including a compromised
one), not the human operator — the human is the trust root. A verdict is
therefore an **attestation, not a claim**, with four properties:

- **Authenticity** — every verdict is signed by a *registered* verifier
  identity. Quipu accepts a verdict-fact only if signed by a verifier registered
  for that predicate. An agent cannot mint Hank's signature.
- **Binding** — the verdict names the *content hash* of the evidence it saw.
  Change the referent → hash changes → the stored verdict is automatically
  stale. Kills tamper-after-the-fact and replay.
- **Reproducibility** — because the predicate is pure and deterministic and the
  evidence is content-addressed, any independent verifier can re-run it over the
  same evidence and must get the same result. Verdicts are **checked, not
  trusted**. (Purity is the security property, not just clean design.) The
  verdict retains `evidence-hash + predicate-id` so third parties can re-derive.
- **Evidence provenance** — every evidence input is either **canonical** (Hank
  reads the source of truth directly from its own live map — the agent's edit
  *is* the evidence, nothing to fake) or a **signed attestation** from the owning
  system (CI signs "green over hash H"; the approver signs the approval). The
  chain terminates at a root; nothing in it is an agent's self-report.

**Architectural constraint:** the verifier must run **out-of-band from the agent
it verifies** — a service the agent *calls*, not a library the agent *hosts* —
or the agent runs its own verifier over a doctored repo and signs anything. The
gatekeeper sits outside the gate.

**Root of trust:** this reduces "trust every verdict" to "trust the verifier
*keys* and the *registry* of which identity may attest which predicate." That
registry is a governed, human-authored definition. The scheme does not eliminate
trust; it concentrates it into a small, human-owned surface and makes everything
downstream reproducible.

## Roles in the stack

- **Hank — live verifier, gatekeeper, guide.** Holds the code in memory, so it
  evaluates code-grounded claims *before commit* at `tier=live`, and that live
  verdict stream *is* the propulsion: it tells the agent which claims are not yet
  green. It enforces gates (via a harness adapter) and injects workflow context.
  It also **exports its verification capabilities** into the catalog — predicates
  like `has-tests`, `blast-radius-within`, `doc-section-exists` that only the
  live map can evaluate. Runs as a **service**, out-of-band from the agent.
- **Quipu — entity / definition / verdict store + committed verifier.** Holds the
  entities, references, and durable bitemporal verdicts; verifies the committed
  tier; enforces definition-time well-formedness via SHACL; keeps the audit
  trail.
- **Bobbin — authoring intelligence + surfacing.** Powers conversational
  authoring (drafting in the vocabulary of *this* codebase) and surfaces
  governance state to agents.
- **Harness adapters — propel / constrain actuation.** Thin per-harness shims
  (Claude Code hooks; a fleet daemon; a generic governance-MCP + tool-call
  proxy). Enforcement strength is a **gradient**, tagged like a tier:
  `prevented` (harness exposes a pre-action gate) vs `observed` (detect,
  record, escalate). Never present `observed` as `prevented`.

## Authoring

Two distinct "creations" — do not conflate:

- **Definition** — authoring a workflow / policy *template*. Governed,
  human-driven, written to Quipu. This is what "authoring" means below.
- **Instance** — a definition going live for a run. Automatic, runtime, held in
  Hank's overlay + Quipu state. Not authored.

### Creation is composition over a catalog

Underneath authoring sits a **catalog** of atomic, pre-verified,
evidence-grounded **selectors and predicates** (see [Catalog](#catalog-initial)).
Authoring is composition three layers deep, bottoming out in the catalog:

```text
Workflow  = ordered graph of Steps
  Step    = { produces, requires, gated-by: [Policy], actor: Role }
  Policy  = { targets: EntityType, claim: <composed catalog predicates> }   (external)
  Catalog predicate = atomic verifier bound to an evidence source           (leaf)
```

Nobody writes raw predicates to build a workflow — they **assemble named
pieces**. That is what makes authoring both user-oriented and safe (cf. a
Semgrep rule registry, Terraform modules). The authoring *modalities* are
interchangeable front-ends over this same composition:

- **Conversational** — describe the process to a builder agent; it decomposes to
  a workflow + inferred policies, renders a draft, iterates. Powered by Bobbin
  retrieval.
- **Visual** — phases as nodes, gates as policy-chips; edit rules via a guided
  builder. Fits Quipu's embeddable web components.
- **Guided rule builder** — WHO / WHAT / WHEN (condition over facts) / THEN
  (effect). Never raw predicate syntax.
- **Mined** — the plane observes fleet behavior in Quipu's bitemporal log and
  *proposes* a codification of a recurring sequence; a human approves.

### The composition grammar

The grammar over the catalog is small: a **selector** yields a set, a
**predicate** tests an element, and **combinators** (`and` / `or` / `not`) plus
**quantifiers** (`all` / `any`) bind them. A composed policy — a fitness
function — reads:

```text
∀ s ∈ changed-public-symbols(run) : has-test(s)
      └─ selector (Hank)             └─ predicate (Hank)
targets Step[implement].exit-gate  ·  effect: block
```

Definitions are **templates**: references and actors are patterns
(`repo://{repo}/docs/design/{bead}.md`, role `reviewer`) that an Instance
resolves when it binds its parameters (bead, repo, agent). Authoring is over
roles and patterns; runtime resolves them.

### Dry-run against history

Because predicates are pure and reproducible, a draft policy is replayed over
past evidence *before* promotion — *"this would have blocked 3 of the last 20
merges"* — via Quipu's counterfactual `speculate()` over the bitemporal log. You
tune against reality, then promote; the same replay validates a **mined**
proposal. No governance rule goes live blind.

### Creation is itself verified

A definition is an Entity with the Claim *"well-formed"* — acyclic step graph,
every referenced policy exists in the catalog, every `produces` / `requires`
resolves. That claim is checked at write time by **SHACL** (definition-time
verification), the same spine one altitude up:

- **Definition-time** — *is this workflow well-formed?* (SHACL, at write, Quipu)
- **Run-time** — *is this workflow satisfied by reality?* (stored verdicts, at
  execution, Hank + Quipu)

### Catalog growth and sovereignty

- **Catalog + escape hatch.** 90% is composition of named predicates; an expert
  may author a *raw* predicate (a graph query / Hank capability) and **register
  it back** into the catalog as a new named primitive. A newly registered
  predicate is itself an entity that must be verified/reviewed before it enters
  the catalog — creation-is-verified, recursively.
- **Sovereignty.** Three producers — human composes, agent proposes, system mines
  — all emit a candidate that becomes governing only after definition-time
  verification **and** human promotion. The thing that governs the agents cannot
  be silently rewritten by an agent.

## Catalog (initial)

A starting set — deliberately small, meant to be refined. Every entry is
evidence-grounded and tier-tagged; the escape hatch grows it. `run` is the bound
Instance context.

### Selectors (evidence → set)

- `changed-files(run)` — files touched — *git / Hank overlay*
- `changed-symbols(run)` — symbols touched — *Hank*
- `changed-public-symbols(run)` — exported subset of the above — *Hank*
- `blast-radius(change)` — dependents / frontier of a change — *Hank*
- `callers(symbol)` / `callees(symbol)` — *Hank*
- `sinks-reached(source)` — taint / dataflow reachable sinks — *Hank (cpg)*
- `referencing-docs(symbol)` / `referenced-symbols(section)` — doc↔code — *Hank*
- `required-artifacts(step)` / `prior-steps(step)` — *Quipu (definition)*

### Predicates (element → bool)

Code-grounded, live tier (*Hank*):

- `has-test(symbol)` · `symbol-exists(ref)` · `doc-section-exists(ref)`
- `blast-radius-within(change, scope)` · `in-scope(action, scope)`
- `sanitized-before(source, sink-class)` — no unsanitized taint path

Record-grounded, committed tier (*Quipu*):

- `approval-exists(entity, by-role)` · `decision-record-exists(change)`
- `different-actor-than(step-a, step-b)` · `artifact-verified(artifact)`
- `within-duration(event, dur)` — depends on a temporal trigger

Attested by external owners:

- `tests-green(artifact)` · `build-succeeds(commit)` — *CI (signed)*
- `commit-exists(ref)` — *git* · `no-secrets(change)` — *scanner (signed)*

### Combinators

- boolean: `and` · `or` · `not`
- quantifiers over a selector: `all` · `any`

## Execution model

Per run, the plane drives the loop through the harness adapter:

```text
turn
  ├─ propel     inject "phase, done, next step, unmet claims"  (derived: definition × live verdicts)
  ├─ constrain  each action → policy gate → allow / ask / deny  (PreToolUse-equivalent)
  ├─ advance    observe completion → verify → advance state machine
  └─ gate       on attempted stop → exit-criteria verdicts met?
                  ├─ no  → block, return remaining steps → agent continues
                  └─ yes → transition / complete → promote run to Quipu (audit)
```

The injected guide content is **not authored separately** — it is *derived* from
`definition × current verdicts`. Verdicts are derived facts: when a referent
changes, the verdict goes `stale` and staleness propagates up the artifact →
step → workflow chain (Quipu's reactive reasoner + Hank freshness).

## Open questions

- **Home of the plane** — a new peer repo, or grown inside Quipu? (Orchestrating
  live agents is a different responsibility than recording facts.) Working name:
  *the loom*.
- **Instance state store** — a fast runtime store that promotes completed runs to
  Quipu, vs. Quipu's append-only log directly.
- **Enforcement floor** — is `observed` acceptable for harnesses that cannot
  hard-gate, or must every governed action route through a proxy that *can*
  prevent?
- **Task → workflow binding** — router by task type / files-touched, declared
  override, or repo-default.
- **Catalog authority** — who holds the privilege to register a new catalog
  predicate.
- **Compensation** — do runs with irreversible side effects need defined undo
  (Saga-style), or is halt-and-escalate enough? (Deferred: halt-and-escalate for
  v1.)
