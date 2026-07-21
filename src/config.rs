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
    /// Capability-scoped edit policy for the pre-edit guard (§5.8/FR-25).
    pub policy: crate::policy::PolicyConfig,
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
            policy: crate::policy::PolicyConfig::default(),
        }
    }
}

/// Freshness / debounce settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct FreshnessConfig {
    /// Debounce for keystroke-driven tree-sitter updates, in milliseconds.
    pub debounce_ms: u64,
    /// Debounce for the deferred heavy tier (graph/frontier recompute, and later
    /// LSP/CPG), in milliseconds. Longer than `debounce_ms` so a burst of edits
    /// does not thrash the expensive recompute (FR-17).
    pub heavy_debounce_ms: u64,
    /// When to compute LSP facts: `"save"` or `"on_demand"`.
    pub lsp_on: String,
}

impl Default for FreshnessConfig {
    fn default() -> Self {
        Self {
            debounce_ms: 300,
            heavy_debounce_ms: 1500,
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
    /// Whether the pre-edit guard should EXPECT a resident daemon at
    /// `bind_address:mcp_http_port` and use it to size edits (FR-31).
    ///
    /// This flag is what makes "daemon not running" LOUD rather than noisy. When
    /// false (the default, and true everywhere today since no daemon runs), the
    /// guard builds the graph transiently and says nothing — absence is normal. When
    /// true, the guard asks the daemon and, if it cannot, warns ONCE per session that
    /// the resident guard is down while still guarding via a transient rebuild. Only
    /// an operator who has actually started a daemon sets this, so the warning fires
    /// exactly when a daemon was expected and isn't there — the cheapest-bypass case.
    pub use_daemon: bool,
}

impl Default for ServeConfig {
    fn default() -> Self {
        Self {
            bind_address: "127.0.0.1".to_string(),
            mcp_http_port: 3040,
            read_only: false,
            use_daemon: false,
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
    /// Base URL of the Quipu to promote into (e.g. `http://localhost:8080`).
    /// Deployment config, NOT a per-call parameter: the graph a promotion writes
    /// into is data identity, not a caller's choice. Empty by default so a
    /// misconfigured deployment refuses rather than guessing a graph. The CLI
    /// `--to` overrides it for one-off promotions.
    pub endpoint: String,
}

impl Default for QuipuConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            promote_on: "merge".to_string(),
            branch_model: "named_graph".to_string(),
            shapes_path: "shapes/".to_string(),
            endpoint: String::new(),
        }
    }
}

impl HankConfig {
    /// Refuse a mutating operation when `serve.read_only` is set.
    ///
    /// The write guard the docs promise (config.md: "Write guard for the broker /
    /// promotion endpoints"). Before this it was documented, settable, and INERT —
    /// an operator who set `read_only = true` before exposing `hank serve` to a
    /// broker got no guard and no warning, which is strictly worse than an absent
    /// switch: a safety control that does nothing invites the trust it cannot
    /// honour (aegis-ltjo). Now the one write hank performs — promotion — calls
    /// this and REFUSES with a distinguishable error naming the key, so the guard
    /// is real and any future served write is one `write_guard` call from being
    /// covered too.
    pub fn write_guard(&self, operation: &str) -> Result<()> {
        if self.serve.read_only {
            return Err(Error::Config(format!(
                "refused: `serve.read_only = true` — this hank instance is \
                 configured read-only, so {operation} (a write) is refused. \
                 Unset serve.read_only to allow writes."
            )));
        }
        Ok(())
    }

    /// Load the merged configuration for a project rooted at `root`.
    ///
    /// Starts from defaults, overlays the user config if present, then the
    /// project's `.bobbin/config.toml` `[hank]` table. Missing files are not an
    /// error; a malformed file is.
    ///
    /// "Overlay" is per-key, not per-file. Replacing the whole table would mean
    /// a project config that sets one unrelated key silently discards every
    /// other setting the user config established — and when the discarded
    /// setting is `[hank.policy]`, the capability guard goes inert while
    /// looking exactly like a clean run. A fleet keeps its scopes in one
    /// user-level file precisely so they cannot drift, so that file has to
    /// survive a workspace defining `base_ref`.
    pub fn load(root: &Path) -> Result<Self> {
        Self::load_layered(user_config_path().as_deref(), root)
    }

