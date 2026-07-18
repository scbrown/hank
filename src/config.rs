//! Hank configuration.
//!
//! Hank shares the stack's `.bobbin/config.toml` under a `[hank]` table, with
//! the same resolution order Quipu uses: compiled defaults are overlaid by the
//! user config (`~/.config/bobbin/config.toml`) and then the project config
//! (`.bobbin/config.toml`). CLI flags win over all of them (applied by the
//! caller). See `docs/hank-spec.md` §11.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use crate::errors::{Error, Result};

/// Top-level Hank configuration (the `[hank]` table).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct HankConfig {
    /// Baseline ref the shared read-only graph is built at.
    pub base_ref: String,
    /// Run the LSP tier for precise facts where a build resolves.
    pub enable_lsp: bool,
    /// Run the CPG/dataflow tier (Phase 2).
    pub enable_cpg: bool,
    /// Languages to extract (defaults to Bobbin's grammar set).
    pub languages: Vec<String>,
    /// Freshness / debounce settings.
    pub freshness: FreshnessConfig,
    /// Multi-tenancy limits.
    pub tenancy: TenancyConfig,
    /// Serving surface (MCP/HTTP) settings.
    pub serve: ServeConfig,
    /// Quipu promotion settings (Phase 4).
    pub quipu: QuipuConfig,
}

impl Default for HankConfig {
    fn default() -> Self {
        Self {
            base_ref: "main".to_string(),
            enable_lsp: true,
            enable_cpg: false,
            languages: ["rust", "typescript", "python", "go", "java", "cpp"]
                .iter()
                .map(|s| (*s).to_string())
                .collect(),
            freshness: FreshnessConfig::default(),
            tenancy: TenancyConfig::default(),
            serve: ServeConfig::default(),
            quipu: QuipuConfig::default(),
        }
    }
}

/// Freshness / debounce settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct FreshnessConfig {
    /// Debounce for keystroke-driven tree-sitter updates, in milliseconds.
    pub debounce_ms: u64,
    /// When to compute LSP facts: `"save"` or `"on_demand"`.
    pub lsp_on: String,
}

impl Default for FreshnessConfig {
    fn default() -> Self {
        Self {
            debounce_ms: 300,
            lsp_on: "save".to_string(),
        }
    }
}

/// Multi-tenancy limits.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct TenancyConfig {
    /// Maximum concurrent per-tenant overlays over one base.
    pub max_overlays: usize,
    /// Symbols with fan-in above this get special frontier handling.
    pub high_fanin_threshold: usize,
    /// Overlay eviction policy: `"on_session_close"` or `"lru"`.
    pub overlay_eviction: String,
}

impl Default for TenancyConfig {
    fn default() -> Self {
        Self {
            max_overlays: 32,
            high_fanin_threshold: 200,
            overlay_eviction: "on_session_close".to_string(),
        }
    }
}

/// Serving surface settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ServeConfig {
    /// Bind address for the HTTP / streamable-HTTP MCP surface.
    pub bind_address: String,
    /// Port for the streamable-HTTP MCP + HTTP API (distinct from Bobbin/Quipu).
    pub mcp_http_port: u16,
    /// Write guard for the broker / promotion endpoints.
    pub read_only: bool,
}

impl Default for ServeConfig {
    fn default() -> Self {
        Self {
            bind_address: "127.0.0.1".to_string(),
            mcp_http_port: 3040,
            read_only: false,
        }
    }
}

/// Quipu promotion settings (Phase 4).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct QuipuConfig {
    /// Whether promotion into Quipu is enabled.
    pub enabled: bool,
    /// When to promote: `"commit"`, `"merge"`, or `"manual"`.
    pub promote_on: String,
    /// Branch model: `"named_graph"` (preferred, needs Quipu quads) or
    /// `"qualifier"` (fallback). See `docs/hank-spec.md` §9.4.
    pub branch_model: String,
    /// Directory holding the SHACL shapes promotion validates against.
    pub shapes_path: String,
}

impl Default for QuipuConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            promote_on: "merge".to_string(),
            branch_model: "named_graph".to_string(),
            shapes_path: "shapes/".to_string(),
        }
    }
}

impl HankConfig {
    /// Load the merged configuration for a project rooted at `root`.
    ///
    /// Starts from defaults, overlays the user config if present, then the
    /// project's `.bobbin/config.toml` `[hank]` table. Missing files are not an
    /// error; a malformed file is.
    pub fn load(root: &Path) -> Result<Self> {
        let mut config = Self::default();

        if let Some(user) = user_config_path() {
            if let Some(table) = read_hank_table(&user)? {
                config = table;
            }
        }

        let project = root.join(".bobbin").join("config.toml");
        if let Some(table) = read_hank_table(&project)? {
            config = table;
        }

        Ok(config)
    }
}

/// Path to the per-user config, if a home directory is resolvable.
fn user_config_path() -> Option<PathBuf> {
    std::env::var_os("HOME").map(|home| {
        PathBuf::from(home)
            .join(".config")
            .join("bobbin")
            .join("config.toml")
    })
}

/// Read the `[hank]` table from a config file, if the file exists.
fn read_hank_table(path: &Path) -> Result<Option<HankConfig>> {
    if !path.exists() {
        return Ok(None);
    }
    let text = std::fs::read_to_string(path)?;
    let root: toml::Value =
        toml::from_str(&text).map_err(|e| Error::Config(format!("{}: {e}", path.display())))?;
    match root.get("hank") {
        Some(section) => {
            let config = section
                .clone()
                .try_into()
                .map_err(|e| Error::Config(format!("{}: [hank]: {e}", path.display())))?;
            Ok(Some(config))
        }
        None => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_sensible() {
        let config = HankConfig::default();
        assert_eq!(config.base_ref, "main");
        assert_eq!(config.serve.mcp_http_port, 3040);
        assert_eq!(config.quipu.branch_model, "named_graph");
        assert!(config.languages.contains(&"rust".to_string()));
    }

    #[test]
    fn load_missing_project_returns_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let config = HankConfig::load(dir.path()).unwrap();
        assert_eq!(config.base_ref, "main");
    }

    #[test]
    fn load_reads_hank_table() {
        let dir = tempfile::tempdir().unwrap();
        let bobbin = dir.path().join(".bobbin");
        std::fs::create_dir_all(&bobbin).unwrap();
        std::fs::write(
            bobbin.join("config.toml"),
            "[hank]\nbase_ref = \"develop\"\n",
        )
        .unwrap();
        let config = HankConfig::load(dir.path()).unwrap();
        assert_eq!(config.base_ref, "develop");
        // Unspecified keys fall back to defaults.
        assert_eq!(config.serve.mcp_http_port, 3040);
    }
}
