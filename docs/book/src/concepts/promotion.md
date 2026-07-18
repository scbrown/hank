# Promotion to Quipu

Hank holds the volatile, per-tenant, in-flight reality. When changes land on a
shared branch (commit/merge), the corresponding facts are **promoted** into
Quipu as a new bitemporal state — valid-time = commit time, transaction-time =
when learned. Quipu holds the settled, governed, versioned record; Hank holds
what's in flight. Uncommitted churn never pollutes the governed graph.

## Export — the governed projection

`hank export` is the projection Hank promotes: the **precise, typed referential
structure** (modules, symbols, `definedIn`/`calls`, and — as the markdown
extractor lands — `Document`/`Section` + `references`), emitted as RDF Turtle in
the `bobbin:` ontology. This is **not** Bobbin's chunking; it is structure for
reasoning and governance.

```bash
hank export src --repo myrepo --format turtle   # dump the referential graph
# hank export --to quipu                         # promote it (Phase 4)
```

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

## Branches as named graphs

Each branch's committed facts belong in an RDF **named graph**, bitemporally
versioned within. Quipu is a triple store today, so this is tracked as an
additive, default-graph-preserving quad-store extension —
[scbrown/quipu#36](https://github.com/scbrown/quipu/issues/36). Until it lands,
Hank can fall back to a branch qualifier. See the
[Specification](../design/specification.md) §9 for the ontology extension and
the quad-store RFC sketch.
