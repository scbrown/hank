# Configuration Reference

Hank reads the `[hank]` table of `.bobbin/config.toml`. All keys are optional;
unspecified keys fall back to the defaults shown below.

```toml
[hank]
# Baseline the shared read-only graph is built at.
base_ref = "main"
# Run the LSP tier for precise facts where a build resolves.
enable_lsp = true
# Run the CPG/dataflow tier (Phase 2).
enable_cpg = false
# Languages to extract (defaults to Bobbin's grammar set).
languages = ["rust", "typescript", "python", "go", "java", "cpp"]

[hank.freshness]
# Debounce for keystroke-driven tree-sitter updates (ms).
debounce_ms = 300
# When to compute LSP facts: "save" | "on_demand".
lsp_on = "save"

[hank.tenancy]
max_overlays = 32
# Symbols with fan-in above this get special frontier handling.
high_fanin_threshold = 200
# "on_session_close" | "lru".
overlay_eviction = "on_session_close"

[hank.serve]
bind_address = "127.0.0.1"
# Distinct from Bobbin's server and Quipu's 3030.
mcp_http_port = 3040
# Write guard for the broker / promotion endpoints.
read_only = false

[hank.quipu]
enabled = false
# "commit" | "merge" | "manual".
promote_on = "merge"
# "named_graph" (preferred, needs Quipu quads) | "qualifier" (fallback).
branch_model = "named_graph"
shapes_path = "shapes/"
```
