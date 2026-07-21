//! Integration tests driving the `hank` binary.

use assert_cmd::Command;
use predicates::prelude::*;

/// Write a throwaway Rust file into a fresh temp dir and return the dir.
fn project_with(file: &str, contents: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join(file), contents).unwrap();
    dir
}

#[test]
fn status_json_reports_base_ref() {
    Command::cargo_bin("hank")
        .unwrap()
        .args(["status", "--json"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"base_ref\""))
        // The resolved baseline commit is reported (this repo is a git repo, so
        // it resolves to a 40-char SHA; the key is present regardless).
        .stdout(predicate::str::contains("\"base_commit\""));
}

#[test]
fn analyze_counts_symbols() {
    let dir = project_with("a.rs", "fn foo() {}\nstruct Bar;\n");
    Command::cargo_bin("hank")
        .unwrap()
        .args(["analyze", dir.path().to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("2 symbol"));
}

#[test]
fn refs_finds_definition() {
    let dir = project_with("a.rs", "fn target() {}\n");
    Command::cargo_bin("hank")
        .unwrap()
        .args(["refs", "target", dir.path().to_str().unwrap()])
        .assert()
        .success()
        // Assert on what ONLY a RESOLVED hit prints (src/cli.rs refs(): the hits
        // branch renders `<file>:<line> <name> (<kind>) [<tier>]`). The old
        // assertion was `contains("target")` — and the EMPTY branch prints
        // "no definition found for target", which also contains "target". So the
        // test passed whether or not refs resolved anything: gutting refs() to push
        // no hits left it green (aegis-fo30). `a.rs:1` (the resolved location) and
        // "(Function)" (the resolved kind) appear ONLY when a definition is found,
        // and the explicit not() pins that the not-found path is NOT what satisfied
        // the test.
        .stdout(
            predicate::str::contains("a.rs:1")
                .and(predicate::str::contains("(Function)"))
                .and(predicate::str::contains("no definition found").not()),
        );
}

#[test]
fn refs_json_contains_the_resolved_definition() {
    // The programmatic FR-4/FR-5 surface (Bobbin + agents consume --json). Mirrors
    // refs_json_is_empty_array_when_absent, but for the POSITIVE case, and asserts
    // on fields the empty result `[]` cannot carry: a resolved hit emits "kind" and
    // "start_line": 1. This is the clean discriminator the empty branch prints
    // nothing of.
    let dir = project_with("a.rs", "fn target() {}\n");
    Command::cargo_bin("hank")
        .unwrap()
        .args(["refs", "target", dir.path().to_str().unwrap(), "--json"])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("\"kind\"")
                .and(predicate::str::contains("\"start_line\": 1"))
                .and(predicate::str::contains("\"name\": \"target\"")),
        );
}

#[test]
fn refs_json_is_empty_array_when_absent() {
    let dir = project_with("a.rs", "fn other() {}\n");
    Command::cargo_bin("hank")
        .unwrap()
        .args(["refs", "missing", dir.path().to_str().unwrap(), "--json"])
        .assert()
        .success()
        .stdout(predicate::str::contains("[]"));
}

