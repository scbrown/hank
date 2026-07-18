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
