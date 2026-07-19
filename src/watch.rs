//! File-watch, debounce, and tiered re-extraction scheduling (FR-17).
//!
//! This is the on-disk edit source for the incremental update path. A `notify`
//! watcher observes the repo tree; raw filesystem events are noisy (an editor
//! save can emit several within milliseconds), so they are **debounced** into a
//! quiet-window batch before any work runs. Work is then dispatched in **tiers**
//! (FR-17): the cheap tree-sitter re-parse fires on a short window (per-keystroke
//! feel), while heavier recompute — the frontier/graph work, and later the LSP
//! and CPG tiers — is deferred behind a longer window so a burst of edits does
//! not thrash it.
//!
//! The design keeps the scheduling logic (`Debouncer`, `TieredScheduler`) pure
//! and time-injected so it is deterministically testable without real
//! filesystem events or sleeps; the `notify` wiring in [`Watcher`] is a thin
//! adapter that feeds those components and invokes a [`TierHandler`].
//!
//! ## Relationship to the incremental update path
//!
//! The tiers here **feed** the existing extraction entrypoints rather than
//! replacing them: the tree-sitter tier re-runs [`crate::extract::extract_structure`]
//! per changed file (FR-16 step 1, "re-parse X"), and the heavy tier rebuilds the
//! [`crate::graph::CodeGraph`]. When the Phase-3 copy-on-write overlay and the
//! frontier-bounded update (FR-16) land, the heavy tier becomes a call into the
//! overlay's `update_frontier` instead of a full rebuild — the scheduler and
//! watcher above it are unchanged. See `docs/hank-spec.md` §5.5, §7.5, FR-16/17.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use notify::{RecommendedWatcher, RecursiveMode, Watcher as _};

use crate::config::FreshnessConfig;
use crate::graph::CodeGraph;
use crate::types::Tier;

/// A last-write-wins debouncer over paths.
///
/// Each observed change for a path (re)stamps that path's deadline at
/// `event_time + window`. [`flush_ready`](Self::flush_ready) returns and removes
/// the paths whose deadline has passed as of the supplied `now`. Repeated events
/// for the same path within the window coalesce into one flush — the point of
/// debouncing.
///
/// Time is passed in explicitly rather than read from a clock, so the whole
/// component is deterministically testable.
#[derive(Debug)]
pub struct Debouncer {
    window: Duration,
    /// path → instant at which it becomes ready to flush.
    deadlines: HashMap<PathBuf, Instant>,
}

impl Debouncer {
    /// Create a debouncer with the given quiet `window`.
    #[must_use]
    pub fn new(window: Duration) -> Self {
        Self {
            window,
            deadlines: HashMap::new(),
        }
    }

    /// Record a change for `path` observed at `at`; resets its quiet window.
    pub fn record(&mut self, path: PathBuf, at: Instant) {
        self.deadlines.insert(path, at + self.window);
    }

    /// Remove and return every path whose quiet window elapsed by `now`.
    ///
    /// The returned paths are sorted for a stable, reproducible batch order.
    pub fn flush_ready(&mut self, now: Instant) -> Vec<PathBuf> {
        let mut ready: Vec<PathBuf> = self
            .deadlines
            .iter()
            .filter(|(_, &deadline)| deadline <= now)
            .map(|(path, _)| path.clone())
            .collect();
        for path in &ready {
            self.deadlines.remove(path);
        }
        ready.sort();
        ready
    }

    /// The earliest deadline still pending, if any — when the caller should next
    /// wake to flush.
    #[must_use]
    pub fn next_deadline(&self) -> Option<Instant> {
        self.deadlines.values().copied().min()
    }

    /// Whether nothing is pending.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.deadlines.is_empty()
    }
}

/// One tier's worth of ready paths produced by a scheduler poll.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct TierBatch {
    /// Files ready for the cheap tree-sitter re-parse ([`Tier::TreeSitter`]).
    pub tree_sitter: Vec<PathBuf>,
    /// Files ready for the deferred heavy recompute (graph/frontier; later LSP/CPG).
    pub heavy: Vec<PathBuf>,
}

impl TierBatch {
    /// Whether both tiers are empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.tree_sitter.is_empty() && self.heavy.is_empty()
    }
}

/// Two-tier debounced scheduler: a short window drives the cheap tree-sitter
/// tier, a longer window defers the heavy tier (FR-17).
///
/// A change is recorded into both debouncers. Because each debouncer resets its
/// path's deadline on every event, a continuous edit burst keeps deferring the
/// heavy tier while the tree-sitter tier fires shortly after the last change —
/// exactly "structure updates on save / debounced keystroke; heavier facts
/// update on save / on-demand".
#[derive(Debug)]
pub struct TieredScheduler {
    fast: Debouncer,
    heavy: Debouncer,
}

