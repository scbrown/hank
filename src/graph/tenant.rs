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
//! identity changed hands). A base caller's call to a name the overlay
//! INTRODUCED (zero base definitions, so no materialized edge) is resolved
//! through the base's FR-16 frontier index (`callers_of_name`) — this is
//! what closes the slice-2 overlay-new-name gap (hank #3).

use std::collections::HashMap;
use std::sync::Arc;

use petgraph::graph::NodeIndex;
use petgraph::Direction as PgDirection;

use super::overlay::{Overlay, ParsedFile};
use super::{Adjacency, Base, Dir, NodeMeta, Reached};

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
///
/// Overlay lifecycle (FR-18, §14.2): overlays are created on first
/// touch/[`open_session`](Self::open_session), removed on
/// [`close_session`](Self::close_session), cleared to base by
/// [`reset`](Self::reset), and capped at `tenancy.max_overlays` — a new
/// overlay past the cap evicts one per `tenancy.overlay_eviction`, always
/// logged, never a silent drop.
pub struct TenantRegistry {
    base: Arc<Base>,
    overlays: HashMap<String, Overlay>,
    /// Content-hash → parse. Immutable entries; retained past revert until a
    /// tenant is closed (then swept — see [`close_session`](Self::close_session)).
    intern: HashMap<String, Arc<ParsedFile>>,
    /// The `[hank.tenancy]` limits: cap, eviction policy, fan-in threshold.
    tenancy: crate::config::TenancyConfig,
    /// Monotonic clock for recency; bumped on every touch/open.
    tick: u64,
    /// Per-tenant `(created_tick, last_used_tick)` — LRU evicts the min
    /// last-used, the `on_session_close` backstop evicts the min created.
    stamps: HashMap<String, (u64, u64)>,
}

impl TenantRegistry {
    /// A registry over `base` with the default tenancy limits, no tenants yet.
    #[must_use]
    pub fn new(base: Arc<Base>) -> Self {
        Self::with_tenancy(base, crate::config::TenancyConfig::default())
    }

    /// A registry over `base` with explicit `[hank.tenancy]` limits — what the
    /// resident daemon and `hank watch` build so the configured cap/policy/
    /// fan-in threshold are actually honored.
    #[must_use]
    pub fn with_tenancy(base: Arc<Base>, tenancy: crate::config::TenancyConfig) -> Self {
        Self {
            base,
            overlays: HashMap::new(),
            intern: HashMap::new(),
            tenancy,
            tick: 0,
            stamps: HashMap::new(),
        }
    }

    /// The shared base.
    #[must_use]
    pub fn base(&self) -> &Arc<Base> {
        &self.base
    }

    /// The tenancy limits in force.
    #[must_use]
    pub fn tenancy(&self) -> &crate::config::TenancyConfig {
        &self.tenancy
    }

    /// Register `tenant`'s session (FR-18): create its overlay slot if absent,
    /// enforcing the `max_overlays` cap first. Idempotent for an open session
    /// (just refreshes recency). Returns the tenant evicted to make room, if any.
    pub fn open_session(&mut self, tenant: &str) -> Option<String> {
        let evicted = self.ensure_capacity_for(tenant);
        self.tick += 1;
        let tick = self.tick;
        self.stamps
            .entry(tenant.to_string())
            .and_modify(|(_, used)| *used = tick)
            .or_insert((tick, tick));
        self.overlays.entry(tenant.to_string()).or_default();
        evicted
    }

    /// Close `tenant`'s session: remove its overlay and metadata entirely (the
    /// `on_session_close` eviction), then sweep interned parses no live overlay
    /// references any more. No-op for an unknown tenant.
    pub fn close_session(&mut self, tenant: &str) {
        self.overlays.remove(tenant);
        self.stamps.remove(tenant);
        self.sweep_intern();
    }

    /// Reset `tenant` to base: clear its overlay (the session stays open, its
    /// recency preserved) so its view is the bare base again. No-op if unknown.
    pub fn reset(&mut self, tenant: &str) {
        if let Some(overlay) = self.overlays.get_mut(tenant) {
            *overlay = Overlay::default();
            self.sweep_intern();
        }
    }

