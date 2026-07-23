//! The tenant-aware tier handler (FR-17 × FR-16): on-disk edits drive a
//! tenant's overlay, and the heavy tier is the frontier recompute the module
//! doc always promised — not a full `CodeGraph` rebuild.
//!
//! Two tiers, two freshness meanings:
//! - **tree-sitter (fast):** re-parse the changed file into the tenant's
//!   overlay (`registry.touch`). The overlay's own facts are now current, but
//!   the *frontier* — who else an edit to that file affects — has not been
//!   recomputed, so the file is marked [`Freshness::Recomputing`].
//! - **heavy (deferred):** recompute the frontier over the composed view via
//!   [`crate::graph::update_frontier`] (the one FR-12 BFS), then mark the file
//!   [`Freshness::Fresh`]. This is the tier that used to rebuild the whole
//!   graph; overlays + FR-16 let it touch only the edited files + their reach.
//!
//! Freshness is tracked HERE, on the update path that actually knows it — not
//! stamped onto served DTOs that have no tracking behind them (the honesty rule
//! from `types.rs`). A caller that wants a fact's freshness asks this handler.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};

use super::TierHandler;
use crate::graph::{update_frontier_bounded, TenantRegistry};
use crate::types::{Freshness, Tier};

/// Drives one tenant's overlay from its working-tree edits, tracking per-file
/// freshness across the two tiers.
pub struct OverlayRefresh {
    registry: Arc<RwLock<TenantRegistry>>,
    tenant: String,
    root: PathBuf,
    hops: u32,
    /// Per-file freshness, shared so a query side can read it while the worker
    /// thread writes it. Keyed by root-relative path.
    freshness: Arc<Mutex<HashMap<String, Freshness>>>,
}

impl OverlayRefresh {
    /// A handler feeding `tenant`'s overlay in `registry`, for the tree rooted
    /// at `root`, recomputing frontiers up to `hops`.
    #[must_use]
    pub fn new(
        registry: Arc<RwLock<TenantRegistry>>,
        tenant: impl Into<String>,
        root: PathBuf,
        hops: u32,
    ) -> Self {
        Self {
            registry,
            tenant: tenant.into(),
            root,
            hops,
            freshness: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// A handle to the per-file freshness map — the query side reads a fact's
    /// freshness from here (it is not stamped onto DTOs blindly).
    #[must_use]
    pub fn freshness_handle(&self) -> Arc<Mutex<HashMap<String, Freshness>>> {
        Arc::clone(&self.freshness)
    }

    /// The freshness of `rel` (root-relative), or `None` if never edited.
    #[must_use]
    pub fn freshness_of(&self, rel: &str) -> Option<Freshness> {
        self.freshness.lock().ok()?.get(rel).copied()
    }

    /// Root-relative form of a watched absolute path.
    fn rel_of(&self, path: &Path) -> String {
        path.strip_prefix(&self.root)
            .unwrap_or(path)
            .display()
            .to_string()
    }

    fn set_freshness(&self, rel: &str, f: Freshness) {
        if let Ok(mut map) = self.freshness.lock() {
            map.insert(rel.to_string(), f);
        }
    }
}

impl TierHandler for OverlayRefresh {
    fn tree_sitter(&mut self, paths: &[PathBuf]) {
        for path in paths {
            let rel = self.rel_of(path);
            match std::fs::read_to_string(path) {
                Ok(source) => {
                    if let Ok(mut reg) = self.registry.write() {
                        reg.touch(&self.tenant, &rel, &source);
                    }
                    // Overlay facts current; frontier not yet recomputed.
                    self.set_freshness(&rel, Freshness::Recomputing);
                    tracing::info!(
                        tenant = %self.tenant, file = %rel, tier = Tier::TreeSitter.as_str(),
                        "overlay touched (tree-sitter); frontier recomputing"
                    );
                }
                Err(_) => {
                    // Removed on disk → an empty touch masks it, matching the
                    // deletion-is-the-empty-touch overlay contract.
                    if let Ok(mut reg) = self.registry.write() {
                        reg.touch(&self.tenant, &rel, "");
                    }
                    self.set_freshness(&rel, Freshness::Recomputing);
                }
            }
        }
    }

    fn heavy(&mut self, paths: &[PathBuf]) {
        let Ok(reg) = self.registry.read() else {
            return;
        };
        let view = reg.view(&self.tenant);
        // Seed the frontier with the symbols the edited files define (their
        // callers/callees are the affected set — FR-16).
        let rels: Vec<String> = paths.iter().map(|p| self.rel_of(p)).collect();
        let mut seed_names: Vec<String> = Vec::new();
        for rel in &rels {
            for s in view.file_symbols(rel) {
                seed_names.push(s.name);
            }
        }
        let seeds: Vec<&str> = seed_names.iter().map(String::as_str).collect();
        // The §14.2 fan-in guard bounds a hot signature's cascade (logged).
        let threshold = reg.tenancy().high_fanin_threshold;
        let frontier = update_frontier_bounded(&view, &seeds, self.hops, threshold);
        tracing::info!(
            tenant = %self.tenant, changed = rels.len(), frontier = frontier.len(),
            "frontier recomputed (heavy tier)"
        );
        // The frontier is current for these files now. `set_freshness` touches
        // only `self.freshness`, so the read view above may live to scope end.
        for rel in &rels {
            self.set_freshness(rel, Freshness::Fresh);
        }
    }
}

#[cfg(test)]
#[path = "overlay_refresh_test.rs"]
mod overlay_refresh_test;
