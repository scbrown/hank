//! Tenant registry + the composed `base + overlay` view (slice 2 of hank #2).
//!
//! [`TenantRegistry`] holds ONE shared [`Base`] and a fully-owned [`Overlay`]
//! per tenant, plus the FR-15 intern cache: parses are keyed by content hash,
//! so two tenants touching identical bytes share one [`ParsedFile`]
//! allocation. [`TenantView`] is the short-lived composition built per query
//! and dropped at its end — it borrows, it never owns, so no overlay ever
//! holds a base `NodeIndex`.
//!
//! **Isolation is structural (§6.3):** a view composes exactly one tenant's
//! overlay by construction; there is no path from tenant A's query to tenant
//! B's overlay because no shared mutable state exists below the registry —
//! the base is immutable behind `Arc`, overlays are disjoint map entries, and
//! interned parses are immutable once created.
//!
//! **Resolution rule:** a touched file is MASKED — the overlay owns its
//! truth. Definitions of a name are the overlay's, plus base definitions in
//! untouched files. Edges compose the same way: base↔base edges are the
//! materialized graph, overlay calls are the overlay's own records, and a
//! base edge into a touched file is remapped BY NAME to the overlay's
//! definitions (the call lives in the untouched caller; only the callee's
//! identity changed hands). One deliberate slice-2 limit, closed by the
//! FR-16 frontier recompute (hank #3): a base caller's call to a name with
//! ZERO base definitions is not in the base graph at all, so an overlay-new
//! symbol cannot see that caller until the frontier is recomputed.

use std::collections::HashMap;
use std::sync::Arc;

use petgraph::graph::NodeIndex;
use petgraph::Direction as PgDirection;

use super::overlay::{Overlay, ParsedFile};
use super::{Adjacency, Base, Dir, NodeMeta};

/// A node handle in the composed view — base and overlay ids kept distinct,
/// never a masked base node.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SymRef {
    /// A node of the shared base graph (its file is untouched).
    Base(NodeIndex),
    /// An overlay-local symbol id.
    Overlay(u32),
}

/// The tenant/session registry: one shared base, one overlay per tenant, one
/// FR-15 intern cache across them. This is the object the resident daemon
/// holds for its lifetime; views are made per query.
pub struct TenantRegistry {
    base: Arc<Base>,
    overlays: HashMap<String, Overlay>,
    /// Content-hash → parse. Immutable entries; unbounded until the FR-18
    /// lifecycle/eviction work (hank #6) gives it a policy.
    intern: HashMap<String, Arc<ParsedFile>>,
}

impl TenantRegistry {
    /// A registry over `base`, with no tenants yet.
    #[must_use]
    pub fn new(base: Arc<Base>) -> Self {
        Self {
            base,
            overlays: HashMap::new(),
            intern: HashMap::new(),
        }
    }

    /// The shared base.
    #[must_use]
    pub fn base(&self) -> &Arc<Base> {
        &self.base
    }

    /// Record `source` as tenant's truth for `rel` (creating the tenant on
    /// first touch). The parse is interned by content hash: identical bytes
    /// across tenants share one allocation (FR-15).
    pub fn touch(&mut self, tenant: &str, rel: &str, source: &str) {
        let parsed = ParsedFile::parse(rel, source);
        let shared = self
            .intern
            .entry(parsed.hash.clone())
            .or_insert_with(|| Arc::new(parsed))
            .clone();
        self.overlays
            .entry(tenant.to_string())
            .or_default()
            .touch(rel, shared);
    }

    /// Drop tenant's touch of `rel` (base resumes answering). No-op for an
    /// unknown tenant or untouched file.
    pub fn revert(&mut self, tenant: &str, rel: &str) {
        if let Some(overlay) = self.overlays.get_mut(tenant) {
            overlay.revert(rel);
        }
    }

    /// Remove a tenant and its overlay entirely (session close).
    pub fn drop_tenant(&mut self, tenant: &str) {
        self.overlays.remove(tenant);
    }

    /// The tenant's overlay, if it has one.
    #[must_use]
    pub fn overlay(&self, tenant: &str) -> Option<&Overlay> {
        self.overlays.get(tenant)
    }

    /// Tenants with an overlay, sorted.
    #[must_use]
    pub fn tenants(&self) -> Vec<&str> {
        let mut t: Vec<&str> = self.overlays.keys().map(String::as_str).collect();
        t.sort_unstable();
        t
    }

    /// The composed view for `tenant` — what every query resolves against. A
    /// tenant with no overlay (including one never seen) views the bare base:
    /// the empty overlay IS the base behavior.
    #[must_use]
    pub fn view(&self, tenant: &str) -> TenantView<'_> {
        TenantView {
            base: &self.base,
            overlay: self.overlays.get(tenant),
        }
    }

    /// What `hank_status` reports about the tenant layer: the base commit and
    /// active overlays, by tenant, with their `O(touched)` sizes.
    #[must_use]
    pub fn status(&self) -> RegistryStatus {
        let mut active: Vec<OverlayStatus> = self
            .overlays
            .iter()
            .map(|(tenant, overlay)| OverlayStatus {
                tenant: tenant.clone(),
                touched_files: overlay.touched_count(),
                symbols: overlay.symbol_count(),
            })
            .collect();
        active.sort_by(|a, b| a.tenant.cmp(&b.tenant));
        RegistryStatus {
            base_commit: self.base.commit().to_string(),
            active_overlays: active,
        }
    }
}