#[test]
fn callers_lists_direct_callers() {
    let dir = project_with("a.rs", "fn leaf() {}\nfn mid() { leaf(); }\n");
    Command::cargo_bin("hank")
        .unwrap()
        .args(["callers", "leaf", dir.path().to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("mid"));
}

#[test]
fn impact_reconciles_with_cochange() {
    let dir = project_with(
        "a.rs",
        "fn leaf() {}\nfn mid() { leaf(); }\nfn top() { mid(); }\n",
    );
    // Co-change set: a.rs is corroborated (also structural); other.rs is not.
    std::fs::write(dir.path().join("cochange.json"), "[\"a.rs\", \"other.rs\"]").unwrap();

    Command::cargo_bin("hank")
        .unwrap()
        .args([
            "impact",
            "leaf",
            dir.path().to_str().unwrap(),
            "--cochange",
            dir.path().join("cochange.json").to_str().unwrap(),
            "--json",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"reconciliation\""))
        .stdout(predicate::str::contains("\"corroborated\""))
        .stdout(predicate::str::contains("other.rs"));
}

#[test]
fn dataflow_traces_dependence() {
    let dir = project_with(
        "a.rs",
        "fn f(a: i32) -> i32 { let b = a + 1; let c = b * 2; c }\n",
    );
    Command::cargo_bin("hank")
        .unwrap()
        .args([
            "dataflow",
            "f",
            dir.path().to_str().unwrap(),
            "--var",
            "c",
            "--json",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"b\""))
        .stdout(predicate::str::contains("\"a\""));
}

#[test]
fn impact_reports_transitive_callers() {
    let dir = project_with(
        "a.rs",
        "fn leaf() {}\nfn mid() { leaf(); }\nfn top() { mid(); }\n",
    );
    Command::cargo_bin("hank")
        .unwrap()
        .args([
            "impact",
            "leaf",
            dir.path().to_str().unwrap(),
            "--json",
            "--hops",
            "5",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"top\""))
        .stdout(predicate::str::contains("\"count\": 2"));
}

// ── The pre-edit policy guard (§5.8/FR-25, FR-30) ────────────────────────
//
// These drive the real binary end to end, because the guard's contract is about
// process behaviour — exit code and stdout — not just its return value. The one
// rule the harness depends on: **exit 0, always**. Exit 2 is Claude Code's
// fail-*closed* channel, so a guard that ever emitted it could hard-block an
// agent.

/// A repo where `leaf` is called from three other files, with a policy applied.
fn guarded_project(policy: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("leaf.rs"), "fn leaf() {}\n").unwrap();
    for i in 0..3 {
        std::fs::write(
            dir.path().join(format!("caller{i}.rs")),
            format!("fn c{i}() {{ leaf(); }}\n"),
        )
        .unwrap();
    }
    let bobbin = dir.path().join(".bobbin");
    std::fs::create_dir_all(&bobbin).unwrap();
    std::fs::write(bobbin.join("config.toml"), policy).unwrap();
    dir
}