impl TieredScheduler {
    /// Create a scheduler with a `fast_window` (tree-sitter) and a `heavy_window`
    /// (deferred recompute). `heavy_window` is expected to be ≥ `fast_window`.
    #[must_use]
    pub fn new(fast_window: Duration, heavy_window: Duration) -> Self {
        Self {
            fast: Debouncer::new(fast_window),
            heavy: Debouncer::new(heavy_window),
        }
    }

    /// Build a scheduler from the `[hank.freshness]` config: `debounce_ms` is the
    /// tree-sitter window; `heavy_debounce_ms` the deferred window.
    #[must_use]
    pub fn from_config(freshness: &FreshnessConfig) -> Self {
        Self::new(
            Duration::from_millis(freshness.debounce_ms),
            Duration::from_millis(freshness.heavy_debounce_ms),
        )
    }

    /// Record a change for `path` observed at `at` into both tiers.
    pub fn record(&mut self, path: PathBuf, at: Instant) {
        self.fast.record(path.clone(), at);
        self.heavy.record(path, at);
    }

    /// Collect the paths whose windows have elapsed by `now`, per tier.
    pub fn poll(&mut self, now: Instant) -> TierBatch {
        TierBatch {
            tree_sitter: self.fast.flush_ready(now),
            heavy: self.heavy.flush_ready(now),
        }
    }

    /// The earliest instant at which either tier will next have work.
    #[must_use]
    pub fn next_deadline(&self) -> Option<Instant> {
        match (self.fast.next_deadline(), self.heavy.next_deadline()) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        }
    }

    /// Whether both tiers are idle.
    #[must_use]
    pub fn is_idle(&self) -> bool {
        self.fast.is_empty() && self.heavy.is_empty()
    }
}

/// Whether a changed path is worth re-extracting: a Rust source file that is not
/// build output or VCS metadata. (Only Rust is wired today; the `langs-extra`
/// grammars widen this later.)
#[must_use]
pub fn is_watch_relevant(path: &Path) -> bool {
    if path.extension().is_none_or(|ext| ext != "rs") {
        return false;
    }
    !path.components().any(|c| {
        let s = c.as_os_str();
        s == "target" || s == ".git"
    })
}

/// Sink for tiered re-extraction work. The watcher calls exactly one method per
/// ready tier, with the debounced batch of changed files.
pub trait TierHandler: Send {
    /// The cheap tree-sitter tier: re-parse each changed file. Fires on the
    /// short window (FR-17). `paths` is non-empty.
    fn tree_sitter(&mut self, paths: &[PathBuf]);

    /// The deferred heavy tier: recompute the affected graph/frontier (and, in
    /// later phases, LSP/CPG facts). Fires on the long window. `paths` is
    /// non-empty.
    fn heavy(&mut self, paths: &[PathBuf]);
}

/// The default handler: feed the existing extraction entrypoints.
///
/// - tree-sitter tier → [`crate::extract::extract_structure`] per changed file
///   (the FR-16 "re-parse X" step);
/// - heavy tier → rebuild [`CodeGraph`] over `root` (the stand-in for the
///   frontier-bounded overlay update until FR-16 lands).
pub struct GraphRefresh {
    root: PathBuf,
}

impl GraphRefresh {
    /// Refresh the graph rooted at `root`.
    #[must_use]
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }
}

impl TierHandler for GraphRefresh {
    fn tree_sitter(&mut self, paths: &[PathBuf]) {
        for path in paths {
            match std::fs::read_to_string(path) {
                Ok(source) => match crate::extract::extract_structure(&source, "rust") {
                    Ok(structure) => tracing::info!(
                        file = %path.display(),
                        symbols = structure.symbols.len(),
                        calls = structure.calls.len(),
                        tier = Tier::TreeSitter.as_str(),
                        "re-extracted (tree-sitter)"
                    ),
                    Err(e) => {
                        tracing::debug!(file = %path.display(), error = %e, "tree-sitter re-parse skipped");
                    }
                },
                Err(e) => {
                    // A removed file is a legitimate event; nothing to re-parse.
                    tracing::debug!(file = %path.display(), error = %e, "unreadable (removed?)");
                }
            }
        }
    }

    fn heavy(&mut self, paths: &[PathBuf]) {
        match CodeGraph::build(&self.root) {
            Ok(graph) => {
                let (nodes, edges) = graph.stats();
                tracing::info!(
                    changed = paths.len(),
                    nodes,
                    edges,
                    "graph rebuilt (heavy tier)"
                );
            }
            Err(e) => tracing::warn!(error = %e, "heavy-tier graph rebuild failed"),
        }
    }
}

