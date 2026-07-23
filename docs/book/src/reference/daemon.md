# Resident Daemon

`hank daemon` is the Phase-3 resident engine (FR-31): it builds the base graph
**once**, holds it in memory, and answers queries over a local HTTP API
(FR-27). The hook and the MCP graph tools become thin clients of it — the
per-invocation `CodeGraph` build that dominates their latency disappears, which
is what makes the sub-100ms guard budget reachable.

```bash
hank daemon              # build the graph, serve on [hank.serve] bind/port
hank daemon --port 4050  # override the port
```

The daemon binds `[hank.serve] bind_address:mcp_http_port` (default
`127.0.0.1:3040`). It refuses to start if the graph cannot be built — it never
serves a graph it does not hold. SIGINT/SIGTERM drain in-flight requests and
exit 0: a supervisor's `stop` is a clean stop, and the shutdown is announced on
stderr, not silent.

## Who uses it

Clients consult the daemon only when `[hank.serve] use_daemon = true` (default
`false`), and even then only after verifying via `/status` that the daemon
serves **the same root** they are asking about — a daemon holding repo B must
never answer for repo A.

- **`hank hook pre-edit`** sizes edits via `POST /measure`. If the daemon is
  down, the guard **fails open and says so** — the edit proceeds and the
  session gets a loud once-per-session notice. Down = allowed AND announced,
  never silently allowed, never blocked (see
  [Pre-Edit Policy Guard](policy-guard.md)).
- **`hank hook post-edit`** extracts the edited file's symbols fresh (their
  content is what just changed) and asks the resident graph for their external
  callers via `/callers`. Daemon down = transient fallback with a stderr note —
  silent to the model, because the advisory is advice, not enforcement.
- **MCP graph tools** (`hank_callers`, `hank_callees`, `hank_impact`,
  `hank_references`) answer from the resident graph when the request is not
  `path`-scoped; otherwise, and on any daemon failure, they fall back to the
  transient build. Fallback here is silent to the model (a transient answer is
  equally correct, just slower); absence still goes to stderr for the operator.

## HTTP API

Every reply carries its provenance `tier` (FR-3). Facts come from the graph
built at daemon startup — `/status.uptime_secs` says how old that snapshot is.
Freshness tags (the FR-3 second half) arrive with overlay updates (hank #3);
replies omit them rather than faking `fresh`.

| Route | Method | Query/body | Answer |
|-------|--------|------------|--------|
| `/health` | GET | — | bare `ok` (the liveness probe) |
| `/status` | GET | — | root, node/edge counts, uptime, tiers |
| `/callers` | GET | `symbol` | direct callers, from the resident graph |
| `/callees` | GET | `symbol` | direct callees, from the resident graph |
| `/impact` | GET | `symbol`, `hops` (default 5) | transitive blast radius |
| `/references` | GET | `symbol` | definition sites, from the resident index |
| `/symbols` | GET | `file` (root-relative) | that file's symbols, line order |
| `/dataflow` | GET | `function`, `path?`, `var?`, `forward?`, `hops?` | intra-procedural data dependence |
| `/measure` | POST | `{file, rel, anchors[], max_hops?}` | edit blast-radius sizing for the guard |

Notes that keep the answers honest:

- **`found` vs empty.** "The symbol is not in the graph" and "the symbol has no
  callers" are different answers; every reply separates them. `/symbols` is the
  one surface that cannot: files enter the graph only through their symbols, so
  `known: false` covers absent, unparseable, and symbol-less files alike —
  render it as "no symbols in the resident graph", never "the file is empty".
- **Root confinement.** `/measure` names a file to read and `/dataflow` a
  subtree to build; both refuse (400) any path resolving outside the daemon's
  root. A localhost daemon must not become an arbitrary-file reader.
- **`/dataflow` is not resident.** Dataflow has no resident model yet
  (hank #22); the daemon computes it per request so the surface is complete
  rather than silently partial. The reply shape will not change when a resident
  model arrives.

## Failure semantics

The client seam reports "daemon not reachable" as its own state that callers
must handle — it can never be folded into an empty answer. What each caller
does with it differs by contract:

| Caller | Daemon down means |
|--------|-------------------|
| pre-edit guard | edit **allowed** + loud notice (fail-open, announced) |
| MCP graph tools | transient rebuild, stderr note (correct, slower) |
| anything parsing `/status` | absent/unparseable = not a usable daemon |

The guard's taxonomy keeps "the guard ran and found nothing" distinct from
"the guard could not run" end to end; killing the daemon is the cheapest
bypass, so its absence must always be visible.