/// One active overlay in a [`RegistryStatus`].
#[derive(Debug, Clone, serde::Serialize, PartialEq, Eq)]
pub struct OverlayStatus {
    /// The tenant holding it.
    pub tenant: String,
    /// Files it has touched (masked from the base).
    pub touched_files: usize,
    /// Symbols it defines — `O(touched)`, never repo-sized.
    pub symbols: usize,
}

/// The registry's status snapshot: base commit + active overlays. Serialized
/// into `hank_status`/`/status` when the resident daemon holds a registry.
#[derive(Debug, Clone, serde::Serialize, PartialEq, Eq)]
pub struct RegistryStatus {
    /// The resolved commit the shared base was built at.
    pub base_commit: String,
    /// Active overlays, sorted by tenant.
    pub active_overlays: Vec<OverlayStatus>,
}

/// One tenant's `base + overlay` composition, built per query, dropped at its
/// end. Implements [`Adjacency`], so the FR-12 BFS walks it unchanged.
pub struct TenantView<'a> {
    base: &'a Base,
    overlay: Option<&'a Overlay>,
}

/// One symbol as the composed view resolves it — the serve-path shape
/// (definitions and per-file listings), with the kind the wire DTOs need.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ViewSymbol {
    /// Symbol name.
    pub name: String,
    /// Symbol kind (lowercase form).
    pub kind: String,
    /// File, relative to the root.
    pub file: String,
    /// 1-based definition line.
    pub start_line: usize,
}

impl TenantView<'_> {
    /// Whether `name` is defined anywhere in the composed view.
    #[must_use]
    pub fn has_symbol(&self, name: &str) -> bool {
        !self.seeds(name).is_empty()
    }

    /// The composed definition sites of `name` — overlay definitions plus
    /// unmasked base ones, the same resolution the walk seeds from.
    #[must_use]
    pub fn definitions(&self, name: &str) -> Vec<ViewSymbol> {
        self.defs_of_name(name, None)
            .into_iter()
            .map(|r| self.view_symbol(r))
            .collect()
    }

    /// The symbols `rel` contributes to the composed view, in line order:
    /// the overlay's if touched (masking), the base's otherwise.
    #[must_use]
    pub fn file_symbols(&self, rel: &str) -> Vec<ViewSymbol> {
        if let Some(overlay) = self.overlay {
            if overlay.is_touched(rel) {
                let mut out: Vec<ViewSymbol> = (0..overlay.symbol_count() as u32)
                    .filter(|&id| overlay.symbol(id).file == rel)
                    .map(|id| self.view_symbol(SymRef::Overlay(id)))
                    .collect();
                out.sort_by_key(|s| s.start_line);
                return out;
            }
        }
        self.base
            .graph()
            .file_symbols(rel)
            .into_iter()
            .map(|n| ViewSymbol {
                name: n.name.clone(),
                kind: n.kind.clone(),
                file: n.file.clone(),
                start_line: n.start_line,
            })
            .collect()
    }

    fn view_symbol(&self, r: SymRef) -> ViewSymbol {
        match r {
            SymRef::Base(ix) => {
                let n = &self.base.graph().graph[ix];
                ViewSymbol {
                    name: n.name.clone(),
                    kind: n.kind.clone(),
                    file: n.file.clone(),
                    start_line: n.start_line,
                }
            }
            SymRef::Overlay(id) => {
                let s = self
                    .overlay
                    .expect("an overlay id implies an overlay")
                    .symbol(id);
                ViewSymbol {
                    name: s.name.clone(),
                    kind: s.kind.clone(),
                    file: s.file.clone(),
                    start_line: s.start_line,
                }
            }
        }
    }

    fn touched(&self, rel: &str) -> bool {
        self.overlay.is_some_and(|o| o.is_touched(rel))
    }

    /// The language a base node's facts belong to (by file extension — the
    /// same rule the graph build applies).
    fn base_language(&self, ix: NodeIndex) -> Option<&'static str> {
        let file = &self.base.graph().graph[ix].file;
        std::path::Path::new(file)
            .extension()
            .and_then(std::ffi::OsStr::to_str)
            .and_then(crate::extract::language_for_extension)
    }

    /// Resolve a callee NAME against the view for a caller of `language`:
    /// overlay definitions plus unmasked base definitions, same language only.
    fn defs_of_name(&self, name: &str, language: Option<&'static str>) -> Vec<SymRef> {
        let mut out = Vec::new();
        if let Some(overlay) = self.overlay {
            for &id in overlay.defs(name) {
                if language.is_none_or(|l| overlay.symbol(id).language == l) {
                    out.push(SymRef::Overlay(id));
                }
            }
        }
        for &ix in self
            .base
            .graph()
            .by_name
            .get(name)
            .map_or(&[][..], Vec::as_slice)
        {
            let node = &self.base.graph().graph[ix];
            if !self.touched(&node.file)
                && language.is_none_or(|l| self.base_language(ix) == Some(l))
            {
                out.push(SymRef::Base(ix));
            }
        }
        out
    }

    /// Base callers of `name` from UNTOUCHED files — reconstructed from the
    /// materialized edges of every base definition of the name (masked ones
    /// included: the call lives in the caller's file, which is untouched).
    fn base_callers_of_name(&self, name: &str) -> Vec<SymRef> {
        let mut out = Vec::new();
        for &def in self
            .base
            .graph()
            .by_name
            .get(name)
            .map_or(&[][..], Vec::as_slice)
        {
            for caller in self
                .base
                .graph()
                .graph
                .neighbors_directed(def, PgDirection::Incoming)
            {
                let file = &self.base.graph().graph[caller].file;
                if !self.touched(file) {
                    out.push(SymRef::Base(caller));
                }
            }
        }
        out
    }
}

