//! The `hank` command-line interface.
//!
//! This is a Phase-1 surface. `analyze`, `refs`, and `status` do real
//! tree-sitter work; `serve`, `callers`, `impact`, `verify`, and `promote` are
//! declared with their final shape and print a phase notice until their engines
//! land (see `docs/hank-spec.md` §12).

use std::io;
use std::path::PathBuf;

use clap::{CommandFactory, Parser, Subcommand};
use colored::Colorize;
use tracing_subscriber::EnvFilter;

use crate::config::HankConfig;
use crate::extract::extract_symbols;
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
    /// Run the MCP (stdio + HTTP) and HTTP API servers.
    Serve,
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
    /// Callers / callees of a symbol (Phase 2).
    Callers {
        /// Symbol name.
        symbol: String,
    },
    /// Blast radius for a change (Phase 2).
    Impact {
        /// Seed symbol.
        symbol: String,
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
    /// Generate shell completions.
    Completions {
        /// Target shell.
        shell: clap_complete::Shell,
    },
}

impl Cli {
    /// Run the parsed command.
    pub async fn run(self) -> anyhow::Result<()> {
        match &self.command {
            Commands::Analyze { path } => self.analyze(path),
            Commands::Refs { symbol, path } => self.refs(symbol, path),
            Commands::Status => self.status(),
            Commands::Completions { shell } => {
                let mut cmd = Cli::command();
                clap_complete::generate(*shell, &mut cmd, "hank", &mut io::stdout());
                Ok(())
            }
            Commands::Serve => {
                self.planned(
                    "serve",
                    1,
                    "build with `--features mcp` to enable the MCP + HTTP surface",
                );
                Ok(())
            }
            Commands::Callers { .. } => {
                self.planned("callers", 2, "call-graph extraction lands in Phase 2");
                Ok(())
            }
            Commands::Impact { .. } => {
                self.planned(
                    "impact",
                    2,
                    "blast radius lands in Phase 2 (the incremental-update primitive)",
                );
                Ok(())
            }
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
    fn analyze(&self, path: &PathBuf) -> anyhow::Result<()> {
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
    fn refs(&self, symbol: &str, path: &PathBuf) -> anyhow::Result<()> {
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

/// Walk `path` for Rust source files, honoring `.gitignore`.
fn rust_files(path: &PathBuf) -> Vec<PathBuf> {
    ignore::WalkBuilder::new(path)
        .build()
        .filter_map(std::result::Result::ok)
        .map(ignore::DirEntry::into_path)
        .filter(|p| p.extension().is_some_and(|ext| ext == "rs"))
        .collect()
}

/// Initialize the tracing subscriber (logs to stderr, `RUST_LOG`-controlled).
pub fn init_tracing() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let _ = tracing_subscriber::fmt()
        .with_writer(io::stderr)
        .with_env_filter(filter)
        .try_init();
}
