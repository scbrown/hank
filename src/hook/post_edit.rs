//! The `PostToolUse` advisory — Hank's answer to "what did that edit reach?".
//!
//! `hank hook post-edit` reads the harness's `PostToolUse` JSON on stdin and
//! returns an advisory: which symbols in the edited file have callers elsewhere,
//! so the agent learns the blast radius of its own change synchronously, without
//! calling a tool. **Advisory only** — the blocking companion is
//! [`super::pre_edit`].
//!
//! With `[hank.serve] use_daemon = true` this is a thin client of the resident
//! daemon (FR-31, hank #1 stage 5): the edited file's symbols are still
//! extracted fresh HERE (their content is what just changed), but their callers
//! come from the resident graph — no per-invocation `CodeGraph::build`. The
//! daemon being unusable falls back to the transient build with a stderr note;
//! like the MCP tools and unlike the pre-edit guard, fallback is silent to the
//! model — this is an advisory, not an enforcement surface, and a transient
//! answer is equally correct.

use std::collections::BTreeSet;
use std::ffi::OsStr;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;

use super::HookInput;
use crate::config::HankConfig;
use crate::daemon::client::{expected_same_root_daemon, fetch_edit};
use crate::extract::extract_symbols;
use crate::graph::{CodeGraph, Dir};

/// Budget per localhost round-trip, same rationale as the MCP thin client:
/// generous against a resident graph, small enough that a wedged daemon costs
/// one slow query before the transient fallback answers.
const DAEMON_TIMEOUT: Duration = Duration::from_millis(500);

/// How many impacted symbols to list before summarizing the rest.
const MAX_LISTED: usize = 8;

/// Run the `post-edit` hook: read the harness payload from stdin and, if the
/// edit has cross-file impact, print the `PostToolUse` advisory envelope.
/// `tenant` is the session's overlay identity (the global `--tenant` flag).
pub fn run_post_edit(tenant: Option<&str>) -> anyhow::Result<()> {
    let mut buf = String::new();
    std::io::stdin().lock().read_to_string(&mut buf).ok();
    let root = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));

    if let Some(text) = advisory_for(&buf, &root, tenant) {
        let envelope = serde_json::json!({
            "hookSpecificOutput": {
                "hookEventName": "PostToolUse",
                "additionalContext": text,
            }
        });
        println!("{envelope}");
    }
    // A hook must never fail the harness: absence of output = nothing to say.
    Ok(())
}

/// Compute the advisory text for a hook payload, or `None` when there is nothing
/// useful to say (unparseable, non-Rust, or no cross-file impact).
#[must_use]
pub fn advisory_for(input_json: &str, default_root: &Path, tenant: Option<&str>) -> Option<String> {
    let input = HookInput::parse(input_json)?;
    let file_path = input.tool_input.file_path.clone()?;
    let file = PathBuf::from(&file_path);
    if file.extension().and_then(OsStr::to_str) != Some("rs") {
        return None;
    }

    let root = input.root(default_root);
    let rel = file
        .strip_prefix(&root)
        .unwrap_or(&file)
        .display()
        .to_string();

    let source = std::fs::read_to_string(&file).ok()?;
    let symbols = extract_symbols(&source, "rust").ok()?;
    if symbols.is_empty() {
        return None;
    }
    let names: Vec<String> = symbols.into_iter().map(|s| s.name).collect();

    let (mut per_symbol, files) =
        resident_feed(&root, &rel, tenant).or_else(|| transient_callers(&root, &rel, &names))?;
    per_symbol.sort();
    per_symbol.dedup();
    if per_symbol.is_empty() {
        return None;
    }

    Some(render(&rel, &per_symbol, &files))
}

/// Per-symbol external-caller counts and their files. The shared shape both
/// sources produce, so the advisory renders identically either way.
type ExternalCallers = (Vec<(String, usize)>, BTreeSet<String>);

