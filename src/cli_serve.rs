//! The long-running commands — `watch`, `serve` (MCP) and `daemon` — split out
//! of `cli` for size (hank #83). See `cli_analyze` for why this is a child
//! module.
//!
//! Each has a feature-stub arm that calls `self.planned`, which stays in `cli`
//! because `cli_promote` needs it too.

use super::*;

impl Cli {
    /// Watch `path` and re-extract changed files on debounced, tiered schedules
    /// (FR-17). Blocks until interrupted (Ctrl-C).
    pub(super) async fn watch(&self, path: &Path) -> anyhow::Result<()> {
        let config = self.load_config(path)?;
        let scheduler = crate::watch::TieredScheduler::from_config(&config.freshness);
        // With a tenant, the heavy tier drives that tenant's overlay via the
        // frontier recompute (FR-16/17); without one, the un-tenanted graph
        // refresh (a full rebuild) stands in. Both honor the same debounce.
        let handler: Box<dyn crate::watch::TierHandler> = match &self.tenant {
            Some(tenant) => {
                let base = crate::graph::Base::build_at(path, &config.base_ref)
                    .map_err(|e| anyhow::anyhow!("watch: no base graph to overlay: {e}"))?;
                let registry = std::sync::Arc::new(std::sync::RwLock::new(
                    crate::graph::TenantRegistry::with_tenancy(base, config.tenancy.clone()),
                ));
                Box::new(crate::watch::OverlayRefresh::new(
                    registry,
                    tenant.clone(),
                    path.to_path_buf(),
                    config.policy.max_hops,
                ))
            }
            None => Box::new(crate::watch::GraphRefresh::new(path.to_path_buf())),
        };
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
    pub(super) async fn serve(&self, http: bool) -> anyhow::Result<()> {
        #[cfg(feature = "mcp")]
        {
            let root = std::env::current_dir()?;
            let tenant = self.tenant.clone();
            let config_override = self.config.clone();
            if http {
                let config = self.load_config(&root)?;
                crate::mcp::run_http(
                    root,
                    tenant,
                    config_override,
                    config.serve.bind_address,
                    config.serve.mcp_http_port,
                )
                .await
            } else {
                crate::mcp::run_stdio(root, tenant, config_override).await
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

    /// Run the resident-graph daemon (Phase 3, stage 1): build the base graph
    /// once and serve its liveness surface. Query endpoints are stage 2.
    pub(super) async fn daemon(&self, port: Option<u16>) -> anyhow::Result<()> {
        #[cfg(feature = "mcp")]
        {
            let root = std::env::current_dir()?;
            let config = self.load_config(&root)?;
            let bind = config.serve.bind_address.clone();
            let port = port.unwrap_or(config.serve.mcp_http_port);
            crate::daemon::serve(&root, self.config.as_deref(), &bind, port).await
        }
        #[cfg(not(feature = "mcp"))]
        {
            let _ = port;
            self.planned(
                "daemon",
                3,
                "build with `--features mcp` to enable the resident HTTP surface",
            );
            Ok(())
        }
    }
}