/// A `PreToolUse` payload editing `file` in `dir`.
fn pre_edit_payload(dir: &std::path::Path, file: &str, old: &str) -> String {
    serde_json::json!({
        "session_id": format!("it-{}-{file}", std::process::id()),
        "cwd": dir.to_str().unwrap(),
        "hook_event_name": "PreToolUse",
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
fn pre_edit_denies_an_edit_beyond_the_blast_radius() {
    let dir = guarded_project(
        "[hank.policy]\nmode = \"enforce\"\ndeadline_ms = 30000\n\
         [hank.policy.scopes.polecat]\nmax_impacted_files = 1\n",
    );
    Command::cargo_bin("hank")
        .unwrap()
        .args(["hook", "pre-edit", "--tenant", "polecat"])
        .write_stdin(pre_edit_payload(dir.path(), "leaf.rs", "fn leaf() {}"))
        .assert()
        // Deny is exit 0 + JSON; the harness never sees a failing process.
        .success()
        .stdout(predicate::str::contains("\"permissionDecision\":\"deny\""))
        .stdout(predicate::str::contains("\"hookEventName\":\"PreToolUse\""))
        // The reason must be actionable: what was exceeded, and by how much.
        .stdout(predicate::str::contains("3 files (ceiling 1)"));
}

#[test]
fn pre_edit_denies_a_path_outside_the_capability_scope() {
    let dir = guarded_project(
        "[hank.policy]\nmode = \"enforce\"\n\
         [hank.policy.scopes.polecat]\nallow_paths = [\"caller*.rs\"]\n",
    );
    Command::cargo_bin("hank")
        .unwrap()
        .args(["hook", "pre-edit", "--tenant", "polecat"])
        .write_stdin(pre_edit_payload(dir.path(), "leaf.rs", "fn leaf() {}"))
        .assert()
        .success()
        .stdout(predicate::str::contains("\"permissionDecision\":\"deny\""))
        .stdout(predicate::str::contains(
            "outside the writable capability scope",
        ));
}

#[test]
fn pre_edit_allows_an_ordinary_edit_silently() {
    let dir = guarded_project(
        "[hank.policy]\nmode = \"enforce\"\ndeadline_ms = 30000\n\
         [hank.policy.scopes.polecat]\nmax_impacted_files = 10\n",
    );
    Command::cargo_bin("hank")
        .unwrap()
        .args(["hook", "pre-edit", "--tenant", "polecat"])
        .write_stdin(pre_edit_payload(dir.path(), "leaf.rs", "fn leaf() {}"))
        .assert()
        .success()
        // Allow is *silence*. Emitting permissionDecision:"allow" would suppress
        // the user's own permission prompt — the guard only ever subtracts.
        .stdout(predicate::str::is_empty());
}

#[test]
fn pre_edit_resolves_the_tenant_from_bobbin_role() {
    let dir = guarded_project(
        "[hank.policy]\nmode = \"enforce\"\n\
         [hank.policy.scopes.polecat]\nallow_paths = [\"caller*.rs\"]\n",
    );
    // Shantytown sets BOBBIN_ROLE per agent, so one hook registration serves
    // every role; this is the path that actually runs in the field.
    Command::cargo_bin("hank")
        .unwrap()
        .args(["hook", "pre-edit"])
        .env("BOBBIN_ROLE", "polecat")
        .write_stdin(pre_edit_payload(dir.path(), "leaf.rs", "fn leaf() {}"))
        .assert()
        .success()
        .stdout(predicate::str::contains("\"permissionDecision\":\"deny\""));
}

#[test]
fn pre_edit_fails_open_on_garbage_and_on_no_policy() {
    // Unparseable payload.
    Command::cargo_bin("hank")
        .unwrap()
        .args(["hook", "pre-edit", "--tenant", "polecat"])
        .write_stdin("not json at all")
        .assert()
        .success()
        .stdout(predicate::str::is_empty());

    // A repo with no policy configured at all.
    let dir = project_with("a.rs", "fn foo() {}\n");
    Command::cargo_bin("hank")
        .unwrap()
        .args(["hook", "pre-edit", "--tenant", "polecat"])
        .write_stdin(pre_edit_payload(dir.path(), "a.rs", "fn foo() {}"))
        .assert()
        .success()
        .stdout(predicate::str::is_empty());
}

#[test]
fn pre_edit_never_denies_in_advise_mode() {
    let dir = guarded_project(
        "[hank.policy]\nmode = \"advise\"\ndeadline_ms = 30000\n\
         [hank.policy.scopes.polecat]\nmax_impacted_files = 1\n",
    );
    Command::cargo_bin("hank")
        .unwrap()
        .args(["hook", "pre-edit", "--tenant", "polecat"])
        .write_stdin(pre_edit_payload(dir.path(), "leaf.rs", "fn leaf() {}"))
        .assert()
        .success()
        .stdout(predicate::str::contains("systemMessage"))
        .stdout(predicate::str::contains("not blocking"))
        // Staging a scope must never block, however badly it is misconfigured.
        .stdout(predicate::str::contains("permissionDecision").not());
}

// ── hank verify: monitor-guided edit verification (FR-23/FR-24) ──────────

#[test]
fn verify_passes_a_clean_buffer_and_reports_its_tier() {
    let dir = project_with("helpers.rs", "fn helper() {}\n");
    let buffer = dir.path().join("proposed.rs");
    std::fs::write(&buffer, "fn f() { helper(); }\n").unwrap();

    Command::cargo_bin("hank")
        .unwrap()
        .current_dir(dir.path())
        .args(["verify", "--file", "a.rs", "--buffer"])
        .arg(&buffer)
        .arg("--json")
        .assert()
        .success()
        .stdout(predicate::str::contains("\"ok\": true"))
        // A clean verdict must never be read as "this compiles".
        .stdout(predicate::str::contains(
            "type-violation (needs the LSP tier)",
        ));
}

#[test]
fn verify_exits_nonzero_and_names_each_violation() {
    let dir = project_with("helpers.rs", "fn helper() {}\n");
    let buffer = dir.path().join("proposed.rs");
    std::fs::write(
        &buffer,
        "fn takes_two(a: u8, b: u8) {}\nfn f() { takes_two(1); ghost(); }\nmod missing;\n",
    )
    .unwrap();

    Command::cargo_bin("hank")
        .unwrap()
        .current_dir(dir.path())
        .args(["verify", "--file", "a.rs", "--buffer"])
        .arg(&buffer)
        .arg("--json")
        .assert()
        // Non-zero so CI and scripts can gate on a verdict.
        .failure()
        .stdout(predicate::str::contains("identifier-does-not-exist"))
        .stdout(predicate::str::contains("wrong-arity"))
        .stdout(predicate::str::contains("unresolved-import"));
}

/// The guard's blocking channel is a JSON object on stdout, never an exit code.
/// Exit `2` is Claude Code's fail-CLOSED channel, so *any* hook invocation that
/// exits `2` blocks the agent's edit.
///
/// The path that matters is version skew, not a typo: a `hank` older than the
/// subcommand answers `hook pre-edit` with clap's "invalid value" error and
/// exit `2`. Deploying the hook against a stale binary would therefore block
/// every Edit/Write in the fleet — the exact outcome the fail-open clause
/// exists to prevent. Absence already fails open (exit `127`); staleness is the
/// case that did not.
///
/// An unknown hook event stands in for "this hank is too old to know the event
/// you asked for", which is indistinguishable from skew at the CLI boundary.
#[test]
fn an_unknown_hook_event_fails_open_instead_of_exiting_2() {
    Command::cargo_bin("hank")
        .unwrap()
        .args(["hook", "some-event-this-hank-does-not-have"])
        .write_stdin(r#"{"tool_name":"Edit","tool_input":{"file_path":"/tmp/x.rs"}}"#)
        .assert()
        .code(0)
        // Silence on stdout: a guard that cannot parse its arguments has not
        // decided anything, and must not appear to have allowed or denied.
        .stdout(predicate::str::is_empty())
        .stderr(predicate::str::contains("failed open"));
}

/// The same protection must not swallow ordinary CLI misuse: a non-hook command
/// still exits `2`, so typos stay loud everywhere it is safe for them to be.
#[test]
fn a_non_hook_command_still_exits_2_on_bad_arguments() {
    Command::cargo_bin("hank")
        .unwrap()
        .args(["definitely-not-a-command"])
        .assert()
        .code(2);
}

/// `hank hook --help` is an "error" in clap's model; it must still print and
/// exit `0` rather than being mistaken for a fail-open.
#[test]
fn hook_help_still_prints_and_exits_0() {
    Command::cargo_bin("hank")
        .unwrap()
        .args(["hook", "--help"])
        .assert()
        .code(0)
        .stdout(predicate::str::contains("post-edit"));
}

/// aegis-ll3p acceptance #1: `--config` makes `status` read the named file, not
/// the ambient config in the cwd.
#[test]
fn config_flag_makes_status_read_the_named_file() {
    let dir = tempfile::tempdir().unwrap();
    // Ambient config in the cwd says one thing...
    let bobbin = dir.path().join(".bobbin");
    std::fs::create_dir_all(&bobbin).unwrap();
    std::fs::write(
        bobbin.join("config.toml"),
        "[hank]\nbase_ref = \"from-cwd\"\n",
    )
    .unwrap();
    // ...the override file says another.
    let other = dir.path().join("other.toml");
    std::fs::write(&other, "[hank]\nbase_ref = \"from-flag\"\n").unwrap();

    Command::cargo_bin("hank")
        .unwrap()
        .current_dir(dir.path())
        .args(["status", "--json", "--config"])
        .arg(&other)
        .assert()
        .success()
        .stdout(predicate::str::contains("from-flag"))
        .stdout(predicate::str::contains("from-cwd").not());
}

/// aegis-ll3p acceptance #2, the load-bearing one: a `deny_paths`/scope rule
/// supplied ONLY via `--config` causes the guard to DENY an edit the ambient
/// config would allow. Negative control: without `--config`, the same edit is
/// allowed. Distinguishes "the override was read" from "the guard failed open".
#[test]
fn config_flag_points_the_guard_at_a_scope_file() {
    // No `.bobbin/config.toml` here, so the ambient config allows everything.
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("leaf.rs"), "fn leaf() {}\n").unwrap();
    let scope = dir.path().join("scope.toml");
    std::fs::write(
        &scope,
        "[hank.policy]\nmode = \"enforce\"\n\
         [hank.policy.scopes.polecat]\nallow_paths = [\"src/**\"]\n",
    )
    .unwrap();
    let payload = pre_edit_payload(dir.path(), "leaf.rs", "fn leaf() {}");

    // Negative control: no --config → ambient (absent) config → allow (silent).
    Command::cargo_bin("hank")
        .unwrap()
        .args(["hook", "pre-edit", "--tenant", "polecat"])
        .write_stdin(payload.clone())
        .assert()
        .success()
        .stdout(predicate::str::is_empty());

    // With --config, `leaf.rs` is outside `src/**` and is denied.
    Command::cargo_bin("hank")
        .unwrap()
        .args(["hook", "pre-edit", "--tenant", "polecat", "--config"])
        .arg(&scope)
        .write_stdin(payload)
        .assert()
        .success()
        .stdout(predicate::str::contains("\"permissionDecision\":\"deny\""))
        .stdout(predicate::str::contains(
            "outside the writable capability scope",
        ));
}

/// A `--config` path that does not exist is a loud failure on an ordinary
/// command, not a silent fall-back to discovery.
#[test]
fn a_missing_config_path_is_a_loud_error_on_status() {
    Command::cargo_bin("hank")
        .unwrap()
        .args(["status", "--config", "/no/such/hank-config.toml"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("does not exist"));
}

/// aegis-hac0 observability: `hank status` must surface the policy layer — the
/// guard's own state was invisible in the command meant to show configuration.
#[test]
fn status_surfaces_policy_and_the_absent_signed_rule_set() {
    let dir = tempfile::tempdir().unwrap();
    let scope = dir.path().join("scope.toml");
    std::fs::write(
        &scope,
        "[hank.policy]\nmode = \"enforce\"\n\
         [hank.policy.scopes.weaver]\nallow_paths = [\"src/**\"]\nmax_impacted_files = 3\n",
    )
    .unwrap();

    Command::cargo_bin("hank")
        .unwrap()
        .args(["status", "--json", "--tenant", "weaver", "--config"])
        .arg(&scope)
        .assert()
        .success()
        // The policy layer is now observable...
        .stdout(predicate::str::contains("\"policy\""))
        .stdout(predicate::str::contains("\"mode\": \"enforce\""))
        .stdout(predicate::str::contains("\"scope_configured\": true"))
        // ...and the not-yet-existing signed rule set is reported ABSENT, loudly,
        // rather than omitted (aegis-hac0's second clause).
        .stdout(predicate::str::contains("\"signed_rule_set\""))
        .stdout(predicate::str::contains("\"never-loaded\""));
}

/// The armed-but-inert state — enforce mode with no scope for the tenant — must
/// be a visible caveat, not read as a healthy enforcing guard.
#[test]
fn status_warns_on_enforce_without_a_scope_for_the_tenant() {
    let dir = tempfile::tempdir().unwrap();
    let scope = dir.path().join("scope.toml");
    std::fs::write(
        &scope,
        "[hank.policy]\nmode = \"enforce\"\n\
         [hank.policy.scopes.someone_else]\nallow_paths = [\"src/**\"]\n",
    )
    .unwrap();

    Command::cargo_bin("hank")
        .unwrap()
        .args(["status", "--json", "--tenant", "weaver", "--config"])
        .arg(&scope)
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "\"enforcing_without_scope\": true",
        ));
}

// --- config keys made real (aegis-ltjo) -------------------------------------

/// `.bobbin/config.toml` under `dir` with the given `[hank]` body.
fn with_config(dir: &std::path::Path, body: &str) {
    let bobbin = dir.join(".bobbin");
    std::fs::create_dir_all(&bobbin).unwrap();
    std::fs::write(bobbin.join("config.toml"), body).unwrap();
}

#[cfg(feature = "langs-extra")] // needs the python grammar compiled in
#[test]
fn languages_restricts_what_analyze_counts() {
    // A mixed-language tree: 2 Rust + 2 Python + 1 TypeScript symbols.
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.rs"), "fn r(){}\nfn r2(){}\n").unwrap();
    std::fs::write(
        dir.path().join("b.py"),
        "def p():\n    pass\ndef p2():\n    pass\n",
    )
    .unwrap();
    std::fs::write(dir.path().join("c.ts"), "export function t(){}\n").unwrap();
    let p = dir.path().to_str().unwrap();

    // languages = ["rust"] -> only the 2 Rust symbols.
    with_config(dir.path(), "[hank]\nlanguages = [\"rust\"]\n");
    Command::cargo_bin("hank")
        .unwrap()
        .args(["analyze", "--json", p])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"symbols\": 2"));

    // Adding python -> 4. The key RESTRICTS; a user who narrows it gets narrowing.
    with_config(dir.path(), "[hank]\nlanguages = [\"rust\",\"python\"]\n");
    Command::cargo_bin("hank")
        .unwrap()
        .args(["analyze", "--json", p])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"symbols\": 4"));
}