    /// If `tenant` is new and the registry is at `max_overlays`, evict one per
    /// policy to make room. Returns the evicted tenant. Logged, never silent.
    fn ensure_capacity_for(&mut self, tenant: &str) -> Option<String> {
        if self.overlays.contains_key(tenant) || self.overlays.len() < self.tenancy.max_overlays {
            return None;
        }
        let lru = self.tenancy.overlay_eviction == "lru";
        // LRU → smallest last-used; on_session_close backstop → smallest created.
        let victim = self
            .stamps
            .iter()
            .filter(|(t, _)| self.overlays.contains_key(t.as_str()))
            .min_by_key(|(_, (created, used))| if lru { *used } else { *created })
            .map(|(t, _)| t.clone())?;
        tracing::warn!(
            evicted = %victim, policy = %self.tenancy.overlay_eviction,
            cap = self.tenancy.max_overlays, opening = %tenant,
            "overlay cap reached — evicting to make room (FR-18)"
        );
        self.overlays.remove(&victim);
        self.stamps.remove(&victim);
        self.sweep_intern();
        Some(victim)
    }

    /// Drop interned parses no live overlay holds any more (`Arc` strong count
    /// back to 1 — only the cache). Keeps the intern cache `O(live overlays)`.
    fn sweep_intern(&mut self) {
        self.intern.retain(|_, arc| Arc::strong_count(arc) > 1);
    }