impl Adjacency for TenantView<'_> {
    type Node = SymRef;

    fn seeds(&self, name: &str) -> Vec<SymRef> {
        self.defs_of_name(name, None)
    }

    fn neighbors(&self, node: SymRef, dir: Dir) -> Vec<SymRef> {
        let mut out = Vec::new();
        match (node, dir) {
            (SymRef::Base(ix), Dir::Callees) => {
                // Materialized base edges; a callee in a touched file is
                // remapped BY NAME to the overlay's definitions (deduped —
                // several masked defs of one name remap to the same targets).
                let language = self.base_language(ix);
                let mut remap: Vec<&str> = Vec::new();
                for callee in self
                    .base
                    .graph()
                    .graph
                    .neighbors_directed(ix, PgDirection::Outgoing)
                {
                    let n = &self.base.graph().graph[callee];
                    if self.touched(&n.file) {
                        if !remap.contains(&n.name.as_str()) {
                            remap.push(&n.name);
                        }
                    } else {
                        out.push(SymRef::Base(callee));
                    }
                }
                if let Some(overlay) = self.overlay {
                    for name in remap {
                        for &id in overlay.defs(name) {
                            if language.is_none_or(|l| overlay.symbol(id).language == l) {
                                out.push(SymRef::Overlay(id));
                            }
                        }
                    }
                }
            }
            (SymRef::Base(ix), Dir::Callers) => {
                // Base callers via materialized edges (untouched files only —
                // a touched caller's calls are the OVERLAY's records now, so
                // a deleted call never resurrects), plus overlay callers of
                // this node's name.
                for caller in self
                    .base
                    .graph()
                    .graph
                    .neighbors_directed(ix, PgDirection::Incoming)
                {
                    let file = &self.base.graph().graph[caller].file;
                    if !self.touched(file) {
                        out.push(SymRef::Base(caller));
                    }
                }
                if let Some(overlay) = self.overlay {
                    let me = &self.base.graph().graph[ix];
                    let language = self.base_language(ix);
                    for &id in overlay.callers_of(&me.name) {
                        if language.is_none_or(|l| overlay.symbol(id).language == l) {
                            out.push(SymRef::Overlay(id));
                        }
                    }
                }
            }
            (SymRef::Overlay(id), Dir::Callees) => {
                let overlay = self.overlay.expect("an overlay id implies an overlay");
                let me = overlay.symbol(id);
                for name in overlay.callee_names(id) {
                    for target in self.defs_of_name(name, Some(me.language)) {
                        if target != node && !out.contains(&target) {
                            out.push(target);
                        }
                    }
                }
            }
            (SymRef::Overlay(id), Dir::Callers) => {
                let overlay = self.overlay.expect("an overlay id implies an overlay");
                let me = overlay.symbol(id);
                for &caller in overlay.callers_of(&me.name) {
                    if overlay.symbol(caller).language == me.language && caller != id {
                        out.push(SymRef::Overlay(caller));
                    }
                }
                // Base callers of my NAME (see the module doc for the
                // overlay-new-name limit until the FR-16 frontier recompute).
                out.extend(self.base_callers_of_name(&me.name));
            }
        }
        out
    }

    fn meta(&self, node: SymRef) -> NodeMeta {
        match node {
            SymRef::Base(ix) => {
                let n = &self.base.graph().graph[ix];
                NodeMeta {
                    name: n.name.clone(),
                    file: n.file.clone(),
                    start_line: n.start_line,
                }
            }
            SymRef::Overlay(id) => {
                let s = self
                    .overlay
                    .expect("an overlay id implies an overlay")
                    .symbol(id);
                NodeMeta {
                    name: s.name.clone(),
                    file: s.file.clone(),
                    start_line: s.start_line,
                }
            }
        }
    }
}