/// A running file-watch. Dropping (or calling [`stop`](Self::stop)) halts the
/// watcher and joins its worker thread.
pub struct Watcher {
    _inner: RecommendedWatcher,
    running: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl Watcher {
    /// Start watching `root` recursively, dispatching debounced, tiered
    /// re-extraction to `handler`.
    ///
    /// `poll_interval` bounds how often the worker wakes to flush ready batches
    /// even absent new events (so a debounce deadline is honored promptly).
    pub fn start(
        root: &Path,
        mut scheduler: TieredScheduler,
        mut handler: Box<dyn TierHandler>,
        poll_interval: Duration,
    ) -> crate::Result<Self> {
        let (tx, rx) = mpsc::channel::<PathBuf>();

        // notify runs the callback on its own thread; forward relevant changed
        // paths into the channel and drop the rest.
        let mut inner = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
            if let Ok(event) = res {
                if !(event.kind.is_create() || event.kind.is_modify() || event.kind.is_remove()) {
                    return;
                }
                for path in event.paths {
                    if is_watch_relevant(&path) {
                        // Receiver gone → watcher shutting down; ignore.
                        let _ = tx.send(path);
                    }
                }
            }
        })
        .map_err(|e| watch_err(&e))?;
        inner
            .watch(root, RecursiveMode::Recursive)
            .map_err(|e| watch_err(&e))?;

        let running = Arc::new(AtomicBool::new(true));
        let worker_running = Arc::clone(&running);
        let worker = std::thread::spawn(move || {
            while worker_running.load(Ordering::Relaxed) {
                // Wake on the next deadline, or the poll interval if idle.
                let wait = scheduler.next_deadline().map_or(poll_interval, |d| {
                    d.saturating_duration_since(Instant::now())
                        .min(poll_interval)
                });
                match rx.recv_timeout(wait) {
                    Ok(path) => scheduler.record(path, Instant::now()),
                    Err(RecvTimeoutError::Timeout) => {}
                    Err(RecvTimeoutError::Disconnected) => break,
                }
                let batch = scheduler.poll(Instant::now());
                if !batch.tree_sitter.is_empty() {
                    handler.tree_sitter(&batch.tree_sitter);
                }
                if !batch.heavy.is_empty() {
                    handler.heavy(&batch.heavy);
                }
            }
        });