    /// Record `source` as tenant's truth for `rel` (creating the tenant on
    /// first touch). Content-hash structural sharing (FR-15) on two levels:
    ///
    /// - **Base hit → no overlay storage.** If `source` hashes to what the base
    ///   already holds for `rel`, the overlay would only re-state base truth, so
    ///   nothing is stored — any prior touch of `rel` is dropped and the base
    ///   resumes answering. This is the primary §6.2 lever: a tenant whose edits
    ///   match the baseline costs nothing.
    /// - **Cross-tenant hit → shared parse.** Otherwise the parse is interned by
    ///   content hash, so N tenants holding identical bytes share ONE
    ///   [`ParsedFile`] allocation rather than N copies.
    pub fn touch(&mut self, tenant: &str, rel: &str, source: &str) {
        let parsed = ParsedFile::parse(rel, source);
        // Base hit: identical to the baseline ⇒ the overlay adds nothing. Does
        // NOT open a session (nothing is stored), so it never triggers eviction.
        if self.base.file(rel).is_some_and(|f| f.hash == parsed.hash) {
            if let Some(overlay) = self.overlays.get_mut(tenant) {
                overlay.revert(rel);
            }
            return;
        }
        // A real touch opens/refreshes the session (cap-enforced, recency-bumped).
        self.open_session(tenant);
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

    /// FR-15 sharing stats over the live overlays (§6.2): how much overlay
    /// storage the content-hash intern is actually saving right now.
    #[must_use]
    pub fn sharing_stats(&self) -> SharingStats {
        let total_touches: usize = self.overlays.values().map(Overlay::touched_count).sum();
        // Distinct parses ACTUALLY referenced by a live overlay — not
        // `intern.len()`, which also counts parses whose only holders have
        // since reverted (retained until FR-18 eviction, hank #6).
        let mut live: std::collections::BTreeSet<*const ParsedFile> =
            std::collections::BTreeSet::new();
        for overlay in self.overlays.values() {
            for rel in overlay.touched() {
                if let Some(p) = overlay.parsed(rel) {
                    live.insert(Arc::as_ptr(p));
                }
            }
        }
        SharingStats {
            total_touches,
            unique_parses: live.len(),
            interned: self.intern.len(),
        }
    }

    /// Drop tenant's touch of `rel` (base resumes answering). No-op for an
    /// unknown tenant or untouched file.
    pub fn revert(&mut self, tenant: &str, rel: &str) {
        if let Some(overlay) = self.overlays.get_mut(tenant) {
            overlay.revert(rel);
        }
    }

    /// Remove a tenant and its overlay entirely. Alias for
    /// [`close_session`](Self::close_session) (the FR-18 name); both sweep the
    /// intern cache.
    pub fn drop_tenant(&mut self, tenant: &str) {
        self.close_session(tenant);
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

/// FR-15 sharing measurement over the live overlays (§6.2). The lever is the
/// gap between `total_touches` (what N tenants each hold) and `unique_parses`
/// (the distinct [`ParsedFile`] allocations actually backing them) — when
/// tenants edit alike, the second stays flat while the first grows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SharingStats {
    /// Touched files summed across every live overlay.
    pub total_touches: usize,
    /// Distinct parse allocations those touches share (by pointer identity).
    pub unique_parses: usize,
    /// Entries in the intern cache — `>= unique_parses`, the excess being
    /// parses retained after all their overlays reverted (freed by FR-18
    /// eviction, hank #6; the excess is logged, never silently dropped).
    pub interned: usize,
}

impl SharingStats {
    /// Overlay parses saved by sharing: `total_touches - unique_parses`. Zero
    /// when every touch is unique; grows as tenants converge on the same bytes.
    #[must_use]
    pub fn saved(&self) -> usize {
        self.total_touches.saturating_sub(self.unique_parses)
    }

    /// Sharing ratio in `[0.0, 1.0]`: the fraction of touches that cost no new
    /// parse. `0.0` when nothing is touched or nothing is shared.
    #[must_use]
    pub fn ratio(&self) -> f64 {
        if self.total_touches == 0 {
            0.0
        } else {
            self.saved() as f64 / self.total_touches as f64
        }
    }
}

/// The result of a fan-in-bounded frontier walk (§14.2). `bounded_seeds` names
/// the high-fan-in symbols whose cascade was clipped to one hop — the caller
/// logs them, so bounding is always visible, never a silent truncation.
#[derive(Debug, Clone)]
pub struct BoundedFrontier {
    /// The reached frontier symbols.
    pub reached: Vec<Reached>,
    /// Seed names whose cascade was bounded (fan-in over threshold).
    pub bounded_seeds: Vec<String>,
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

    /// The FR-16 frontier of editing `changed` symbols: everything whose facts
    /// an edit can perturb — the changed symbols' transitive callers (impact)
    /// AND callees (dependencies), within `hops`, over the COMPOSED view. This
    /// is the second caller of the one `reachable_over` BFS (FR-12, "build it
    /// once"); it walks base+overlay, so the frontier spans the tenant's edit
    /// and the shared base — including base callers of a name the overlay just
    /// introduced. Deduplicated by (name, file); the seeds themselves are
    /// excluded (the frontier is what the edit REACHES). Each item keeps its
    /// `distance`/`via`, so a caller can see how far a consequence propagated.
    #[must_use]
    pub fn frontier(&self, changed: &[&str], hops: u32) -> Vec<Reached> {
        self.frontier_bounded(changed, hops, usize::MAX).reached
    }

    /// The FR-16 frontier with the §14.2 high-fan-in guard: a seed whose direct
    /// (1-hop) fan already exceeds `high_fanin_threshold` is a widely-referenced
    /// signature whose transitive cascade could blow the budget, so it is walked
    /// only ONE hop instead of `hops`. The seed names so bounded are returned
    /// (and logged by the caller) — the cascade is bounded, never silently
    /// truncated. `usize::MAX` disables the guard (plain [`frontier`](Self::frontier)).
    #[must_use]
    pub fn frontier_bounded(
        &self,
        changed: &[&str],
        hops: u32,
        high_fanin_threshold: usize,
    ) -> BoundedFrontier {
        let seeds: std::collections::BTreeSet<&str> = changed.iter().copied().collect();
        let mut seen: std::collections::BTreeSet<(String, String)> =
            std::collections::BTreeSet::new();
        let mut out = Vec::new();
        let mut bounded_seeds = Vec::new();
        for &name in changed {
            // Direct fan = 1-hop callers + callees. Cheap, and it is exactly the
            // "widely referenced" measure §14.2 warns about.
            let direct_fan = super::reachable_over(self, name, Dir::Callers, 1).len()
                + super::reachable_over(self, name, Dir::Callees, 1).len();
            let effective_hops = if direct_fan > high_fanin_threshold {
                bounded_seeds.push(name.to_string());
                1
            } else {
                hops
            };
            for dir in [Dir::Callers, Dir::Callees] {
                for r in super::reachable_over(self, name, dir, effective_hops) {
                    if !seeds.contains(r.name.as_str())
                        && seen.insert((r.name.clone(), r.file.clone()))
                    {
                        out.push(r);
                    }
                }
            }
        }
        BoundedFrontier {
            reached: out,
            bounded_seeds,
        }
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

    /// Base callers of `name` from UNTOUCHED files, of `language`, via the
    /// FR-16 frontier index. This answers even when `name` has NO base
    /// definition — the overlay-new symbol a tenant just introduced (hank #3) —
    /// which walking a definition's incoming edges never could.
    fn base_callers_of_name(&self, name: &str, language: &'static str) -> Vec<SymRef> {
        let mut out = Vec::new();
        for &caller in self.base.graph().callers_of_name(name) {
            let file = self.base.graph().node_file(caller);
            if !self.touched(file) && self.base_language(caller) == Some(language) {
                out.push(SymRef::Base(caller));
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
                // Base callers of my NAME, via the FR-16 frontier index — this
                // now answers even when the overlay INTRODUCED the name (zero
                // base defs), which the old edge-walk could not (hank #3).
                out.extend(self.base_callers_of_name(&me.name, me.language));
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
