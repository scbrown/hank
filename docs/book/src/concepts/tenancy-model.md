# The Tenancy Model

A team means there is no single "the AST." Each developer sits at some
branch/commit plus an uncommitted working delta, and those deltas diverge.
Hank resolves this with **shared base + per-tenant overlay (copy-on-write)**.

- **Shared base.** The full structural graph is computed once at a baseline
  commit and held read-only in memory.
- **Per-tenant overlay.** Each session gets a lightweight overlay: only touched
  files are re-parsed, only affected edges recomputed, layered over the base.
  Queries resolve against `base + overlay`. An overlay is invisible to other
  tenants — isolation is automatic.
- **Content-hash sharing.** With structural sharing, N developers cost *one
  base plus N small deltas*, not N full graphs.

## Blast radius is the incremental-update primitive

The overlay is not just the edited file — it is the edited file **plus its
frontier**. When a signature changes, every reference to it (possibly in files
the developer never opened) now has different facts. So the updater queries the
base graph for the references and dependents of the changed symbols — the *same*
reachability query that answers "what does this change affect?" for a consumer.

**One primitive, two uses — build it once.**

See the [Specification](../design/specification.md) §5.5 for the frontier
algorithm, eviction policy, and high-fan-in handling.

## Implementation state

The engine above exists as a library (hank #2): `graph::Base` (shared,
read-only, `Arc`-shared, built at a resolved commit with per-file content
hashes), `graph::Overlay` (owned re-parses of touched files only —
`O(touched)`, never `O(repo)`), and `graph::TenantRegistry` /
`graph::TenantView` (the per-query `base + overlay` composition, walked by the
same FR-12 BFS as every other graph). Isolation is structural: a view composes
exactly one tenant's overlay, the base is immutable, and interned parses are
shared by content hash (FR-15) without sharing any view state. The
`tests/overlay_isolation_tests.rs` suite pins §6.3 absolute isolation,
masking/revert/deletion, and the cost shape.

The [resident daemon](../reference/daemon.md) wires this live: it holds the
registry (base at the startup `HEAD`), `POST /edit` is the FR-30 feed (the
post-edit hook calls it per save), query endpoints take `tenant=`, and
`/status` reports the base commit and active overlays. Still open: the FR-16
frontier recompute (hank #3) — until then an overlay-NEW symbol name (one
with zero base definitions) cannot see its base callers — and FR-17/FR-18
watch integration and overlay lifecycle (hank #5/#6).
