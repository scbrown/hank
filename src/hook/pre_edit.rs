//! The `PreToolUse` policy guard — a blocking, capability-scoped edit check.
//!
//! This is where the §5.8 trust boundary becomes concrete: an agent's edit tool
//! call is intercepted *before* it lands, sized against the tenant's capability
//! scope (FR-25), and denied with a readable reason when it exceeds it.
//!
//! Everything here is arranged around one invariant: **fail open**. The harness
//! launches every crew agent through this hook, so a guard that fails closed
//! bricks the fleet the moment Hank is unavailable. Only a policy decision
//! blocks an edit; every error, timeout, and unknown degrades to "allow". See
//! `docs/book/src/reference/policy-guard.md` for the pinned contract.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use super::measure::{measure_within, relative, Measured};
use super::{deny_envelope, first_notice_for_session, system_message, HookInput};
use crate::config::HankConfig;
use crate::policy::Mode;

/// What the guard decided — the value the CLI turns into stdout.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// Allow the edit, silently. Exit 0, empty stdout.
    Allow,
    /// Block the edit with this model-facing reason.
    Deny(String),
    /// Allow the edit but tell the user something (advise mode, or fail-open).
    Notify(String),
}

/// Run the `pre-edit` guard: read the harness payload from stdin, decide, and
/// print at most one JSON object. Always returns `Ok` — the process must exit 0
/// so the harness never treats the guard as a fail-closed block.
pub fn run_pre_edit(tenant: Option<&str>) -> anyhow::Result<()> {
    let mut buf = String::new();
    std::io::stdin().lock().read_to_string(&mut buf).ok();
    let root = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));

    match guard(&buf, &root, tenant) {
        Outcome::Allow => {}
        Outcome::Deny(reason) => println!("{}", deny_envelope(&reason)),
        Outcome::Notify(message) => println!("{}", system_message(&message)),
    }
    Ok(())
}

/// Decide an edit. Pure apart from reading the repo, so it is directly testable.
#[must_use]
pub fn guard(input_json: &str, default_root: &Path, tenant: Option<&str>) -> Outcome {
    let started = Instant::now();

    // An unparseable payload is an ALLOW: the guard only speaks up about edits
    // it genuinely understands.
    let Some(input) = HookInput::parse(input_json) else {
        return Outcome::Allow;
    };
    let Some(file_path) = input.tool_input.file_path.clone() else {
        return Outcome::Allow;
    };
    let root = input.root(default_root);

    let config = match HankConfig::load(&root) {
        Ok(config) => config,
        Err(e) => return fail_open(&input, "config", &format!("unreadable config ({e})")),
    };

    // No scope for this tenant — mode is off, or the tenant is unconstrained.
    let (Some(tenant), Some(scope)) = (tenant, config.policy.scope_for(tenant)) else {
        return Outcome::Allow;
    };

    // A scope whose globs do not compile is misconfigured; say so rather than
    // quietly under-enforcing it.
    let glob_errors = scope.glob_errors();
    if !glob_errors.is_empty() {
        let detail: Vec<String> = glob_errors
            .iter()
            .map(|(pattern, why)| format!("`{pattern}` ({why})"))
            .collect();
        return fail_open(
            &input,
            "globs",
            &format!(
                "scope for tenant `{tenant}` has malformed path globs: {}",
                detail.join(", ")
            ),
        );
    }

    let file = PathBuf::from(&file_path);
    let rel = relative(&file, &root);

    // 1. Path scope — cheap, no graph needed, so it runs even under a blown
    //    deadline. This is the check that must never be skipped.
    if let Some(violation) = scope.check_path(&rel, tenant) {
        return decide(config.policy.mode, violation.message);
    }

    // 2. Blast radius — expensive. Bounded by whatever remains of the budget.
    let budget = Duration::from_millis(config.policy.deadline_ms)
        .checked_sub(started.elapsed())
        .unwrap_or_default();
    if scope.max_impacted_symbols.is_none() && scope.max_impacted_files.is_none() {
        return Outcome::Allow; // Nothing to measure against.
    }
    let radius = match measure_within(&root, &file, &rel, &input, config.policy.max_hops, budget) {
        Measured::Radius(radius) => radius,
        // NOTHING TO SIZE. Not a gap — allow, and stay quiet. The graph is Rust-only
        // today, so every .py/.ts edit arrives here; warning would be noise from day
        // one and would bury the case below.
        Measured::NotMeasurable => return Outcome::Allow,
        // THE DEADLINE EXPIRED, so the blast-radius rules DID NOT RUN. Still allow —
        // that contract is deliberate and stays (hank #35: a stale hank once blocked
        // every edit in the fleet, and a guard that fails CLOSED is worse than one
        // that fails open). What changes is that it is no longer SILENT.
        //
        // Silence here was indistinguishable from a clean pass, so a repo simply
        // being large turned its own enforcement off and nothing said so. MEASURED
        // 2026-07-20 against the shipped default of 100 ms: quipu (33k lines) takes
        // 152 ms and bobbin (69k lines) 313 ms, so on those trees this branch is
        // taken on EVERY edit. The same edit that is denied with a real budget was
        // allowed with empty output at the default — same repo, same policy,
        // opposite enforcement, no visible difference.
        Measured::TimedOut => {
            return fail_open(
                &input,
                // Per-repo: a session touching two oversized trees must hear about both.
                &format!("deadline-{}", root.display()),
                &format!(
                    "blast-radius deadline exceeded ({} ms) — size rules did NOT run for `{rel}`; \
                     raise hank.policy.deadline_ms or the guard is off on this repo",
                    config.policy.deadline_ms
                ),
            )
        }
    };

    match scope.check_blast(radius, &rel, tenant) {
        Some(violation) => decide(config.policy.mode, violation.message),
        None => Outcome::Allow,
    }
}

