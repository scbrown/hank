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
use crate::extract::{extract_structure, language_for_extension};
use crate::graph::{reachable_over, CodeGraph, Dir};
use crate::policy::BlastRadius;
use crate::types::Symbol;

/// What a sizing attempt produced. THREE outcomes, and the third is the point:
/// an edit can be measured, or measured-and-fine, or NOT MEASURED AT ALL — and
/// the last must never be reported as either of the first two.
///
/// It used to be `Option<BlastRadius>`, and `None` meant all of "no grammar for
/// this language", "deadline blown", "nothing to seed from" and "unreadable
/// file". The caller allowed on `None`, silently, so a rule that could not be
/// evaluated produced the same empty stdout as a rule that passed. Measured on a
/// live build: a blast-radius ceiling of ZERO denied a Rust edit reaching two
/// files and ALLOWED the identical Python and TypeScript edits, with no output
/// distinguishing them. The guard is fail-open by design and must stay that way
/// — so the fix is not to block, it is to SAY SO.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Sizing {
    /// The graph answered. This is the only variant a ceiling may be tested on.
    Measured(BlastRadius),
    /// This build has no grammar for the file's extension, so nothing about its
    /// reach is known. NOT a radius of zero.
    NoGrammar { ext: String },
    /// Parsed, but no symbol could be found to seed the walk (an empty or
    /// symbol-less file). Also not a measurement.
    NoAnchors,
    /// The wall-clock budget expired before the graph answered.
    Deadline,
    /// The file could not be read at all.
    Unreadable,
}

