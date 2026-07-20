//! The `hank` command-line interface.
//!
//! `analyze`, `refs`, `status`, `serve` (MCP), the Phase-2 call-graph commands
//! `callers`/`impact` and `dataflow`, `export` (referential structure as Turtle,
//! §5.10/FR-34), the `watch` file-watcher (debounced, tiered re-extraction,
//! §5.5/FR-17), and the `hook` adapter (edit-reactive harness integration,
//! §5.9/FR-30) are live. `verify` and `promote` are declared with their final
//! shape and print a phase notice until their engines land (`docs/hank-spec.md`).

use std::io;
use std::path::{Path, PathBuf};

use clap::{CommandFactory, Parser, Subcommand, ValueEnum};
use colored::Colorize;
use tracing_subscriber::EnvFilter;

use crate::cli_cmds;
use crate::config::HankConfig;
use crate::extract::{extract_symbols, rust_files};
use crate::types::Symbol;

/// Hank — live, per-tenant code structure for the Bobbin × Quipu stack.
#[derive(Debug, Parser)]
#[command(name = "hank", version, about, long_about = None)]
pub struct Cli {
    /// The subcommand to run.
    #[command(subcommand)]
    command: Commands,

    /// Emit machine-readable JSON.
    #[arg(long, global = true)]
    json: bool,

    /// Suppress non-essential output.
    #[arg(long, global = true)]
    quiet: bool,

    /// Show detailed progress.
    #[arg(long, global = true)]
    verbose: bool,

    /// Tenant/session id (defaults to single-tenant).
    #[arg(long, global = true, env = "BOBBIN_ROLE")]
    tenant: Option<String>,

    /// Path to a config file (overrides discovery).
    #[arg(long, global = true)]
    config: Option<PathBuf>,
}

