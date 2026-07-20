//! Sizing an edit: how far does this change reach?
//!
//! The guard needs one number pair — symbols and files transitively affected —
//! computed inside a hard wall-clock budget (FR-31). That is this module's whole
//! job, kept separate from [`super::pre_edit`]'s decision flow so the deadline
//! machinery and the graph walk can be tested on their own.
//!
//! The measurement is the FR-12 reachability primitive pointed at a guard
//! question instead of a query one: seed with the symbols the edit lands inside,
//! walk `Dir::Callers`, and count what comes back.

use std::collections::BTreeSet;
use std::ffi::OsStr;
use std::path::Path;
use std::time::Duration;

use super::HookInput;
use crate::extract::extract_structure;
use crate::graph::{reachable_over, CodeGraph, Dir};
use crate::policy::BlastRadius;
use crate::types::Symbol;

/// Measure the edit's blast radius, abandoning the attempt if `budget` expires.
///
/// The work runs on a worker thread so the deadline is real wall-clock, not a
/// checkpoint the graph build might sail past. An abandoned worker is harmless:
/// the process exits immediately after the guard prints.
pub fn measure_within(
    root: &Path,
    file: &Path,
    rel: &str,
    input: &HookInput,
    max_hops: u32,
    budget: Duration,
) -> Option<BlastRadius> {
    if budget.is_zero() {
        return None;
    }
    let (tx, rx) = std::sync::mpsc::channel();
    let (root, file, rel) = (root.to_path_buf(), file.to_path_buf(), rel.to_string());
    let anchors: Vec<String> = input
        .replaced_texts()
        .into_iter()
        .map(str::to_string)
        .collect();

    std::thread::spawn(move || {
        let _ = tx.send(measure(&root, &file, &rel, &anchors, max_hops));
    });
    rx.recv_timeout(budget).ok().flatten()
}

/// The blast radius of editing the symbols `anchors` fall inside.
fn measure(
    root: &Path,
    file: &Path,
    rel: &str,
    anchors: &[String],
    max_hops: u32,
) -> Option<BlastRadius> {
    // The graph is Rust-only today (see `CodeGraph::build`), so sizing a
    // non-Rust edit would report a misleadingly small radius. Decline instead.
    if file.extension().and_then(OsStr::to_str) != Some("rs") {
        return None;
    }
    let source = std::fs::read_to_string(file).ok()?;
    let structure = extract_structure(&source, "rust").ok()?;
    let touched = touched_symbols(&structure.symbols, &source, anchors);
    if touched.is_empty() {
        return None;
    }

    let graph = CodeGraph::build(root).ok()?;
    let mut symbols: BTreeSet<String> = BTreeSet::new();
    let mut files: BTreeSet<String> = BTreeSet::new();
    for name in &touched {
        for reached in reachable_over(&graph, name, Dir::Callers, max_hops) {
            // The edited file is the change, not its blast radius.
            if reached.file == rel {
                continue;
            }
            symbols.insert(reached.name);
            files.insert(reached.file);
        }
    }
    Some(BlastRadius {
        symbols: symbols.len(),
        files: files.len(),
    })
}

/// The names of the symbols this edit lands inside.
///
/// Each anchor (the `old_string` being replaced) is located in the current file
/// and mapped to the symbols whose line span contains it — so editing one
/// function in a large file is sized as that function, not the whole file. A
/// `Write`, or an anchor that cannot be found, falls back to every symbol in the
/// file, which is what a whole-file replacement actually touches.
fn touched_symbols(symbols: &[Symbol], source: &str, anchors: &[String]) -> Vec<String> {
    let all = || symbols.iter().map(|s| s.name.clone()).collect::<Vec<_>>();
    if anchors.is_empty() {
        return all();
    }

    let mut lines: Vec<(usize, usize)> = Vec::new();
    for anchor in anchors {
        let Some(offset) = source.find(anchor.as_str()) else {
            return all(); // Anchor not found — do not under-report the radius.
        };
        let start = source[..offset].lines().count().max(1);
        let end = start + anchor.lines().count().saturating_sub(1);
        lines.push((start, end));
    }

    let touched: Vec<String> = symbols
        .iter()
        .filter(|symbol| {
            lines
                .iter()
                .any(|(start, end)| symbol.start_line <= *end && symbol.end_line >= *start)
        })
        .map(|symbol| symbol.name.clone())
        .collect();

    if touched.is_empty() {
        all()
    } else {
        touched
    }
}

/// `file` relative to `root`, for matching against path globs.
pub fn relative(file: &Path, root: &Path) -> String {
    file.strip_prefix(root)
        .unwrap_or(file)
        .display()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_edit_is_sized_by_the_function_it_lands_in() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("lib.rs"), "fn quiet() {}\nfn loud() {}\n").unwrap();
        let source = std::fs::read_to_string(dir.path().join("lib.rs")).unwrap();
        let symbols = crate::extract::extract_symbols(&source, "rust").unwrap();

        // Editing `quiet` touches only `quiet`, even though `loud` shares the file.
        let touched = touched_symbols(&symbols, &source, &["fn quiet() {}".to_string()]);
        assert_eq!(touched, vec!["quiet".to_string()]);
        // A Write (no anchor) touches everything in the file.
        assert_eq!(touched_symbols(&symbols, &source, &[]).len(), 2);
    }

    #[test]
    fn an_unfindable_anchor_falls_back_to_the_whole_file() {
        let source = "fn a() {}\nfn b() {}\n";
        let symbols = crate::extract::extract_symbols(source, "rust").unwrap();
        // Never under-report the radius just because the anchor moved.
        let touched = touched_symbols(&symbols, source, &["fn nowhere() {}".to_string()]);
        assert_eq!(touched.len(), 2);
    }

    #[test]
    fn measures_transitive_callers_excluding_the_edited_file() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("leaf.rs"), "fn leaf() {}\n").unwrap();
        std::fs::write(dir.path().join("mid.rs"), "fn mid() { leaf(); }\n").unwrap();
        std::fs::write(dir.path().join("top.rs"), "fn top() { mid(); }\n").unwrap();

        let radius = measure(
            dir.path(),
            &dir.path().join("leaf.rs"),
            "leaf.rs",
            &["fn leaf() {}".to_string()],
            5,
        )
        .unwrap();
        // mid (1 hop) and top (2 hops), in two files. `leaf` itself is the
        // change, not its blast radius.
        assert_eq!(
            radius,
            BlastRadius {
                symbols: 2,
                files: 2
            }
        );
    }

    #[test]
    fn a_zero_budget_measures_nothing() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("leaf.rs"), "fn leaf() {}\n").unwrap();
        let input = HookInput::parse(
            &serde_json::json!({ "tool_input": { "file_path": "leaf.rs" } }).to_string(),
        )
        .unwrap();
        assert!(measure_within(
            dir.path(),
            &dir.path().join("leaf.rs"),
            "leaf.rs",
            &input,
            5,
            Duration::ZERO,
        )
        .is_none());
    }

    #[test]
    fn a_non_rust_file_is_not_measured_against_a_rust_graph() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("notes.md"), "# hi\n").unwrap();
        // Reporting a misleadingly small radius would be worse than declining.
        assert!(measure(dir.path(), &dir.path().join("notes.md"), "notes.md", &[], 5).is_none());
    }
}