#[test]
fn serve_read_only_refuses_a_write() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("x.rs"), "fn x(){}\n").unwrap();
    // git init so promote's own preconditions don't mask the guard.
    Command::new("git")
        .arg("init")
        .arg("-q")
        .current_dir(dir.path())
        .assert()
        .success();

    // read_only = true -> promotion (the write) is REFUSED with a distinguishable
    // error naming the key. This is the guard the docs claimed and did not perform.
    with_config(dir.path(), "[hank.serve]\nread_only = true\n");
    Command::cargo_bin("hank")
        .unwrap()
        .arg("promote")
        .current_dir(dir.path())
        .assert()
        .failure()
        .stderr(predicate::str::contains("read_only"))
        .stderr(predicate::str::contains("refused"));

    // read_only = false -> the write guard PASSES. What happens next depends on the
    // build, and the point of this half is only that the failure (if any) is NOT the
    // guard: whatever stops the promotion, it is never `read_only`.
    with_config(dir.path(), "[hank.serve]\nread_only = false\n");
    let assert = Command::cargo_bin("hank")
        .unwrap()
        .arg("promote")
        .current_dir(dir.path())
        .assert();
    if cfg!(feature = "quipu") {
        // With promotion wired, `promote` with no `--to` refuses for lack of an
        // endpoint — a real precondition, reached only because the guard let it
        // through. The guard is proven passed by the absence of its name here.
        assert
            .failure()
            .stderr(predicate::str::contains("--to").or(predicate::str::contains("endpoint")))
            .stderr(predicate::str::contains("read_only").not());
    } else {
        // Without the feature, promotion is a phase-4 stub that succeeds.
        assert.success();
    }
}