    /// Resolve configuration honouring an explicit `--config` override.
    ///
    /// `Some(path)` **replaces** discovery: FR-29 ranks a flag above project and
    /// user config, so the override loads exactly that file over defaults and
    /// the ambient `.bobbin/config.toml` is never consulted. `None` runs the
    /// normal [`load`](Self::load) discovery rooted at `root`.
    ///
    /// A `--config` path that cannot be read is an ERROR, never a silent
    /// fall-back to discovery. That fall-back is the whole defect this override
    /// closes (aegis-ll3p): an operator who points the guard at a scope file and
    /// mistypes the path must get a loud failure, not the ambient scope wearing
    /// the success of the command they meant to scope.
    pub fn resolve(override_path: Option<&Path>, root: &Path) -> Result<Self> {
        match override_path {
            Some(path) => Self::load_from(path),
            None => Self::load(root),
        }
    }

    /// Load configuration from exactly one file, over defaults — no discovery.
    ///
    /// The file must exist: [`read_hank_table`] returns `None` both for an
    /// absent file and for a present file with no `[hank]` table, and only the
    /// first is an error, so existence is checked explicitly. A present file
    /// with no `[hank]` table is a valid (if unusual) request for defaults.
    pub fn load_from(path: &Path) -> Result<Self> {
        if !path.exists() {
            return Err(Error::Config(format!(
                "--config path does not exist: {}",
                path.display()
            )));
        }
        match read_hank_table(path)? {
            Some(value) => value
                .try_into()
                .map_err(|e| Error::Config(format!("{}: [hank]: {e}", path.display()))),
            None => Ok(Self::default()),
        }
    }

