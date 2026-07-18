# Promotion to Quipu

Hank holds the volatile, per-tenant, in-flight reality. When changes land on a
shared branch (commit/merge), the corresponding facts are **promoted** into
Quipu as a new bitemporal state — valid-time = commit time, transaction-time =
when learned. Quipu holds the settled, governed, versioned record; Hank holds
what's in flight. Uncommitted churn never pollutes the governed graph.

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