/// The FR-30 cycle against the RESIDENT daemon: ONE `POST /edit` records the
/// just-saved file in this tenant's overlay AND returns the advisory from the
/// fresh composed view. `None` to fall back — daemon not expected (silent), or
/// expected-but-unusable (stderr note; an advisory has no enforcement gap to
/// be loud about). On fallback the edit is not recorded anywhere, which is
/// fine: the overlay caches the tenant's edits, the file on disk is the record.
fn resident_feed(root: &Path, rel: &str, tenant: Option<&str>) -> Option<ExternalCallers> {
    let config = HankConfig::resolve(None, root).ok()?;
    let (host, port) = match expected_same_root_daemon(&config, root, DAEMON_TIMEOUT)? {
        Ok(addr) => addr,
        Err(reason) => {
            eprintln!("hank post-edit: daemon expected but unusable, transient fallback: {reason}");
            return None;
        }
    };
    let tenant = tenant.unwrap_or("single-tenant");
    match fetch_edit(&host, port, tenant, rel, DAEMON_TIMEOUT) {
        Ok(reply) => Some((
            reply
                .advised
                .into_iter()
                .map(|a| (a.symbol, a.external_callers))
                .collect(),
            reply.files.into_iter().collect(),
        )),
        Err(reason) => {
            eprintln!("hank post-edit: daemon edit feed failed, transient fallback: {reason}");
            None
        }
    }
}

/// External callers from a transient whole-root build — the pre-daemon path,
/// kept as the fallback.
fn transient_callers(root: &Path, rel: &str, names: &[String]) -> Option<ExternalCallers> {
    let graph = CodeGraph::build(root).ok()?;
    let mut per_symbol: Vec<(String, usize)> = Vec::new();
    let mut files: BTreeSet<String> = BTreeSet::new();
    for name in names {
        let external: Vec<_> = graph
            .direct(name, Dir::Callers)
            .into_iter()
            .filter(|caller| caller.file != rel)
            .collect();
        if !external.is_empty() {
            per_symbol.push((name.clone(), external.len()));
            for caller in &external {
                files.insert(caller.file.clone());
            }
        }
    }
    Some((per_symbol, files))
}