    /// The layering itself, with the user-config path injected.
    ///
    /// Taking it as an argument keeps this testable without reassigning
    /// `$HOME`: that variable is process-global, and Cargo runs tests in
    /// threads, so a test that moved it would race every other test reading it.
    fn load_layered(user: Option<&Path>, root: &Path) -> Result<Self> {
        let project = root.join(".bobbin").join("config.toml");
        let sources = [user, Some(project.as_path())];

        let mut merged: Option<toml::Value> = None;
        for path in sources.into_iter().flatten() {
            let Some(table) = read_hank_table(path)? else {
                continue;
            };
            merged = Some(match merged {
                Some(base) => merge(base, table),
                None => table,
            });
        }

        match merged {
            None => Ok(Self::default()),
            Some(value) => value
                .try_into()
                .map_err(|e| Error::Config(format!("[hank]: {e}"))),
        }
    }
}

/// Deep-merge `overlay` onto `base`: tables merge key-by-key, everything else
/// is replaced outright.
///
/// Arrays replace rather than concatenate. Accumulating them would let a
/// workspace's `allow_paths` silently *widen* a scope the user config narrowed,
/// which inverts the direction a capability scope is allowed to move.
fn merge(base: toml::Value, overlay: toml::Value) -> toml::Value {
    match (base, overlay) {
        (toml::Value::Table(mut base), toml::Value::Table(overlay)) => {
            for (key, value) in overlay {
                let merged = match base.remove(&key) {
                    Some(existing) => merge(existing, value),
                    None => value,
                };
                base.insert(key, merged);
            }
            toml::Value::Table(base)
        }
        (_, overlay) => overlay,
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

/// Read the raw `[hank]` table from a config file, if the file exists.
///
/// Returns the un-deserialized [`toml::Value`] so callers can merge tables
/// before building the struct — deserializing each file separately would bake
/// in defaults for its absent keys, and those defaults would then overwrite
/// real values from a lower-precedence file.
///
/// Each file is still type-checked here, even though the result is discarded,
/// so a malformed `[hank]` is reported against the file that actually contains
/// it rather than surfacing later as an error about the merged whole.
fn read_hank_table(path: &Path) -> Result<Option<toml::Value>> {
    if !path.exists() {
        return Ok(None);
    }
    let text = std::fs::read_to_string(path)?;
    let root: toml::Value =
        toml::from_str(&text).map_err(|e| Error::Config(format!("{}: {e}", path.display())))?;
    match root.get("hank") {
        Some(section) => {
            let _: HankConfig = section
                .clone()
                .try_into()
                .map_err(|e| Error::Config(format!("{}: [hank]: {e}", path.display())))?;
            Ok(Some(section.clone()))
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

    /// The shape a fleet actually deploys: capability scopes live in ONE
    /// user-level config so six workspaces cannot drift, and each workspace
    /// sets its own unrelated keys. The workspace file must not take the policy
    /// down with it — a guard that silently stops enforcing is indistinguishable
    /// from a guard finding nothing wrong.
    #[test]
    fn a_project_config_does_not_wipe_user_level_policy() {
        let user = tempfile::tempdir().unwrap();
        let user_config = user.path().join("config.toml");
        std::fs::write(
            &user_config,
            "[hank.policy]\nmode = \"enforce\"\n\
             [hank.policy.scopes.weaver]\nallow_paths = [\"src/**\"]\n",
        )
        .unwrap();

        let project = tempfile::tempdir().unwrap();
        let bobbin = project.path().join(".bobbin");
        std::fs::create_dir_all(&bobbin).unwrap();
        // Sets one unrelated key; says nothing about policy.
        std::fs::write(
            bobbin.join("config.toml"),
            "[hank]\nbase_ref = \"develop\"\n",
        )
        .unwrap();

        let config = HankConfig::load_layered(Some(&user_config), project.path()).unwrap();
        // The workspace's own key wins...
        assert_eq!(config.base_ref, "develop");
        // ...without disarming the guard.
        assert_eq!(config.policy.mode, crate::policy::Mode::Enforce);
        let scope = config
            .policy
            .scope_for(Some("weaver"))
            .expect("user-level scope must survive a project config");
        assert_eq!(scope.allow_paths, vec!["src/**".to_string()]);
    }

    /// Merging must not cost precedence: the project still wins key-for-key.
    #[test]
    fn a_project_config_overrides_the_same_key() {
        let user = tempfile::tempdir().unwrap();
        let user_config = user.path().join("config.toml");
        std::fs::write(
            &user_config,
            "[hank]\nbase_ref = \"main\"\n[hank.policy]\nmode = \"enforce\"\n",
        )
        .unwrap();

        let project = tempfile::tempdir().unwrap();
        let bobbin = project.path().join(".bobbin");
        std::fs::create_dir_all(&bobbin).unwrap();
        std::fs::write(
            bobbin.join("config.toml"),
            "[hank.policy]\nmode = \"off\"\n",
        )
        .unwrap();

        let config = HankConfig::load_layered(Some(&user_config), project.path()).unwrap();
        assert_eq!(config.policy.mode, crate::policy::Mode::Off);
        // Untouched keys from the user config survive the override.
        assert_eq!(config.base_ref, "main");
    }

    /// A scope narrowed by the user config must not be widened by a workspace
    /// appending to it — arrays replace, they do not accumulate.
    #[test]
    fn a_project_config_replaces_rather_than_widens_allow_paths() {
        let user = tempfile::tempdir().unwrap();
        let user_config = user.path().join("config.toml");
        std::fs::write(
            &user_config,
            "[hank.policy]\nmode = \"enforce\"\n\
             [hank.policy.scopes.weaver]\nallow_paths = [\"src/**\"]\n",
        )
        .unwrap();

        let project = tempfile::tempdir().unwrap();
        let bobbin = project.path().join(".bobbin");
        std::fs::create_dir_all(&bobbin).unwrap();
        std::fs::write(
            bobbin.join("config.toml"),
            "[hank.policy.scopes.weaver]\nallow_paths = [\"docs/**\"]\n",
        )
        .unwrap();

        let config = HankConfig::load_layered(Some(&user_config), project.path()).unwrap();
        let scope = config.policy.scope_for(Some("weaver")).unwrap();
        assert_eq!(scope.allow_paths, vec!["docs/**".to_string()]);
    }

    /// `resolve(Some(path), ..)` reads exactly that file over defaults and never
    /// consults the ambient project config — the core of the `--config` fix.
    #[test]
    fn resolve_with_override_reads_the_named_file_not_the_cwd() {
        let project = tempfile::tempdir().unwrap();
        let bobbin = project.path().join(".bobbin");
        std::fs::create_dir_all(&bobbin).unwrap();
        std::fs::write(
            bobbin.join("config.toml"),
            "[hank]\nbase_ref = \"from-cwd\"\n",
        )
        .unwrap();

        let other = project.path().join("other.toml");
        std::fs::write(&other, "[hank]\nbase_ref = \"from-flag\"\n").unwrap();

        let overridden = HankConfig::resolve(Some(&other), project.path()).unwrap();
        assert_eq!(overridden.base_ref, "from-flag");

        // And without the override, discovery still finds the cwd config.
        let discovered = HankConfig::resolve(None, project.path()).unwrap();
        assert_eq!(discovered.base_ref, "from-cwd");
    }

    /// A `--config` path that does not exist is a loud error, not a silent
    /// fall-back to discovery.
    #[test]
    fn load_from_a_missing_path_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("nope.toml");
        let err = HankConfig::load_from(&missing).unwrap_err();
        assert!(err.to_string().contains("does not exist"));
    }

    /// A file that exists but has no `[hank]` table is a valid request for
    /// defaults, not an error.
    #[test]
    fn load_from_a_file_without_a_hank_table_yields_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("empty.toml");
        std::fs::write(&path, "[something_else]\nkey = 1\n").unwrap();
        let config = HankConfig::load_from(&path).unwrap();
        assert_eq!(config.base_ref, "main");
    }

    /// `serve.read_only` refuses a write, and is silent otherwise — the guard the
    /// docs promised and did not perform (aegis-ltjo).
    #[test]
    fn read_only_guards_a_write() {
        let mut config = HankConfig::default();
        assert!(
            config.write_guard("promotion").is_ok(),
            "default must allow writes"
        );

        config.serve.read_only = true;
        let err = config.write_guard("promotion").unwrap_err().to_string();
        assert!(
            err.contains("read_only"),
            "the error must name the key: {err}"
        );
        assert!(
            err.contains("promotion"),
            "the error must name the operation: {err}"
        );
    }

    /// THE anti-drift guard the bead asked for: every `pub` field on the config
    /// structs must either be READ somewhere outside config.rs, or be listed here
    /// with the phase that will honour it. A new inert key — documented, settable,
    /// doing nothing — fails this test until it is wired OR explicitly declared
    /// phased. That is the whole defect: a control that looks live and is not.
    #[test]
    fn every_config_key_is_read_or_explicitly_phased() {
        // key -> why it is not yet read. Anything not here MUST have a reader.
        let phased: &[(&str, &str)] = &[
            ("enable_lsp", "Phase 2/3 — LSP tier not built"),
            ("enable_cpg", "Phase 2 — CPG tier not built"),
            ("lsp_on", "LSP tier not built"),
            (
                "tenancy",
                "Phase 3 — overlay manager not built (whole sub-table)",
            ),
            ("max_overlays", "Phase 3 — overlay manager not built"),
            (
                "high_fanin_threshold",
                "Phase 3 — overlay manager not built",
            ),
            ("overlay_eviction", "Phase 3 — overlay manager not built"),
            ("promote_on", "Phase 4 — Quipu promotion not built"),
            ("shapes_path", "Phase 4 — Quipu promotion not built"),
        ];

        let manifest = env!("CARGO_MANIFEST_DIR");
        let config_rs = std::path::Path::new(manifest).join("src/config.rs");
        let this = std::fs::read_to_string(&config_rs).unwrap();

        // Every `pub <ident>:` field declared in this file.
        let fields: Vec<&str> = this
            .lines()
            .filter_map(|l| l.trim().strip_prefix("pub "))
            .filter_map(|rest| rest.split(':').next())
            .filter(|ident| ident.chars().all(|c| c.is_ascii_lowercase() || c == '_'))
            .filter(|ident| !ident.is_empty())
            .collect();
        assert!(fields.len() >= 15, "field parse looks wrong: {fields:?}");

        // All of src EXCEPT config.rs — the "is it read anywhere else" corpus.
        let mut elsewhere = String::new();
        collect_rs(
            std::path::Path::new(manifest).join("src").as_path(),
            &config_rs,
            &mut elsewhere,
        );

        for field in fields {
            let read = elsewhere.contains(field);
            let is_phased = phased.iter().any(|(k, _)| *k == field);
            assert!(
                read || is_phased,
                "config key `{field}` is read by NOTHING outside config.rs and is \
                 not in the phased allowlist. Wire it, or add it to `phased` with \
                 the phase that will honour it — a documented, settable, inert key \
                 is the defect aegis-ltjo closed."
            );
        }

        // And the allowlist must not rot the other way: a key listed as phased
        // that HAS gained a reader should be removed from the list.
        for (key, _why) in phased {
            assert!(
                !elsewhere.contains(key),
                "`{key}` is in the phased allowlist but now HAS a reader outside \
                 config.rs — remove it from the list and mark it live in the docs."
            );
        }
    }

    fn collect_rs(dir: &std::path::Path, skip: &std::path::Path, out: &mut String) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                collect_rs(&path, skip, out);
            } else if path.extension().is_some_and(|e| e == "rs") && path != skip {
                if let Ok(text) = std::fs::read_to_string(&path) {
                    out.push_str(&text);
                }
            }
        }
    }
}