/// Turn a violation into an outcome according to the enforcement mode.
fn decide(mode: Mode, message: String) -> Outcome {
    match mode {
        Mode::Enforce => Outcome::Deny(message),
        // Advise: report what would have been denied, but never block.
        Mode::Advise => Outcome::Notify(format!("hank (advise, not blocking): {message}")),
        Mode::Off => Outcome::Allow,
    }
}

/// Degrade to "allow", loudly. Writes the stderr line the contract requires and,
/// once per session, a user-visible notice — because a hook's stderr is
/// surfaced only on exit `2`, so stderr alone would be silent in practice.
fn fail_open(input: &HookInput, kind: &str, reason: &str) -> Outcome {
    eprintln!("hank: policy guard UNMEASURED: {reason}");
    // `kind` keeps distinct gaps from muting each other — see first_notice_for_session.
    if first_notice_for_session(input.session_id.as_deref(), kind) {
        return Outcome::Notify(format!(
            "hank: policy guard UNMEASURED ({reason}) — this edit was NOT checked, and \
             edits like it are UNGUARDED this session."
        ));
    }
    Outcome::Allow
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A repo where `leaf` is called from three other files.
    fn wide_repo() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("leaf.rs"), "fn leaf() {}\n").unwrap();
        for (i, name) in ["one", "two", "three"].iter().enumerate() {
            std::fs::write(
                dir.path().join(format!("caller{i}.rs")),
                format!("fn {name}() {{ leaf(); }}\n"),
            )
            .unwrap();
        }
        dir
    }

    fn write_policy(dir: &Path, body: &str) {
        let bobbin = dir.join(".bobbin");
        std::fs::create_dir_all(&bobbin).unwrap();
        std::fs::write(bobbin.join("config.toml"), body).unwrap();
    }

    /// A session id unique to this call, so the once-per-session fail-open
    /// marker (a file in the temp dir, which outlives the test process) cannot
    /// leak state between tests or between `cargo test` runs.
    fn unique_session() -> String {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        format!(
            "test-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        )
    }

    fn edit_payload(dir: &Path, file: &str, old: &str) -> String {
        serde_json::json!({
            "session_id": unique_session(),
            "cwd": dir.to_str().unwrap(),
            "tool_name": "Edit",
            "tool_input": {
                "file_path": dir.join(file).to_str().unwrap(),
                "old_string": old,
                "new_string": "fn leaf() { changed(); }",
            },
        })
        .to_string()
    }

    #[test]
    fn allows_when_no_policy_is_configured() {
        let dir = wide_repo();
        let payload = edit_payload(dir.path(), "leaf.rs", "fn leaf() {}");
        assert_eq!(guard(&payload, dir.path(), Some("t")), Outcome::Allow);
    }

    #[test]
    fn allows_when_mode_is_off_despite_a_scope() {
        let dir = wide_repo();
        write_policy(
            dir.path(),
            "[hank.policy]\nmode = \"off\"\n[hank.policy.scopes.t]\nmax_impacted_symbols = 0\n",
        );
        let payload = edit_payload(dir.path(), "leaf.rs", "fn leaf() {}");
        assert_eq!(guard(&payload, dir.path(), Some("t")), Outcome::Allow);
    }

    #[test]
    fn denies_an_out_of_scope_path() {
        let dir = wide_repo();
        write_policy(
            dir.path(),
            "[hank.policy]\nmode = \"enforce\"\n\
             [hank.policy.scopes.t]\nallow_paths = [\"caller*.rs\"]\n",
        );
        let payload = edit_payload(dir.path(), "leaf.rs", "fn leaf() {}");
        let Outcome::Deny(reason) = guard(&payload, dir.path(), Some("t")) else {
            panic!("expected a deny");
        };
        assert!(reason.contains("leaf.rs"));
        assert!(reason.contains("outside the writable capability scope"));
    }

    #[test]
    fn denies_an_edit_that_exceeds_the_blast_radius() {
        let dir = wide_repo();
        write_policy(
            dir.path(),
            "[hank.policy]\nmode = \"enforce\"\ndeadline_ms = 30000\n\
             [hank.policy.scopes.t]\nmax_impacted_files = 1\n",
        );
        let payload = edit_payload(dir.path(), "leaf.rs", "fn leaf() {}");
        let Outcome::Deny(reason) = guard(&payload, dir.path(), Some("t")) else {
            panic!("expected a deny");
        };
        // leaf is called from three files; the ceiling is one.
        assert!(reason.contains("3 files (ceiling 1)"), "got: {reason}");
    }

    #[test]
    fn allows_an_edit_within_the_blast_radius() {
        let dir = wide_repo();
        write_policy(
            dir.path(),
            "[hank.policy]\nmode = \"enforce\"\ndeadline_ms = 30000\n\
             [hank.policy.scopes.t]\nmax_impacted_files = 10\n",
        );
        let payload = edit_payload(dir.path(), "leaf.rs", "fn leaf() {}");
        assert_eq!(guard(&payload, dir.path(), Some("t")), Outcome::Allow);
    }

    #[test]
    fn advise_mode_never_denies() {
        let dir = wide_repo();
        write_policy(
            dir.path(),
            "[hank.policy]\nmode = \"advise\"\ndeadline_ms = 30000\n\
             [hank.policy.scopes.t]\nmax_impacted_files = 1\n",
        );
        let payload = edit_payload(dir.path(), "leaf.rs", "fn leaf() {}");
        let Outcome::Notify(message) = guard(&payload, dir.path(), Some("t")) else {
            panic!("expected an advisory, not a block");
        };
        assert!(message.contains("not blocking"));
    }

    #[test]
    fn an_untouched_tenant_is_unconstrained() {
        let dir = wide_repo();
        write_policy(
            dir.path(),
            "[hank.policy]\nmode = \"enforce\"\n\
             [hank.policy.scopes.other]\nallow_paths = [\"nothing/**\"]\n",
        );
        let payload = edit_payload(dir.path(), "leaf.rs", "fn leaf() {}");
        assert_eq!(guard(&payload, dir.path(), Some("t")), Outcome::Allow);
        // ...and so is an agent with no tenant at all.
        assert_eq!(guard(&payload, dir.path(), None), Outcome::Allow);
    }

    #[test]
    fn a_blown_deadline_allows_the_edit_but_SAYS_SO() {
        // aegis-nz2x. Two claims, and both matter.
        //
        // It still ALLOWS: the fail-open contract is deliberate and stays. hank #35
        // was a stale hank blocking every edit in the fleet, and a guard that fails
        // closed is worse than one that fails open. This test is what stops a future
        // "make the deadline strict" change from re-paying that bill.
        //
        // And it is now AUDIBLE. It used to return a bare Allow — byte-identical to
        // a clean pass — so a repo large enough to blow the budget turned its own
        // blast-radius rules off and nothing said so. MEASURED against the shipped
        // default of 100 ms: quipu (33k lines) 152 ms, bobbin (69k lines) 313 ms, so
        // this branch is live on real repos today, not on a hypothetical one.
        let dir = wide_repo();
        write_policy(
            dir.path(),
            "[hank.policy]\nmode = \"enforce\"\ndeadline_ms = 0\n\
             [hank.policy.scopes.t]\nmax_impacted_files = 1\n",
        );
        let payload = edit_payload(dir.path(), "leaf.rs", "fn leaf() {}");
        // The same edit is denied with a real budget (see the test above), so
        // this proves the deadline — not the policy — is what let it through.
        match guard(&payload, dir.path(), Some("t")) {
            Outcome::Notify(msg) => {
                assert!(
                    msg.contains("deadline") || msg.contains("failed open"),
                    "a blown deadline must name itself, got: {msg}"
                );
            }
            Outcome::Allow => panic!(
                "a blown deadline allowed the edit SILENTLY — indistinguishable from a \
                 clean pass, which is the whole defect (aegis-nz2x)"
            ),
            other => panic!("a blown deadline must never block: {other:?}"),
        }
    }

    #[test]
    fn nothing_measurable_stays_QUIET() {
        // The other half of the split, and it is what keeps the warning meaningful.
        // The graph is Rust-only today (aegis-81t2), so every .py/.ts edit reaches
        // this branch. If it warned, the first day would bury the real signal — the
        // one above — under noise from files that were never measurable to begin with.
        let dir = wide_repo();
        std::fs::write(dir.path().join("notes.md"), "# hi\n").unwrap();
        write_policy(
            dir.path(),
            "[hank.policy]\nmode = \"enforce\"\ndeadline_ms = 30000\n\
             [hank.policy.scopes.t]\nmax_impacted_files = 1\n",
        );
        let payload = edit_payload(dir.path(), "notes.md", "# hi");
        assert_eq!(
            guard(&payload, dir.path(), Some("t")),
            Outcome::Allow,
            "an unmeasurable file is not an enforcement gap and must not warn"
        );
    }

    #[test]
    fn a_path_check_still_applies_under_a_zero_deadline() {
        let dir = wide_repo();
        write_policy(
            dir.path(),
            "[hank.policy]\nmode = \"enforce\"\ndeadline_ms = 0\n\
             [hank.policy.scopes.t]\nallow_paths = [\"caller*.rs\"]\n",
        );
        let payload = edit_payload(dir.path(), "leaf.rs", "fn leaf() {}");
        assert!(matches!(
            guard(&payload, dir.path(), Some("t")),
            Outcome::Deny(_)
        ));
    }

    #[test]
    fn garbage_and_unknown_payloads_allow() {
        let dir = wide_repo();
        assert_eq!(guard("not json", dir.path(), Some("t")), Outcome::Allow);
        let no_file = serde_json::json!({ "tool_input": {} }).to_string();
        assert_eq!(guard(&no_file, dir.path(), Some("t")), Outcome::Allow);
    }

    #[test]
    fn a_malformed_glob_fails_open_loudly() {
        let dir = wide_repo();
        write_policy(
            dir.path(),
            "[hank.policy]\nmode = \"enforce\"\n\
             [hank.policy.scopes.t]\nallow_paths = [\"src/[\"]\n",
        );
        let session = unique_session();
        let payload = serde_json::json!({
            "session_id": session,
            "cwd": dir.path().to_str().unwrap(),
            "tool_name": "Edit",
            "tool_input": {
                "file_path": dir.path().join("leaf.rs").to_str().unwrap(),
                "old_string": "fn leaf() {}",
            },
        })
        .to_string();

        let Outcome::Notify(message) = guard(&payload, dir.path(), Some("t")) else {
            panic!("expected a fail-open notice, not a block");
        };
        assert!(message.contains("UNGUARDED"));
        assert!(message.contains("malformed path globs"));

        // Second edit in the same session: still allowed, but no longer noisy.
        assert_eq!(guard(&payload, dir.path(), Some("t")), Outcome::Allow);
        let _ = std::fs::remove_file(
            std::env::temp_dir().join(format!("hank-guard-failopen-{session}")),
        );
    }

    #[test]
    fn a_non_rust_edit_is_not_sized_against_a_rust_graph() {
        let dir = wide_repo();
        std::fs::write(dir.path().join("notes.md"), "# hi\n").unwrap();
        write_policy(
            dir.path(),
            "[hank.policy]\nmode = \"enforce\"\ndeadline_ms = 30000\n\
             [hank.policy.scopes.t]\nmax_impacted_files = 0\n",
        );
        let payload = edit_payload(dir.path(), "notes.md", "# hi");
        // A zero ceiling would deny anything measurable; declining to measure a
        // language the graph cannot see is the honest answer (FR-3).
        assert_eq!(guard(&payload, dir.path(), Some("t")), Outcome::Allow);
    }
}