/// The available subcommands.
#[derive(Debug, Subcommand)]
enum Commands {
    /// Run the MCP server (stdio by default; `--http` for streamable-HTTP).
    Serve {
        /// Serve over streamable-HTTP instead of stdio.
        #[arg(long)]
        http: bool,
    },
    /// Build the base graph for a path and print a summary.
    Analyze {
        /// Directory or file to analyze (defaults to the current directory).
        #[arg(default_value = ".")]
        path: PathBuf,
        /// Analyze the tree at a git ref (a baseline commit) instead of the
        /// working tree — the FR-13 base. Repo-relative; degrades to empty
        /// outside a repo or for an unresolved ref.
        #[arg(long)]
        at: Option<String>,
    },
    /// What does a CHANGE do — which entities does it add, remove or modify?
    ///
    /// The FR-13 baseline pointed at a change-time question instead of a tree
    /// one. `--base` is the ref to diff FROM (defaults to the configured
    /// `base_ref`); omit `--to` to judge the WORKING TREE, which is the shape a
    /// proposed, uncommitted change has.
    Changed {
        /// Ref to diff from (defaults to the configured `base_ref`).
        #[arg(long)]
        base: Option<String>,
        /// Ref to diff to. Omit to diff against the working tree.
        #[arg(long)]
        to: Option<String>,
    },
    /// Find the definition sites of a symbol by name.
    Refs {
        /// Symbol name to locate.
        symbol: String,
        /// Directory to search (defaults to the current directory).
        #[arg(default_value = ".")]
        path: PathBuf,
    },
    /// Direct callers and callees of a symbol.
    Callers {
        /// Symbol name.
        symbol: String,
        /// Directory to build the call graph over (defaults to current dir).
        #[arg(default_value = ".")]
        path: PathBuf,
    },
    /// Blast radius: symbols transitively affected by changing a symbol.
    Impact {
        /// Seed symbol.
        symbol: String,
        /// Directory to build the call graph over (defaults to current dir).
        #[arg(default_value = ".")]
        path: PathBuf,
        /// Maximum hops to follow.
        #[arg(long, default_value_t = 5)]
        hops: u32,
        /// Reconcile against a co-change file set (FR-11): a JSON array of
        /// paths, or a newline-separated list. Supplied by Bobbin.
        #[arg(long)]
        cochange: Option<PathBuf>,
    },
    /// Detected communities: densely-connected clusters of symbols (FR-9).
    Communities {
        /// Directory to build the call graph over (defaults to current dir).
        #[arg(default_value = ".")]
        path: PathBuf,
    },
    /// Export the referential structure (modules, symbols, edges) as Turtle.
    Export {
        /// Directory to export (defaults to current dir).
        #[arg(default_value = ".")]
        path: PathBuf,
        /// Repository name to attribute entities to (defaults to the dir name).
        #[arg(long)]
        repo: Option<String>,
        /// Output format.
        #[arg(long, default_value = "turtle")]
        format: ExportFormat,
    },
    /// Intra-procedural data dependence within a function.
    Dataflow {
        /// Function to analyze.
        function: String,
        /// Directory to build the dataflow over (defaults to current dir).
        #[arg(default_value = ".")]
        path: PathBuf,
        /// Trace flow for a specific variable (omit to list all edges).
        #[arg(long)]
        var: Option<String>,
        /// Trace what the variable flows into, rather than what it depends on.
        #[arg(long)]
        forward: bool,
        /// Maximum hops to follow.
        #[arg(long, default_value_t = 5)]
        hops: u32,
    },
    /// Verdict on a proposed edit buffer (FR-23/FR-24).
    Verify {
        /// The file being edited.
        #[arg(long)]
        file: PathBuf,
        /// The edited buffer to check.
        #[arg(long)]
        buffer: PathBuf,
    },
    /// Promote a commit's structural facts into Quipu (Phase 4).
    Promote {
        /// Commit-ish to promote.
        #[arg(long, default_value = "HEAD")]
        commit: String,
    },
    /// Watch a tree and re-extract changed files, debounced and tiered (FR-17).
    Watch {
        /// Directory to watch (defaults to the current directory).
        #[arg(default_value = ".")]
        path: PathBuf,
    },
    /// Show base commit, tiers, and configuration.
    Status,
    /// Agent-harness hook adapter (reads the hook payload on stdin).
    Hook {
        /// Which hook event to handle.
        event: HookEvent,
    },
    /// Generate shell completions.
    Completions {
        /// Target shell.
        shell: clap_complete::Shell,
    },
}

/// Supported agent-harness hook events.
#[derive(Debug, Clone, Copy, ValueEnum)]
enum HookEvent {
    /// Claude Code `PostToolUse` on Edit/Write: advise on cross-file blast radius.
    PostEdit,
    /// Claude Code `PreToolUse` on Edit/Write: deny an edit that exceeds the
    /// tenant's capability scope. Opt-in, and always fails open.
    PreEdit,
}

/// Output formats for `hank export`.
#[derive(Debug, Clone, Copy, ValueEnum)]
enum ExportFormat {
    /// RDF Turtle in the `bobbin:` code ontology.
    Turtle,
}

