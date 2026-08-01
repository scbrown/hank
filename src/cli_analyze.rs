//! `analyze`, `refs` and `changed` — the commands that BUILD structure and
//! report it, split out of `cli` for size (hank #83).
//!
//! A child module of `cli`, not a sibling: these stay `impl Cli` methods reading
//! the global output flags (`--json`, `--quiet`) and `load_config` straight off
//! `self`, exactly as before, which a sibling could not do without widening
//! `Cli`'s private fields. Only the entry points are `pub(super)`.

use super::*;

impl Cli {
    /// Build the base graph for `path` and print a summary. With `at`, source
    /// the summary from the git tree at that ref (the FR-13 base) rather than
    /// the working tree.
    pub(super) fn analyze(&self, path: &Path, at: Option<&str>) -> anyhow::Result<()> {
        // The `languages` key becomes real here (aegis-ltjo): analysis is
        // restricted to the configured set instead of always extracting every
        // compiled grammar. Discovery roots at the analyze target.
        let languages = self.load_config(path)?.languages;
        let (files, symbols) = match at {
            Some(reference) => Self::analyze_at_ref(path, reference, &languages)?,
            None => Self::analyze_working_tree(path, &languages)?,
        };

        if self.json {
            let mut out =
                serde_json::json!({ "files": files, "symbols": symbols, "tier": "treesitter" });
            if let Some(reference) = at {
                out["at"] = serde_json::json!(reference);
            }
            println!("{}", serde_json::to_string_pretty(&out)?);
        } else if !self.quiet {
            let at_note = at.map_or_else(String::new, |r| format!(" @ {r}"));
            println!(
                "{} {files} file(s), {symbols} symbol(s) [tree-sitter]{at_note}",
                "analyzed".green().bold()
            );
        }
        Ok(())
    }

    /// Count files and symbols across the working tree under `path`, restricted
    /// to the configured `languages` (aegis-ltjo).
    fn analyze_working_tree(path: &Path, languages: &[String]) -> anyhow::Result<(usize, usize)> {
        let mut files = 0usize;
        let mut symbols = 0usize;
        for (file, language) in crate::extract::source_files_in(path, languages) {
            let source = std::fs::read_to_string(&file)?;
            files += 1;
            symbols += extract_symbols(&source, language)?.len();
        }
        Ok((files, symbols))
    }

    /// Count files and symbols in the git tree at `reference` (the FR-13 base).
    fn analyze_at_ref(
        path: &Path,
        reference: &str,
        languages: &[String],
    ) -> anyhow::Result<(usize, usize)> {
        let root = std::env::current_dir()?;
        // REFUSE rather than report an empty baseline. `analyze --at no-such-ref`
        // printed "0 file(s), 0 symbol(s)" and exited 0, which is what a ref
        // holding no parseable files looks like — so a typo in a ref name read as
        // a real, empty measurement.
        if !crate::git::is_repo(&root) {
            anyhow::bail!(
                "not a git work tree (or `git` is unavailable), so NO BASELINE was \
                 built at `{reference}` — this is not an empty baseline"
            );
        }
        if crate::git::resolve_commit(&root, reference).is_none() {
            anyhow::bail!(
                "`{reference}` does not resolve to a commit, so NO BASELINE was \
                 built — this is not an empty baseline"
            );
        }
        let prefix = path.strip_prefix(".").unwrap_or(path);
        let mut files = 0usize;
        let mut symbols = 0usize;
        for file in crate::git::list_files_at(&root, reference) {
            if !file.starts_with(prefix) {
                continue;
            }
            // Honour the configured languages instead of hardcoding Rust: a file
            // whose extension maps to no compiled grammar, or to one the config
            // excludes, is skipped (aegis-ltjo).
            let Some(language) = file
                .extension()
                .and_then(std::ffi::OsStr::to_str)
                .and_then(crate::extract::language_for_extension)
                .filter(|lang| languages.iter().any(|a| a == lang))
            else {
                continue;
            };
            let Some(source) = crate::git::read_blob_at(&root, reference, &file) else {
                continue;
            };
            files += 1;
            symbols += extract_symbols(&source, language)?.len();
        }
        Ok((files, symbols))
    }

