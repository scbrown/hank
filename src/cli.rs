//! The `hank` command-line interface.
//!
//! `analyze`, `refs`, `status`, `serve` (MCP), the Phase-2 call-graph commands
//! `callers`/`impact` and `dataflow`, and the `hook` adapter (edit-reactive
//! harness integration, §5.9/FR-30) are live. `verify` and `promote` are
//! declared with their final shape and print a phase notice until their engines
//! land (see `docs/hank-spec.md` §12).

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
    /// Verdict on a proposed edit buffer (Phase 5).
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
}

impl Cli {
    /// Run the parsed command.
    pub async fn run(self) -> anyhow::Result<()> {
        match &self.command {
            Commands::Analyze { path } => self.analyze(path),
            Commands::Refs { symbol, path } => self.refs(symbol, path),
            Commands::Status => self.status(),
            Commands::Hook { event } => match event {
                HookEvent::PostEdit => crate::hook::run_post_edit(),
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
            Commands::Verify { .. } => {
                self.planned(
                    "verify",
                    5,
                    "monitor-guided edit verification lands in Phase 5",
                );
                Ok(())
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

    /// Build the base graph for `path` and print a summary.
    fn analyze(&self, path: &Path) -> anyhow::Result<()> {
        let mut files = 0usize;
        let mut symbols = 0usize;
        for file in rust_files(path) {
            let source = std::fs::read_to_string(&file)?;
            let found = extract_symbols(&source, "rust")?;
            files += 1;
            symbols += found.len();
        }

        if self.json {
            let out =
                serde_json::json!({ "files": files, "symbols": symbols, "tier": "treesitter" });
            println!("{}", serde_json::to_string_pretty(&out)?);
        } else if !self.quiet {
            println!(
                "{} {files} file(s), {symbols} symbol(s) [tree-sitter]",
                "analyzed".green().bold()
            );
        }
        Ok(())
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

    /// Print base ref, tier availability, and config.
    fn status(&self) -> anyhow::Result<()> {
        let root = std::env::current_dir()?;
        let config = HankConfig::load(&root)?;
        let tenant = self.tenant.as_deref().unwrap_or("(single-tenant)");

        if self.json {
            let out = serde_json::json!({
                "base_ref": config.base_ref,
                "tenant": tenant,
                "tiers": tier_availability(),
                "quipu": { "enabled": config.quipu.enabled, "branch_model": config.quipu.branch_model },
            });
            println!("{}", serde_json::to_string_pretty(&out)?);
        } else {
            println!("{}", "hank status".bold());
            println!("  base ref  : {}", config.base_ref);
            println!("  tenant    : {tenant}");
            println!("  tiers     : {}", tier_availability().join(", "));
            println!(
                "  quipu     : enabled={} branch_model={}",
                config.quipu.enabled, config.quipu.branch_model
            );
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
