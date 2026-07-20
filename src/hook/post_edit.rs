//! The `PostToolUse` advisory — Hank's answer to "what did that edit reach?".
//!
//! `hank hook post-edit` reads the harness's `PostToolUse` JSON on stdin and
//! returns an advisory: which symbols in the edited file have callers elsewhere,
//! so the agent learns the blast radius of its own change synchronously, without
//! calling a tool. **Advisory only** — the blocking companion is
//! [`super::pre_edit`].
//!
//! This prototype builds the call graph transiently per invocation. Once the
//! Phase-3 resident per-tenant overlay lands, the hook becomes a thin client of
//! the `hank serve` daemon and meets the sub-100ms budget a synchronous guard
//! needs (FR-31).

use std::collections::BTreeSet;
use std::ffi::OsStr;
use std::io::Read;
use std::path::{Path, PathBuf};

use super::HookInput;
use crate::extract::extract_symbols;
use crate::graph::{CodeGraph, Dir};

/// How many impacted symbols to list before summarizing the rest.
const MAX_LISTED: usize = 8;

/// Run the `post-edit` hook: read the harness payload from stdin and, if the
/// edit has cross-file impact, print the `PostToolUse` advisory envelope.
pub fn run_post_edit() -> anyhow::Result<()> {
    let mut buf = String::new();
    std::io::stdin().lock().read_to_string(&mut buf).ok();
    let root = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));

    if let Some(text) = advisory_for(&buf, &root) {
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
pub fn advisory_for(input_json: &str, default_root: &Path) -> Option<String> {
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

    let graph = CodeGraph::build(&root).ok()?;
    let mut per_symbol: Vec<(String, usize)> = Vec::new();
    let mut files: BTreeSet<String> = BTreeSet::new();
    for symbol in &symbols {
        let external: Vec<_> = graph
            .direct(&symbol.name, Dir::Callers)
            .into_iter()
            .filter(|caller| caller.file != rel)
            .collect();
        if !external.is_empty() {
            per_symbol.push((symbol.name.clone(), external.len()));
            for caller in &external {
                files.insert(caller.file.clone());
            }
        }
    }
    per_symbol.sort();
    per_symbol.dedup();
    if per_symbol.is_empty() {
        return None;
    }

    Some(render(&rel, &per_symbol, &files))
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

        let text = advisory_for(&payload, dir.path()).expect("expected an advisory");
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
        assert!(advisory_for(&payload, dir.path()).is_none());
    }

    #[test]
    fn quiet_on_non_rust_or_garbage() {
        assert!(advisory_for("not json", Path::new(".")).is_none());
        let payload = serde_json::json!({ "tool_input": { "file_path": "README.md" } }).to_string();
        assert!(advisory_for(&payload, Path::new(".")).is_none());
    }
}