    /// Find definitions of `symbol` by name under `path`.
    ///
    /// Reads the SAME graph `callers`/`impact` read, deliberately. This walked
    /// `rust_files` and parsed every hit as `"rust"`, so on a Python (or Go,
    /// or TypeScript) tree it scanned ZERO files and printed "no definition
    /// found" — while `hank callers` on the same symbol in the same tree
    /// answered from the multi-language graph and listed call sites (hank #76).
    /// That is the `from_sources` "parse each file as the language it IS" bug
    /// surviving in the one command whose name advertises symbol lookup, and it
    /// failed in the worst direction: a confident "this symbol does not exist"
    /// rather than an error.
    pub(super) fn refs(&self, symbol: &str, path: &Path) -> anyhow::Result<()> {
        let graph = crate::graph::CodeGraph::build(path)?;
        let hits = graph.definitions(symbol);
        let (nodes, _) = graph.stats();

        if self.json {
            let rows: Vec<_> = hits
                .iter()
                .map(|sym| {
                    serde_json::json!({
                        "file": sym.file,
                        "name": sym.name,
                        "kind": sym.kind,
                        "start_line": sym.start_line,
                        "end_line": sym.end_line,
                        // `as_str()`, not the raw serde form: `Tier`'s derive
                        // renames to snake_case ("tree_sitter") while every
                        // other served surface — MCP, the daemon wire,
                        // `not_found` — spells it "treesitter" (the documented
                        // wire/ontology form). Emitting both spellings in ONE
                        // document made a consumer's tier check position-
                        // dependent.
                        "tier": sym.tier.as_str(),
                    })
                })
                .collect();
            // The empty answer carries its tier too (FR-3) — the hole
            // `cli_cmds::not_found` closed for callers/impact/dataflow, closed
            // here. `searched` is the honest half of a zero result: 0 symbols
            // searched means NOTHING here was parseable, which is a different
            // fact from "the name is absent from a graph that has 4000 symbols"
            // and must not be reported as the same one.
            let out = serde_json::json!({
                "symbol": symbol,
                "count": rows.len(),
                "definitions": rows,
                "searched_symbols": nodes,
                "tier": "treesitter",
            });
            println!("{}", serde_json::to_string_pretty(&out)?);
        } else if hits.is_empty() {
            if !self.quiet {
                if nodes == 0 {
                    println!(
                        "no definition found for {symbol} \
                         (nothing parseable under {} — the graph is empty, \
                         so this is not evidence the symbol is absent)",
                        path.display()
                    );
                } else {
                    println!("no definition found for {symbol} (searched {nodes} symbol(s))");
                }
            }
        } else {
            for sym in &hits {
                println!(
                    "{}:{} {} ({}) [{:?}]",
                    sym.file,
                    sym.start_line,
                    sym.name.cyan(),
                    sym.kind,
                    sym.tier
                );
            }
        }
        Ok(())
    }

    /// Print the entities a change touches — and, separately, the files it
    /// could NOT read.
    ///
    /// The two lists are printed apart on purpose. A rule enforced on the first
    /// list while the second is non-empty has judged a SUBSET of the change and
    /// will report a clean result for it; the operator has to be able to see
    /// that from the output, not infer it. Exit 2 when anything was unread, for
    /// the same reason: a caller that only checks the exit code still learns
    /// that the answer was partial.
    pub(super) fn changed(&self, base: Option<&str>, to: Option<&str>) -> anyhow::Result<()> {
        let root = std::env::current_dir()?;
        let config = HankConfig::load(&root)?;
        let base = base.unwrap_or(&config.base_ref);

        let set = match crate::change::changed_entities(&root, base, to) {
            Ok(set) => set,
            Err(e) => {
                // NOT an empty change. Say which, and fail — a caller that read
                // "0 entities" here would treat an unevaluated change as a clean
                // one, which is the premise this command exists to protect.
                if self.json {
                    println!(
                        "{}",
                        serde_json::json!({ "error": e.to_string(), "evaluated": false })
                    );
                } else {
                    eprintln!("hank: {e}");
                }
                std::process::exit(2);
            }
        };

        if self.json {
            println!("{}", serde_json::to_string_pretty(&set)?);
        } else {
            println!("{}", "hank changed".bold());
            println!("  base : {}", set.base);
            println!("  to   : {}", set.to);
            if set.entities.is_empty() {
                println!("  entities: none — this change touches no known entities");
            } else {
                println!("  entities: {}", set.entities.len());
                for e in &set.entities {
                    println!("    {:<9} {} :: {}", e.kind, e.file, e.name);
                }
            }
            if let Some(summary) = set.unread_summary() {
                println!();
                println!("  ⚠ {summary}");
                for u in &set.unread {
                    println!("    {} — {}", u.file, u.why);
                }
                println!("    A rule judged on the entities above has NOT been applied to these.");
            }
        }
        if !set.fully_read() {
            std::process::exit(2);
        }
        Ok(())
    }
}
