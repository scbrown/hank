# Harness Integration (Claude Code Hooks)

Hank can plug into an agent harness so it responds to edits **synchronously**,
the way a language server reacts to a keystroke. The agent's edit tool call *is*
the change event; a harness hook feeds it to Hank, and Hank replies inline — no
tool call for the agent to remember.

For Claude Code this uses [hooks](https://docs.claude.com/en/docs/claude-code/hooks).

## Post-edit advisory (available now)

`hank hook post-edit` reads the `PostToolUse` payload on stdin and, when the
edited file has symbols called from *other* files, returns a cross-file
blast-radius advisory as injected context.

Wire it in `.claude/settings.json`:

```json
{
  "hooks": {
    "PostToolUse": [
      {
        "matcher": "Edit|Write|MultiEdit",
        "hooks": [
          { "type": "command", "command": "hank hook post-edit" }
        ]
      }
    ]
  }
}
```

After the agent edits `src/auth.rs`, it sees something like:

```text
Hank (tree-sitter): your edit to src/auth.rs touches symbol(s) with callers
elsewhere — re-check these still compile.
  authenticate <- 3 caller(s)
  verify_token <- 1 caller(s)
Impacted files: src/api/login.rs, src/api/session.rs
```

The advisory is emitted only when there is cross-file impact, and the hook never
fails the harness (no output = nothing to say).

## Pre-edit guard (planned, Phase 5)

`hank hook pre-edit` (`PreToolUse`) will verify the *proposed* buffer before the
edit lands and — for capability-scoped agents — optionally block it with a
reason. Blocking is opt-in; the default stays advisory.

## Performance note

This prototype builds the call graph transiently on each invocation. Once the
Phase-3 resident per-tenant overlay lands, the hook becomes a thin client of the
`hank serve` daemon and meets the sub-100ms budget a synchronous guard needs.
See [the specification](../design/specification.md) §5.9 (FR-30/FR-31).
