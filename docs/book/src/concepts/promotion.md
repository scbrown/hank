# Promotion to Quipu

Hank holds the volatile, per-tenant, in-flight reality. When changes land on a
shared branch (commit/merge), the corresponding facts are **promoted** into
Quipu as a new bitemporal state — valid-time = commit time, transaction-time =
when learned. Quipu holds the settled, governed, versioned record; Hank holds
what's in flight. Uncommitted churn never pollutes the governed graph.

## Export — the governed projection

`hank export` is the projection Hank promotes: the **precise, typed referential
structure** (modules, symbols, `definedIn`/`calls`/`imports`, and — as the
markdown extractor lands — `Document`/`Section` + `references`), emitted as RDF
Turtle in the `bobbin:` ontology. This is **not** Bobbin's chunking; it is
structure for reasoning and governance.

Module dependencies (`bobbin:imports`, `CodeModule → CodeModule`) are resolved
from `use`/`mod` declarations at the tree-sitter tier — best-effort by module
stem, so they over-connect on shared names like any tree-sitter-tier fact; the
`lsp` tier refines them.

```bash
hank export src --repo myrepo --format turtle    # dump the referential graph
hank promote --commit HEAD --to http://quipu.svc .  # SHACL-validate + write it
```

`hank promote` needs `--features quipu`; without it the binary says so rather
than pretending. It emits the Turtle, SHACL-validates in-process against
`shapes/`, and writes to `/knot` **only if it conforms** — a rejected promotion
exits non-zero so a script can't read it as landed.

Code and docs are one referential graph (spec §5.10): code leans real-time (the
live graph + edit hook), docs lean asynchronous (this export). Once in Quipu,
doc rot becomes a SPARQL query — "every `Document` referencing a `CodeSymbol`
that no longer exists."

- Facts are emitted as Turtle in the existing `bobbin:` code ontology and
  **SHACL-validated before write** — Hank never writes to Quipu without passing
  the shapes in `shapes/code-entities.ttl` (extended with edge shapes).
- Writes go through Quipu's existing surface (`quipu_knot` / `POST /knot` /
  `Store::transact`), honoring `valid_from`/`valid_to`, `actor`, and `source`
  (the commit SHA).

## Querying it back — dependency and blast radius

Once promoted, the dependency graph is queryable in Quipu. Store the **direct**
edges (`bobbin:calls`, `bobbin:imports`) and let SPARQL property paths do the
transitive work — never pre-compute and store a transitive closure that goes
stale. These queries are verified against live Quipu (`POST /query`, JSON body
`{"query": "…"}`).

**What does a symbol depend on?** (one hop)

```sparql
PREFIX bobbin: <http://aegis.gastown.local/ontology/>
SELECT ?dep WHERE { ?s bobbin:name "hbiw_alpha" . ?s bobbin:calls ?dep }
```

**Blast radius — what breaks if a symbol changes?** The transitive set of callers,
the `+` property path (this is the "if X dies, what breaks?" query; assert its
*members*, not a nonzero count):

```sparql
PREFIX bobbin: <http://aegis.gastown.local/ontology/>
SELECT ?affected WHERE { ?t bobbin:name "hbiw_beta" . ?affected bobbin:calls+ ?t }
```

Code entities do **not** suffer the alias-fragmentation that afflicts the
human-named infrastructure graph (a blast-radius query over fragmented nodes
returns a confident *subset*, worse than nothing): Hank mints one deterministic
IRI per symbol (`…/code/<repo>/<file>::<symbol>`), so re-promotion updates the
same node rather than minting a synonym, and the `calls+` closure is complete.

## Branches as named graphs

Each branch's committed facts belong in an RDF **named graph**, bitemporally
versioned within. Quipu is a triple store today, so this is tracked as an
additive, default-graph-preserving quad-store extension —
[scbrown/quipu#36](https://github.com/scbrown/quipu/issues/36). Until it lands,
Hank can fall back to a branch qualifier. See the
[Specification](../design/specification.md) §9 for the ontology extension and
the quad-store RFC sketch.