impl Cli {
    /// Run the parsed command.
    pub async fn run(self) -> anyhow::Result<()> {
        match &self.command {
            Commands::Analyze { path, at } => self.analyze(path, at.as_deref()),
            Commands::Refs { symbol, path } => self.refs(symbol, path),
            Commands::Watch { path } => self.watch(path).await,
            Commands::Status => self.status(),
            Commands::Hook { event } => match event {
                HookEvent::PostEdit => crate::hook::run_post_edit(),
                HookEvent::PreEdit => crate::hook::run_pre_edit(self.tenant.as_deref()),
            },
            Commands::Completions { shell } => {
                let mut cmd = Cli::command();
                clap_complete::generate(*shell, &mut cmd, "hank", &mut io::stdout());
                Ok(())
            }
            Commands::Serve { http } => self.serve(*http).await,
            Commands::Callers { symbol, path } => {
                cli_cmds::callers(self.json, self.quiet, symbol, path)
            }
            Commands::Communities { path } => cli_cmds::communities(self.json, self.quiet, path),
            Commands::Impact {
                symbol,
                path,
                hops,
                cochange,
            } => cli_cmds::impact(
                self.json,
                self.quiet,
                symbol,
                path,
                *hops,
                cochange.as_deref(),
            ),
            Commands::Export { path, repo, format } => {
                let ExportFormat::Turtle = format;
                cli_cmds::export(path, repo.as_deref())
            }
            Commands::Dataflow {
                function,
                path,
                var,
                forward,
                hops,
            } => cli_cmds::dataflow(
                self.json,
                self.quiet,
                function,
                path,
                var.as_deref(),
                *forward,
                *hops,
            ),
            Commands::Changed { base, to } => self.changed(base.as_deref(), to.as_deref()),
            Commands::Verify { file, buffer } => {
                cli_cmds::verify(self.json, self.quiet, file, buffer)
            }
            Commands::Promote { .. } => {
                self.planned(
                    "promote",
                    4,
                    "Quipu promotion lands in Phase 4 (`--features quipu`)",
                );
                Ok(())
            }
        }
    }

