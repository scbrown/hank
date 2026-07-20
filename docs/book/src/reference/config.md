# Configuration Reference

Hank reads the `[hank]` table of `.bobbin/config.toml`. All keys are optional;
unspecified keys fall back to the defaults shown below.

```toml
[hank]
# Baseline the shared read-only graph is built at.
base_ref = "main"
# (Phase 2/3 — not yet read) Run the LSP tier for precise facts where a build resolves.
enable_lsp = true
# (Phase 2 — not yet read) Run the CPG/dataflow tier.
enable_cpg = false
# Languages to extract. RESTRICTS `hank analyze`: a file whose language is not
# listed is not counted.
languages = ["rust", "typescript", "python", "go", "java", "cpp"]

[hank.freshness]
# Debounce for keystroke-driven tree-sitter updates (ms).
debounce_ms = 300
# (LSP tier — not yet read) When to compute LSP facts: "save" | "on_demand".
lsp_on = "save"

[hank.tenancy]
# (Phase 3 — not yet read) Maximum concurrent per-tenant overlays over one base.
max_overlays = 32
# (Phase 3 — not yet read) Symbols with fan-in above this get special frontier handling.
high_fanin_threshold = 200
# (Phase 3 — not yet read) "on_session_close" | "lru".
overlay_eviction = "on_session_close"

[hank.serve]
bind_address = "127.0.0.1"
# Distinct from Bobbin's server and Quipu's 3030.
mcp_http_port = 3040
# Write guard: when true, hank REFUSES mutating operations (promotion) with a
# distinguishable error. The served MCP/HTTP surface is read-only regardless
# today; this guards the write path and any future served write.
read_only = false

# (Phase 4) Quipu promotion. `promote_on` and `shapes_path` are not yet read.
[hank.quipu]
enabled = false
# "commit" | "merge" | "manual".
promote_on = "merge"
# "named_graph" (preferred, needs Quipu quads) | "qualifier" (fallback).
branch_model = "named_graph"
shapes_path = "shapes/"

[hank.policy]
# "off" (inert) | "advise" (report only) | "enforce" (deny).
mode = "off"
# Wall-clock budget for the whole pre-edit guard (ms). Expiry => allow.
deadline_ms = 100
# Warn the user, once per session, when the guard fails open.
notify_on_fail_open = true
# How far to follow the call graph when sizing an edit.
max_hops = 5

# Per-tenant capability scopes, keyed by tenant/role id. A tenant with no entry
# is unconstrained. See "Pre-Edit Policy Guard" for the full contract.
[hank.policy.scopes.polecat-3]
allow_paths = ["src/**", "tests/**"]   # empty = any path
deny_paths = ["src/config.rs"]         # beats allow_paths
max_impacted_symbols = 25
max_impacted_files = 10
```

An unrecognized `mode` is a config **error**, not a silently inert guard.
