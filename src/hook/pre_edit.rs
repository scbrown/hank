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

use super::measure::{measure_within, relative, Sizing};
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
pub fn run_pre_edit(tenant: Option<&str>, config_override: Option<&Path>) -> anyhow::Result<()> {
    let mut buf = String::new();
    std::io::stdin().lock().read_to_string(&mut buf).ok();
    let root = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));

    match guard(&buf, &root, tenant, config_override) {
        Outcome::Allow => {}
        Outcome::Deny(reason) => println!("{}", deny_envelope(&reason)),
        Outcome::Notify(message) => println!("{}", system_message(&message)),
    }
    Ok(())
}

/// Decide an edit. Pure apart from reading the repo, so it is directly testable.
#[must_use]
pub fn guard(
    input_json: &str,
    default_root: &Path,
    tenant: Option<&str>,
    config_override: Option<&Path>,
) -> Outcome {
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

    // Honour `--config` if the operator scoped the guard at a specific file. A
    // bad override path errors here and lands in `fail_open` — a loud allow,
    // never a silent revert to the ambient config the operator meant to bypass.
    let config = match HankConfig::resolve(config_override, &root) {
        Ok(config) => config,
        Err(e) => return fail_open(&input, &format!("unreadable config ({e})")),
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
        Sizing::Measured(radius) => radius,
        // NOT MEASURED. Allowing is still the contract — the guard is fail-open by
        // design and a language we cannot parse must not brick an agent's edits.
        // But allowing SILENTLY is the defect: an unparseable file and a
        // genuinely-clean edit produced identical empty stdout, so a rule that
        // could not be evaluated looked exactly like a rule that passed. Say it
        // instead. Rate-limited to once per session by the same gate the
        // fail-open notice uses, because a per-edit message would be scrolled
        // past and then ignored.
        unmeasured => {
            let reason = unmeasured
                .unmeasured_reason()
                .unwrap_or_else(|| "unmeasured".to_string());
            eprintln!("hank: blast radius UNMEASURED for `{rel}`: {reason}");
            if first_notice_for_session(input.session_id.as_deref()) {
                return Outcome::Notify(format!(
                    "hank: blast-radius rules were NOT EVALUATED for `{rel}` — \
                     {reason}. The edit is allowed (the guard fails open), but \
                     tenant `{tenant}`'s ceilings did not apply to it. Treat this \
                     file as UNGUARDED by blast radius, not as within limits."
                ));
            }
            return Outcome::Allow;
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
fn fail_open(input: &HookInput, reason: &str) -> Outcome {
    eprintln!("hank: policy guard failed open: {reason}");
    if first_notice_for_session(input.session_id.as_deref()) {
        return Outcome::Notify(format!(
            "hank: policy guard failed open ({reason}) — edits are UNGUARDED this session."
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
        assert_eq!(guard(&payload, dir.path(), Some("t"), None), Outcome::Allow);
    }

    #[test]
    fn allows_when_mode_is_off_despite_a_scope() {
        let dir = wide_repo();
        write_policy(
            dir.path(),
            "[hank.policy]\nmode = \"off\"\n[hank.policy.scopes.t]\nmax_impacted_symbols = 0\n",
        );
        let payload = edit_payload(dir.path(), "leaf.rs", "fn leaf() {}");
        assert_eq!(guard(&payload, dir.path(), Some("t"), None), Outcome::Allow);
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
        let Outcome::Deny(reason) = guard(&payload, dir.path(), Some("t"), None) else {
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
        let Outcome::Deny(reason) = guard(&payload, dir.path(), Some("t"), None) else {
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
        assert_eq!(guard(&payload, dir.path(), Some("t"), None), Outcome::Allow);
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
        let Outcome::Notify(message) = guard(&payload, dir.path(), Some("t"), None) else {
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
        assert_eq!(guard(&payload, dir.path(), Some("t"), None), Outcome::Allow);
        // ...and so is an agent with no tenant at all.
        assert_eq!(guard(&payload, dir.path(), None, None), Outcome::Allow);
    }

    #[test]
    fn a_blown_deadline_allows_the_edit_and_says_so() {
        let dir = wide_repo();
        write_policy(
            dir.path(),
            "[hank.policy]\nmode = \"enforce\"\ndeadline_ms = 0\n\
             [hank.policy.scopes.t]\nmax_impacted_files = 1\n",
        );
        let payload = edit_payload(dir.path(), "leaf.rs", "fn leaf() {}");
        // The same edit is denied with a real budget (see the test above), so
        // this proves the deadline — not the policy — is what let it through.
        // CONTRACT CHANGE: it still ALLOWS (fail-open is deliberate), but it no
        // longer allows in SILENCE. This test previously asserted Outcome::Allow,
        // which is the same value a clean edit produces — so the suite could not
        // tell "we did not look" from "we looked and it was fine", and neither
        // could an operator.
        match guard(&payload, dir.path(), Some("t"), None) {
            Outcome::Notify(message) => {
                assert!(message.contains("NOT EVALUATED"), "{message}");
                assert!(message.contains("deadline_ms"), "{message}");
            }
            other => panic!("a blown deadline must be reported, got {other:?}"),
        }
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
            guard(&payload, dir.path(), Some("t"), None),
            Outcome::Deny(_)
        ));
    }

    #[test]
    fn garbage_and_unknown_payloads_allow() {
        let dir = wide_repo();
        assert_eq!(
            guard("not json", dir.path(), Some("t"), None),
            Outcome::Allow
        );
        let no_file = serde_json::json!({ "tool_input": {} }).to_string();
        assert_eq!(guard(&no_file, dir.path(), Some("t"), None), Outcome::Allow);
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

        let Outcome::Notify(message) = guard(&payload, dir.path(), Some("t"), None) else {
            panic!("expected a fail-open notice, not a block");
        };
        assert!(message.contains("UNGUARDED"));
        assert!(message.contains("malformed path globs"));

        // Second edit in the same session: still allowed, but no longer noisy.
        assert_eq!(guard(&payload, dir.path(), Some("t"), None), Outcome::Allow);
        let _ = std::fs::remove_file(
            std::env::temp_dir().join(format!("hank-guard-failopen-{session}")),
        );
    }

    #[test]
    fn an_unparseable_language_is_reported_unmeasured_not_silently_allowed() {
        let dir = wide_repo();
        std::fs::write(dir.path().join("notes.md"), "# hi\n").unwrap();
        write_policy(
            dir.path(),
            "[hank.policy]\nmode = \"enforce\"\ndeadline_ms = 30000\n\
             [hank.policy.scopes.t]\nmax_impacted_files = 0\n",
        );
        let payload = edit_payload(dir.path(), "notes.md", "# hi");
        // A zero ceiling denies anything measurable, so an Allow here means the
        // rule did not apply. Declining to measure is still right — but the
        // decline must be VISIBLE, or a rule that cannot be evaluated is
        // indistinguishable from one that passed. That is the whole bug: a fleet
        // was days from turning blocking on over ceilings that silently did not
        // apply to .py/.ts/.go.
        match guard(&payload, dir.path(), Some("t"), None) {
            Outcome::Notify(message) => {
                assert!(message.contains("NOT EVALUATED"), "{message}");
                assert!(message.contains("no grammar for `.md`"), "{message}");
                assert!(message.contains("UNGUARDED"), "{message}");
            }
            other => panic!("an unparseable language must be reported, got {other:?}"),
        }
    }

    /// THE regression test, at the guard level: a ceiling that denies a Rust edit
    /// must deny the identical edit in Python and TypeScript. Measured on the
    /// shipped v0.2.0 binary, both ALLOWED with empty stdout.
    #[cfg(feature = "langs-extra")]
    #[test]
    fn a_ceiling_that_denies_rust_denies_python_and_typescript_too() {
        let cases: &[(&str, &str, &str, &str)] = &[
            (
                "py",
                "def leaf():\n    return 1\n",
                "from leaf import leaf\ndef one():\n    return leaf()\n",
                "def leaf():",
            ),
            (
                "ts",
                "export function leaf(): number { return 1; }\n",
                "import { leaf } from \"./leaf\";\nexport function one() { return leaf(); }\n",
                "export function leaf(): number { return 1; }",
            ),
        ];
        for (ext, leaf_src, caller_src, anchor) in cases {
            let dir = tempfile::tempdir().unwrap();
            std::fs::write(dir.path().join(format!("leaf.{ext}")), leaf_src).unwrap();
            std::fs::write(dir.path().join(format!("one.{ext}")), caller_src).unwrap();
            write_policy(
                dir.path(),
                "[hank.policy]\nmode = \"enforce\"\ndeadline_ms = 30000\n\
                 [hank.policy.scopes.t]\nmax_impacted_files = 0\n",
            );
            let payload = edit_payload(dir.path(), &format!("leaf.{ext}"), anchor);
            match guard(&payload, dir.path(), Some("t"), None) {
                Outcome::Deny(message) => assert!(
                    message.contains("reaches"),
                    ".{ext}: denied, but not for reach: {message}"
                ),
                other => panic!(
                    ".{ext}: a zero ceiling did NOT deny an edit reaching another \
                     file — got {other:?}. The rule silently does not apply."
                ),
            }
        }
    }

    /// The load-bearing test for aegis-ll3p: a scope supplied ONLY via
    /// `--config` must actually govern the edit. The ambient config allows
    /// (no policy), so a deny here can only come from the override being read.
    #[test]
    fn a_config_override_scopes_the_guard() {
        let dir = wide_repo(); // no `.bobbin/config.toml` — ambient allows everything
        let scope_file = dir.path().join("elsewhere").join("scope.toml");
        std::fs::create_dir_all(scope_file.parent().unwrap()).unwrap();
        std::fs::write(
            &scope_file,
            "[hank.policy]\nmode = \"enforce\"\n\
             [hank.policy.scopes.t]\nallow_paths = [\"src/**\"]\n",
        )
        .unwrap();
        let payload = edit_payload(dir.path(), "leaf.rs", "fn leaf() {}");

        // Negative control: without the override, the ambient (absent) config
        // allows the edit.
        assert_eq!(guard(&payload, dir.path(), Some("t"), None), Outcome::Allow);

        // With the override, `leaf.rs` is outside `src/**` and is denied.
        let Outcome::Deny(reason) = guard(&payload, dir.path(), Some("t"), Some(&scope_file))
        else {
            panic!("the --config scope must govern the edit");
        };
        assert!(reason.contains("leaf.rs"));
        assert!(reason.contains("outside the writable capability scope"));
    }

    /// A `--config` path that does not exist must fail OPEN loudly, never
    /// silently revert to the ambient config — reverting is the disarm this
    /// override exists to prevent. Fail-open (allow) is still correct for a
    /// guard, but it must be the loud, once-per-session kind.
    #[test]
    fn a_missing_config_override_fails_open_loudly() {
        let dir = wide_repo();
        // An ambient policy that WOULD deny, to prove the fail-open is the
        // override erroring — not the ambient config quietly taking over.
        write_policy(
            dir.path(),
            "[hank.policy]\nmode = \"enforce\"\n\
             [hank.policy.scopes.t]\nallow_paths = [\"src/**\"]\n",
        );
        let missing = dir.path().join("does-not-exist.toml");
        let payload = edit_payload(dir.path(), "leaf.rs", "fn leaf() {}");

        let Outcome::Notify(message) = guard(&payload, dir.path(), Some("t"), Some(&missing))
        else {
            panic!("a bad --config must fail open loudly, not deny and not silently revert");
        };
        assert!(message.contains("UNGUARDED"));
        assert!(message.contains("does not exist"));
    }
}