        Ok(Self {
            _inner: inner,
            running,
            worker: Some(worker),
        })
    }

    /// Stop the watcher and join its worker thread.
    pub fn stop(mut self) {
        self.shutdown();
    }

    fn shutdown(&mut self) {
        self.running.store(false, Ordering::Relaxed);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

impl Drop for Watcher {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// Map a `notify` error into the crate error type.
fn watch_err(e: &notify::Error) -> crate::Error {
    crate::Error::Config(format!("file-watch: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    fn p(s: &str) -> PathBuf {
        PathBuf::from(s)
    }

    #[test]
    fn debouncer_holds_until_window_elapses() {
        let t0 = Instant::now();
        let mut d = Debouncer::new(Duration::from_millis(300));
        d.record(p("a.rs"), t0);

        // Before the window: nothing ready.
        assert!(d.flush_ready(t0 + Duration::from_millis(299)).is_empty());
        // At/after the window: the path flushes exactly once.
        assert_eq!(
            d.flush_ready(t0 + Duration::from_millis(300)),
            vec![p("a.rs")]
        );
        assert!(d.flush_ready(t0 + Duration::from_secs(10)).is_empty());
    }

    #[test]
    fn debouncer_coalesces_a_burst() {
        let t0 = Instant::now();
        let mut d = Debouncer::new(Duration::from_millis(300));
        // A save-storm: same file several times inside the window.
        d.record(p("a.rs"), t0);
        d.record(p("a.rs"), t0 + Duration::from_millis(100));
        d.record(p("a.rs"), t0 + Duration::from_millis(250));

        // The window is measured from the *last* event, so 300ms after t0 is
        // still too early — the burst coalesced, deadline moved out.
        assert!(d.flush_ready(t0 + Duration::from_millis(300)).is_empty());
        // One flush, not three, once the tail settles.
        assert_eq!(
            d.flush_ready(t0 + Duration::from_millis(550)),
            vec![p("a.rs")]
        );
    }

    #[test]
    fn debouncer_next_deadline_tracks_earliest() {
        let t0 = Instant::now();
        let mut d = Debouncer::new(Duration::from_millis(100));
        d.record(p("late.rs"), t0 + Duration::from_millis(50));
        d.record(p("early.rs"), t0);
        assert_eq!(d.next_deadline(), Some(t0 + Duration::from_millis(100)));
    }

    #[test]
    fn tiered_scheduler_fires_fast_before_heavy() {
        let t0 = Instant::now();
        let mut s = TieredScheduler::new(Duration::from_millis(300), Duration::from_millis(1500));
        s.record(p("x.rs"), t0);

        // After the fast window: tree-sitter fires, heavy still deferred.
        let batch = s.poll(t0 + Duration::from_millis(400));
        assert_eq!(batch.tree_sitter, vec![p("x.rs")]);
        assert!(batch.heavy.is_empty());

        // After the heavy window: heavy fires (and only once).
        let batch = s.poll(t0 + Duration::from_millis(1600));
        assert!(batch.tree_sitter.is_empty());
        assert_eq!(batch.heavy, vec![p("x.rs")]);
    }

    #[test]
    fn tiered_scheduler_defers_heavy_across_a_burst() {
        let t0 = Instant::now();
        let mut s = TieredScheduler::new(Duration::from_millis(300), Duration::from_millis(1000));
        // Continuous editing every 500ms keeps pushing the heavy deadline out.
        s.record(p("x.rs"), t0);
        s.record(p("x.rs"), t0 + Duration::from_millis(500));
        s.record(p("x.rs"), t0 + Duration::from_millis(1000));

        // Fast tier already fired for the earlier edits; heavy has NOT — its
        // deadline is now t0+2000, so at t0+1200 it is still pending.
        let batch = s.poll(t0 + Duration::from_millis(1200));
        assert!(batch.heavy.is_empty(), "heavy must defer during a burst");

        // Once edits stop, heavy finally fires.
        let batch = s.poll(t0 + Duration::from_millis(2100));
        assert_eq!(batch.heavy, vec![p("x.rs")]);
    }

    #[test]
    fn relevance_filter_accepts_only_source_rust() {
        assert!(is_watch_relevant(Path::new("src/lib.rs")));
        assert!(!is_watch_relevant(Path::new("README.md")));
        assert!(!is_watch_relevant(Path::new("target/debug/build.rs")));
        assert!(!is_watch_relevant(Path::new(".git/HEAD")));
    }

    /// A recording handler for the integration test.
    #[derive(Clone, Default)]
    struct Recorder {
        tree_sitter: Arc<Mutex<Vec<PathBuf>>>,
        heavy: Arc<Mutex<Vec<PathBuf>>>,
    }

    impl TierHandler for Recorder {
        fn tree_sitter(&mut self, paths: &[PathBuf]) {
            self.tree_sitter.lock().unwrap().extend_from_slice(paths);
        }
        fn heavy(&mut self, paths: &[PathBuf]) {
            self.heavy.lock().unwrap().extend_from_slice(paths);
        }
    }

    #[test]
    fn watcher_dispatches_a_real_file_change() {
        let dir = tempfile::tempdir().unwrap();
        let rec = Recorder::default();
        let seen_heavy = Arc::clone(&rec.heavy);

        // Short windows so the test is quick but still exercises debouncing.
        let scheduler = TieredScheduler::new(Duration::from_millis(50), Duration::from_millis(120));
        let watcher = Watcher::start(
            dir.path(),
            scheduler,
            Box::new(rec.clone()),
            Duration::from_millis(20),
        )
        .unwrap();

        // Give the OS watcher a moment to arm, then create + modify a file.
        std::thread::sleep(Duration::from_millis(100));
        let file = dir.path().join("thing.rs");
        std::fs::write(&file, "fn a() {}\n").unwrap();
        std::thread::sleep(Duration::from_millis(30));
        std::fs::write(&file, "fn a() {}\nfn b() { a(); }\n").unwrap();

        // Poll for up to ~3s for the deferred heavy tier to fire — it fires after
        // the tree-sitter tier (longer window), so observing it proves the whole
        // pipeline ran. If the platform delivers no events (unsupported FS), skip
        // rather than fail — same degradation stance as the git-backed tests.
        let mut got = false;
        for _ in 0..150 {
            if !seen_heavy.lock().unwrap().is_empty() {
                got = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        watcher.stop();

        if got {
            // The cheap tier necessarily ran before the heavy one did.
            assert!(
                rec.tree_sitter.lock().unwrap().iter().any(|p| p == &file),
                "tree-sitter tier must have re-extracted the changed file"
            );
        } else {
            eprintln!("note: no filesystem events delivered; watcher dispatch not exercised");
        }
    }
}