    /// Build the base graph for `path` and print a summary. With `at`, source
    /// the summary from the git tree at that ref (the FR-13 base) rather than
    /// the working tree.
    fn analyze(&self, path: &Path, at: Option<&str>) -> anyhow::Result<()> {
        let (files, symbols) = match at {
            Some(reference) => Self::analyze_at_ref(path, reference)?,
            None => Self::analyze_working_tree(path)?,
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

    /// Count files and symbols across the working tree under `path`.
    fn analyze_working_tree(path: &Path) -> anyhow::Result<(usize, usize)> {
        let mut files = 0usize;
        let mut symbols = 0usize;
        for file in rust_files(path) {
            let source = std::fs::read_to_string(&file)?;
            files += 1;
            symbols += extract_symbols(&source, "rust")?.len();
        }
        Ok((files, symbols))
    }

    /// Count files and symbols in the git tree at `reference` (the FR-13 base).
    fn analyze_at_ref(path: &Path, reference: &str) -> anyhow::Result<(usize, usize)> {
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
            if file.extension().is_none_or(|e| e != "rs") || !file.starts_with(prefix) {
                continue;
            }
            let Some(source) = crate::git::read_blob_at(&root, reference, &file) else {
                continue;
            };
            files += 1;
            symbols += extract_symbols(&source, "rust")?.len();
        }
        Ok((files, symbols))
    }

    /// Find definitions of `symbol` by name under `path`.
    fn refs(&self, symbol: &str, path: &Path) -> anyhow::Result<()> {
        let mut hits: Vec<(PathBuf, Symbol)> = Vec::new();
        for file in rust_files(path) {
            let source = std::fs::read_to_string(&file)?;
            for found in extract_symbols(&source, "rust")? {
                if found.name == symbol {
                    hits.push((file.clone(), found));
                }
            }
        }

        if self.json {
            let rows: Vec<_> = hits
                .iter()
                .map(|(file, sym)| {
                    serde_json::json!({
                        "file": file.display().to_string(),
                        "name": sym.name,
                        "kind": sym.kind,
                        "start_line": sym.start_line,
                        "end_line": sym.end_line,
                        "tier": sym.tier,
                    })
                })
                .collect();
            println!("{}", serde_json::to_string_pretty(&rows)?);
        } else if hits.is_empty() {
            if !self.quiet {
                println!("no definition found for {symbol}");
            }
        } else {
            for (file, sym) in &hits {
                println!(
                    "{}:{} {} ({:?}) [{:?}]",
                    file.display(),
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
    fn changed(&self, base: Option<&str>, to: Option<&str>) -> anyhow::Result<()> {
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

    /// Print base ref, tier availability, and config.
    fn status(&self) -> anyhow::Result<()> {
        let root = std::env::current_dir()?;
        let config = HankConfig::load(&root)?;
        let tenant = self.tenant.as_deref().unwrap_or("(single-tenant)");
        // Resolve the configured base ref to a concrete commit (None outside a
        // repo / unresolved ref — degrade, never fail).
        let base_commit = crate::git::resolve_commit(&root, &config.base_ref);

        if self.json {
            let out = serde_json::json!({
                "base_ref": config.base_ref,
                "base_commit": base_commit,
                "tenant": tenant,
                "tiers": tier_availability(),
                "quipu": { "enabled": config.quipu.enabled, "branch_model": config.quipu.branch_model },
            });
            println!("{}", serde_json::to_string_pretty(&out)?);
        } else {
            let commit = base_commit.as_deref().map_or_else(
                || "(unresolved — not a git repo or ref absent)".to_string(),
                |c| c[..c.len().min(12)].to_string(),
            );
            println!("{}", "hank status".bold());
            println!("  base ref    : {}", config.base_ref);
            println!("  base commit : {commit}");
            println!("  tenant      : {tenant}");
            println!("  tiers       : {}", tier_availability().join(", "));
            println!(
                "  quipu       : enabled={} branch_model={}",
                config.quipu.enabled, config.quipu.branch_model
            );
        }
        Ok(())
    }

    /// Watch `path` and re-extract changed files on debounced, tiered schedules
    /// (FR-17). Blocks until interrupted (Ctrl-C).
    async fn watch(&self, path: &Path) -> anyhow::Result<()> {
        let config = HankConfig::load(path)?;
        let scheduler = crate::watch::TieredScheduler::from_config(&config.freshness);
        let handler = Box::new(crate::watch::GraphRefresh::new(path.to_path_buf()));
        let _watcher = crate::watch::Watcher::start(
            path,
            scheduler,
            handler,
            std::time::Duration::from_millis(100),
        )?;
        if !self.quiet {
            println!(
                "{} {} (tree-sitter @ {}ms, heavy @ {}ms) — Ctrl-C to stop",
                "watching".green().bold(),
                path.display(),
                config.freshness.debounce_ms,
                config.freshness.heavy_debounce_ms,
            );
        }
        tokio::signal::ctrl_c().await?;
        if !self.quiet {
            println!("{}", "watch stopped".yellow());
        }
        Ok(())
    }

    /// Run the MCP server (stdio, or streamable-HTTP with `http = true`).
    async fn serve(&self, http: bool) -> anyhow::Result<()> {
        #[cfg(feature = "mcp")]
        {
            let root = std::env::current_dir()?;
            let tenant = self.tenant.clone();
            if http {
                let config = HankConfig::load(&root)?;
                crate::mcp::run_http(
                    root,
                    tenant,
                    config.serve.bind_address,
                    config.serve.mcp_http_port,
                )
                .await
            } else {
                crate::mcp::run_stdio(root, tenant).await
            }
        }
        #[cfg(not(feature = "mcp"))]
        {
            let _ = http;
            self.planned(
                "serve",
                1,
                "build with `--features mcp` to enable the MCP + HTTP surface",
            );
            Ok(())
        }
    }

    /// Print a notice for a command whose engine has not yet landed.
    fn planned(&self, name: &str, phase: u8, detail: &str) {
        if !self.quiet {
            eprintln!(
                "{} `{name}` is planned for Phase {phase}: {detail}. See docs/hank-spec.md.",
                "note:".yellow().bold()
            );
        }
    }
}

/// The extraction tiers this build can serve.
fn tier_availability() -> Vec<String> {
    let mut tiers = vec!["treesitter".to_string()];
    if cfg!(feature = "lsp") {
        tiers.push("lsp".to_string());
    }
    if cfg!(feature = "cpg") {
        tiers.push("cpg".to_string());
    }
    tiers
}

/// Initialize the tracing subscriber (logs to stderr, `RUST_LOG`-controlled).
pub fn init_tracing() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let _ = tracing_subscriber::fmt()
        .with_writer(io::stderr)
        .with_env_filter(filter)
        .try_init();
}