/// Format the advisory shown to the agent.
fn render(rel: &str, per_symbol: &[(String, usize)], files: &BTreeSet<String>) -> String {
    let mut out = format!(
        "Hank (tree-sitter): your edit to {rel} touches symbol(s) with callers elsewhere \
         — re-check these still compile.\n"
    );
    for (name, count) in per_symbol.iter().take(MAX_LISTED) {
        out.push_str(&format!("  {name} <- {count} caller(s)\n"));
    }
    if per_symbol.len() > MAX_LISTED {
        out.push_str(&format!(
            "  ... and {} more\n",
            per_symbol.len() - MAX_LISTED
        ));
    }
    let file_list: Vec<&str> = files.iter().map(String::as_str).collect();
    out.push_str(&format!("Impacted files: {}", file_list.join(", ")));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn advises_on_cross_file_impact() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.rs"), "fn leaf() {}\n").unwrap();
        std::fs::write(dir.path().join("b.rs"), "fn mid() { leaf(); }\n").unwrap();

        let payload = serde_json::json!({
            "tool_name": "Edit",
            "cwd": dir.path().to_str().unwrap(),
            "tool_input": { "file_path": dir.path().join("a.rs").to_str().unwrap() },
        })
        .to_string();

        let text = advisory_for(&payload, dir.path(), None).expect("expected an advisory");
        assert!(text.contains("leaf"));
        assert!(text.contains("b.rs"));
    }

    #[test]
    fn quiet_when_no_external_callers() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("a.rs"),
            "fn leaf() {}\nfn mid() { leaf(); }\n",
        )
        .unwrap();
        // leaf's only caller (mid) is in the same file → no cross-file impact.
        let payload = serde_json::json!({
            "cwd": dir.path().to_str().unwrap(),
            "tool_input": { "file_path": dir.path().join("a.rs").to_str().unwrap() },
        })
        .to_string();
        assert!(advisory_for(&payload, dir.path(), None).is_none());
    }

    #[test]
    fn quiet_on_non_rust_or_garbage() {
        assert!(advisory_for("not json", Path::new("."), None).is_none());
        let payload = serde_json::json!({ "tool_input": { "file_path": "README.md" } }).to_string();
        assert!(advisory_for(&payload, Path::new("."), None).is_none());
    }

    // Project config expecting a daemon at 127.0.0.1:port. Written as the
    // PROJECT config so it wins over any developer user config for these keys.
    fn write_daemon_config(root: &Path, port: u16) {
        let bobbin = root.join(".bobbin");
        std::fs::create_dir_all(&bobbin).unwrap();
        std::fs::write(
            bobbin.join("config.toml"),
            format!(
                "[hank.serve]\nuse_daemon = true\nbind_address = \"127.0.0.1\"\n\
                 mcp_http_port = {port}\n"
            ),
        )
        .unwrap();
    }

    fn edit_payload(root: &Path, file: &str) -> String {
        serde_json::json!({
            "tool_name": "Edit",
            "cwd": root.to_str().unwrap(),
            "tool_input": { "file_path": root.join(file).to_str().unwrap() },
        })
        .to_string()
    }

    #[test]
    fn daemon_EXPECTED_but_DOWN_falls_back_to_the_transient_advisory() {
        // Port 1 never listens. The advisory must still be produced (transient
        // fallback) — a down daemon degrades performance, never the advisory.
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.rs"), "fn leaf() {}\n").unwrap();
        std::fs::write(dir.path().join("b.rs"), "fn mid() { leaf(); }\n").unwrap();
        write_daemon_config(dir.path(), 1);

        let text = advisory_for(&edit_payload(dir.path(), "a.rs"), dir.path(), None)
            .expect("the transient fallback must still advise");
        assert!(text.contains("leaf"));
        assert!(text.contains("b.rs"));
    }

    // Serving the router needs axum (`mcp` feature); the down/fallback quadrant
    // above runs feature-free.
    #[cfg(feature = "mcp")]
    #[tokio::test(flavor = "multi_thread")]
    async fn daemon_up_and_same_root_advises_from_the_RESIDENT_view_and_feeds_the_overlay() {
        use crate::daemon::{http, ResidentEngine};
        // The tenant layer anchors to a COMMIT, so the fixture is a real repo.
        let dir = tempfile::tempdir().unwrap();
        let run = |args: &[&str]| {
            let out = std::process::Command::new("git")
                .arg("-C")
                .arg(dir.path())
                .args(args)
                .output()
                .unwrap();
            assert!(out.status.success(), "git {args:?} failed");
        };
        run(&["init", "-q", "-b", "main"]);
        run(&["config", "user.email", "t@t"]);
        run(&["config", "user.name", "t"]);
        std::fs::write(dir.path().join("a.rs"), "fn leaf() {}\n").unwrap();
        std::fs::write(dir.path().join("b.rs"), "fn mid() { leaf(); }\n").unwrap();
        run(&["add", "-A"]);
        run(&["commit", "-qm", "base"]);

        let engine = ResidentEngine::build(dir.path(), None).unwrap();
        let observer = engine.clone();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            let _ = axum::serve(listener, http::router(engine)).await;
        });
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        write_daemon_config(dir.path(), port);

        // A REAL edit to a.rs (differs from base, still defines `leaf`): the
        // daemon reads it from disk, so it must not match the baseline or the
        // FR-15 base-hit would make it a no-op.
        std::fs::write(dir.path().join("a.rs"), "fn leaf() {}\nfn added() {}\n").unwrap();
        // `late.rs` is UNCOMMITTED and untouched: the tenant view composes
        // over base@HEAD, so it must not appear — a transient build would see
        // it. Its absence below proves who answered.
        std::fs::write(dir.path().join("late.rs"), "fn late() { leaf(); }\n").unwrap();

        let payload = edit_payload(dir.path(), "a.rs");
        let root = dir.path().to_path_buf();
        let text = tokio::task::spawn_blocking(move || advisory_for(&payload, &root, Some("t1")))
            .await
            .unwrap()
            .expect("an up, same-root daemon must advise");
        assert!(text.contains("b.rs"), "{text}");
        assert!(
            !text.contains("late.rs"),
            "`late.rs` is uncommitted and untouched — its presence means a \
             transient build answered, not the tenant view: {text}"
        );

        // And the advisory FED the overlay (FR-30): the daemon now holds the
        // edit as tenant t1's touch of a.rs.
        let reg = observer.registry().expect("repo ⇒ tenant layer");
        let reg = reg.read().unwrap();
        let overlay = reg.overlay("t1").expect("the edit created t1's overlay");
        assert!(overlay.is_touched("a.rs"));
    }
}
