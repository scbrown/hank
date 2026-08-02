//! pre_bash — the input path the action resolver never had.
//!
//! `crate::action::resolve` was written for gap 2 of the trace phase and shipped
//! **unreachable**: hank supported only `PreEdit`/`PostEdit`, the only live
//! wiring was `post-edit` on `Write|Edit`, and the fleet's Bash guard chain
//! referenced hank zero times. So a resolver with tests and documentation sat in
//! the tree with no caller and no way to acquire a command line — present,
//! plausible, and structurally incapable of running.
//!
//! That is precisely the failure class this epic exists to prevent (a control
//! that cannot fail, believed to be working), occurring inside the epic's own
//! first phase. This module is the correction: a hook event that receives the
//! harness's Bash payload and feeds it to the resolver.
//!
//! RECORD-ONLY. This never denies, never warns, and prints NOTHING — not even
//! on a resolved dangerous-looking command. Enforcement on the action path is a
//! later phase gated on evals, and a hook that started advising here would be
//! enforcement arriving without the gate. It also shares the Bash matcher with
//! the existing guard chain, so anything printed would interleave with a guard
//! whose refusals are load-bearing.
//!
//! ALWAYS EXIT 0. A bookkeeping hook that can fail a command is worse than no
//! bookkeeping: it converts an observability feature into an outage. Every path
//! here returns Ok, and the emit itself is fail-silent by construction.
//!
//! WHAT IS RECORDED, and why `target_class` is present even when unknown:
//!
//! ```text
//! {"kind":"action","target_class":"host","verb":"ssh","target":"build-01",
//!  "agent":"…","tenant":"…","item":"…","ts":…}
//! ```
//!
//! `verb` and `target` are OMITTED when the resolver abstained — an absent field
//! is honestly silent, whereas `""` or `"unknown"` would be replayed later as if
//! it were a value. But `target_class` is ALWAYS written, including
//! `"unknown"`, because the replay in the next phase needs to divide resolved
//! actions by TOTAL actions. Without a row for the abstentions the denominator
//! is invisible and a resolver covering 5% of traffic looks identical to one
//! covering 95%.

use std::io::Read;

use crate::action;

/// Handle a Claude Code `PreToolUse` payload for the Bash tool.
///
/// Reads the payload on stdin, resolves the command to (verb, target,
/// `target_class`), and records it. Errors are swallowed: an unparseable or
/// unexpected payload records nothing and still succeeds.
pub fn run_pre_bash() -> anyhow::Result<()> {
    let mut buf = String::new();
    // A read failure is not an error worth surfacing: the command the operator
    // asked for must run either way.
    std::io::stdin().lock().read_to_string(&mut buf).ok();
    if let Some(cmd) = command_of(&buf) {
        record(&action::resolve(&cmd));
    }
    Ok(())
}

/// Pull `tool_input.command` out of a harness payload.
///
/// Returns `None` for anything that is not a Bash-shaped payload, which
/// includes the case where this hook is wired to a matcher that also delivers
/// other tools: recording a non-command as a command would put fiction in the
/// trace, and the trace's only value is that it is evidence.
#[must_use]
pub fn command_of(payload: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(payload).ok()?;
    let cmd = v.get("tool_input")?.get("command")?.as_str()?;
    if cmd.trim().is_empty() {
        return None;
    }
    Some(cmd.to_string())
}

/// Emit one `action` record. Fail-silent via the spool's own contract.
fn record(a: &action::Action) {
    let mut fields: Vec<(&str, serde_json::Value)> =
        vec![("target_class", a.target_class.as_str().into())];
    if let Some(v) = &a.verb {
        fields.push(("verb", v.clone().into()));
    }
    if let Some(t) = &a.target {
        fields.push(("target", t.clone().into()));
    }
    crate::metrics::emit("action", &fields);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_a_bash_command() {
        let p = r#"{"tool_name":"Bash","tool_input":{"command":"ssh build-01 uptime"}}"#;
        assert_eq!(command_of(p).as_deref(), Some("ssh build-01 uptime"));
    }

    #[test]
    fn non_bash_payloads_yield_nothing() {
        // Both directions: the positive above proves the extractor works, so
        // these Nones mean "correctly declined", not "extractor is broken".
        assert_eq!(command_of(r#"{"tool_input":{"file_path":"/a/b.rs"}}"#), None);
        assert_eq!(command_of(r#"{"tool_name":"Bash"}"#), None);
        assert_eq!(command_of("{not json"), None);
        assert_eq!(command_of(""), None);
    }

    #[test]
    fn a_blank_command_is_not_a_command() {
        assert_eq!(command_of(r#"{"tool_input":{"command":"   "}}"#), None);
    }

    #[test]
    fn resolution_reaches_the_resolver() {
        // The point of this module is that the resolver becomes REACHABLE.
        // Asserting payload -> resolve end to end is the test that would have
        // failed for as long as action.rs had no caller.
        //
        // The host carries an explicit user@, because the resolver deliberately
        // refuses a bare single-word operand as too weak to claim. My first
        // version of this test asserted that `ssh build-01 uptime` resolves and
        // it FAILED — correctly. The expectation was wrong, not the resolver.
        let cmd = command_of(r#"{"tool_input":{"command":"ssh deploy@build-01 uptime"}}"#).unwrap();
        let a = action::resolve(&cmd);
        assert_eq!(a.verb.as_deref(), Some("ssh"));
        assert_eq!(a.target.as_deref(), Some("build-01"));
        assert_eq!(a.target_class.as_str(), "host");
    }

    #[test]
    fn a_bare_host_operand_is_deliberately_refused() {
        // The abstain rule, asserted so a later "improvement" that loosens it
        // has to argue with a test instead of a comment.
        let a = action::resolve("ssh build-01 uptime");
        assert!(a.verb.is_none());
        assert_eq!(a.target_class.as_str(), "unknown");
    }

    #[test]
    fn an_unresolvable_command_still_carries_a_class() {
        // The denominator case: abstentions must remain countable.
        let a = action::resolve("frobnicate --wibble");
        assert_eq!(a.target_class.as_str(), "unknown");
        assert!(a.verb.is_none());
    }
}