impl Sizing {
    /// A short, stable tag naming which not-sized case this is — used to key the
    /// once-per-session notice so different gaps in one session do not mute each
    /// other (see `first_notice_for_session`). Distinct from `unmeasured_reason`, which
    /// is prose for the operator.
    #[must_use]
    pub fn kind_tag(&self) -> &'static str {
        match self {
            Self::Measured(_) => "measured",
            Self::NoGrammar { .. } => "no-grammar",
            Self::NoAnchors => "no-anchors",
            Self::Deadline => "deadline",
            Self::Unreadable => "unreadable",
        }
    }

    /// The one-line reason this edit was not sized, or `None` when it was.
    /// Callers surface this; they must never fold it into "allowed".
    #[must_use]
    pub fn unmeasured_reason(&self) -> Option<String> {
        match self {
            Self::Measured(_) => None,
            Self::NoGrammar { ext } => Some(format!(
                "this build has no grammar for `.{ext}`, so the edit's reach is UNKNOWN"
            )),
            Self::NoAnchors => Some(
                "no symbols could be located in the edited file, so there was \
                 nothing to size the change from"
                    .to_string(),
            ),
            Self::Deadline => Some(
                "the measurement budget (policy.deadline_ms) expired before the \
                 graph answered"
                    .to_string(),
            ),
            Self::Unreadable => Some("the edited file could not be read".to_string()),
        }
    }
}

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
) -> Sizing {
    if budget.is_zero() {
        return Sizing::Deadline;
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
    // A timeout is its own answer, not a missing one.
    rx.recv_timeout(budget).unwrap_or(Sizing::Deadline)
}

/// The blast radius of editing the symbols `anchors` fall inside.
fn measure(root: &Path, file: &Path, rel: &str, anchors: &[String], max_hops: u32) -> Sizing {
    // Parse the file as the language it IS. This used to demand `.rs` and then
    // extract as "rust"; every other language fell out here as a silent allow.
    let ext = file
        .extension()
        .and_then(OsStr::to_str)
        .unwrap_or_default()
        .to_string();
    let Some(language) = language_for_extension(&ext) else {
        return Sizing::NoGrammar { ext };
    };
    let Ok(source) = std::fs::read_to_string(file) else {
        return Sizing::Unreadable;
    };
    let Ok(structure) = extract_structure(&source, language) else {
        // The extension mapped, but this build cannot actually parse it. Same
        // ignorance, same report — never a pass.
        return Sizing::NoGrammar { ext };
    };
    let touched = touched_symbols(&structure.symbols, &source, anchors);
    if touched.is_empty() {
        return Sizing::NoAnchors;
    }

    let Ok(graph) = CodeGraph::build(root) else {
        return Sizing::Unreadable;
    };
    let mut symbols: BTreeSet<String> = BTreeSet::new();
    let mut files: BTreeSet<String> = BTreeSet::new();
    for name in &touched {
        for reached in reachable_over(&graph, name, Dir::Callers, max_hops) {
            // The edited file is the change, not its blast radius.
            if reached.file == rel {
                continue;
            }
            // A same-named symbol in ANOTHER LANGUAGE is a coincidence, not a
            // caller. The walk seeds by name, so in a polyglot repo `leaf` in
            // Python seeded the Rust, Go and TypeScript `leaf`s too — measured: a
            // four-language fixture reported every edit as reaching FOUR files
            // from one caller each. An inflated radius is a FALSE DENY, which
            // blocks legitimate work exactly as confidently as the silent allow
            // this change removed.
            // (What remains, deliberately: two same-language symbols sharing a
            // name still both seed. That is the graph's name-resolution limit,
            // pre-existing and identical for Rust; it over-reports, which is the
            // safe direction, and it is not this fix's to change.)
            let reached_language = Path::new(&reached.file)
                .extension()
                .and_then(OsStr::to_str)
                .and_then(language_for_extension);
            if reached_language != Some(language) {
                continue;
            }
            symbols.insert(reached.name);
            files.insert(reached.file);
        }
    }
    Sizing::Measured(BlastRadius {
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
        );
        let Sizing::Measured(radius) = radius else {
            panic!("a Rust edit with callers must MEASURE, got {radius:?}")
        };
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
        // A blown budget is its own answer — Deadline, not "nothing found".
        // The caller must be able to tell "we did not look" from "we looked and
        // it was fine", which is the whole defect this type exists for.
        assert_eq!(
            measure_within(
                dir.path(),
                &dir.path().join("leaf.rs"),
                "leaf.rs",
                &input,
                5,
                Duration::ZERO,
            ),
            Sizing::Deadline
        );
    }

    #[test]
    fn a_file_with_no_grammar_reports_no_grammar_not_a_clean_radius() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("notes.md"), "# hi\n").unwrap();
        // Reporting a misleadingly small radius would be worse than declining —
        // and declining SILENTLY is worse than both, because it is identical to
        // passing. The reason travels with the refusal.
        let sizing = measure(dir.path(), &dir.path().join("notes.md"), "notes.md", &[], 5);
        assert_eq!(
            sizing,
            Sizing::NoGrammar {
                ext: "md".to_string()
            }
        );
        assert!(sizing.unmeasured_reason().unwrap().contains("UNKNOWN"));
    }

    /// THE regression test for the defect this type exists for: a ceiling that
    /// denies a Rust edit must deny the identical edit in every language the
    /// build can parse. Measured on the shipped v0.2.0 binary, the Python and
    /// TypeScript cases ALLOWED with empty stdout — indistinguishable from a
    /// pass.
    #[test]
    fn a_leaf_with_callers_is_measured_in_every_compiled_language() {
        #[cfg(feature = "langs-extra")]
        {
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
                (
                    "go",
                    "package main\nfunc leaf() int { return 1 }\n",
                    "package main\nfunc one() int { return leaf() }\n",
                    "func leaf() int { return 1 }",
                ),
            ];
            for (ext, leaf_src, caller_src, anchor) in cases {
                let dir = tempfile::tempdir().unwrap();
                std::fs::write(dir.path().join(format!("leaf.{ext}")), leaf_src).unwrap();
                std::fs::write(dir.path().join(format!("one.{ext}")), caller_src).unwrap();
                let sizing = measure(
                    dir.path(),
                    &dir.path().join(format!("leaf.{ext}")),
                    &format!("leaf.{ext}"),
                    &[(*anchor).to_string()],
                    5,
                );
                match sizing {
                    Sizing::Measured(radius) => assert!(
                        radius.files >= 1 && radius.symbols >= 1,
                        ".{ext}: measured a radius of {radius:?} — a caller in \
                         another file was not seen"
                    ),
                    other => panic!(
                        ".{ext}: NOT MEASURED ({other:?}) — a blast-radius rule \
                         would silently not apply to this language"
                    ),
                }
            }
        }
    }
}
