# Configuration

Hank shares the stack's `.bobbin/config.toml` under a `[hank]` table, with the
same resolution order Quipu uses: compiled defaults are overlaid by the user
config (`~/.config/bobbin/config.toml`), then the project's
`.bobbin/config.toml`. CLI flags win over all of them.

See the full [Configuration Reference](../reference/config.md) for every key and
its default.

```toml
[hank]
base_ref = "main"
# Restricts `hank analyze` to these languages.
languages = ["rust", "typescript", "python", "go", "java", "cpp"]

[hank.serve]
bind_address = "127.0.0.1"
mcp_http_port = 3040
# When true, hank refuses mutating operations (promotion).
read_only = false

[hank.quipu]
enabled = false
branch_model = "named_graph"
```

`enable_lsp`, `enable_cpg`, and the `[hank.tenancy]` limits exist but are not yet
read — see the [Configuration Reference](../reference/config.md), where each is
marked with the phase that will honour it.