/// `hank export` prints Turtle; `hank export --to <url>` PROMOTES instead — one
/// promotion path, two spec spellings (§15). This pins the routing: `--to`
/// reaches the same validate-then-write path `promote` does, and plain `export`
/// still prints.
#[test]
fn export_to_routes_through_promotion_not_print() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("x.rs"), "pub fn x() -> u32 { y() }\nfn y() -> u32 { 1 }\n")
        .unwrap();

    // Plain export prints Turtle in the bobbin ontology — a read, always.
    Command::cargo_bin("hank")
        .unwrap()
        .arg("export")
        .current_dir(dir.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("bobbin:"))
        .stdout(predicate::str::contains("CodeSymbol"));

    // export --to <unreachable>: with quipu it routes into promotion, validates
    // the (valid) Turtle, then fails to REACH the endpoint — proving it took the
    // write path, not the print path (a print would have succeeded and emitted
    // Turtle). Without quipu it is the phase-4 stub.
    let assert = Command::cargo_bin("hank")
        .unwrap()
        .args(["export", "--to", "http://127.0.0.1:1"])
        .current_dir(dir.path())
        .assert();
    if cfg!(feature = "quipu") {
        assert
            .failure()
            .stdout(predicate::str::contains("bobbin:").not());
    } else {
        assert.success();
    }
}
