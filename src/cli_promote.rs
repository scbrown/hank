//! `hank promote` — the Phase-4 Quipu promotion command, split out of `cli` for
//! size (hank #83). See `cli_analyze` for why this is a child module.
//!
//! Two `cfg` forms of one method: the real path under `quipu`, and a stub that
//! prints the phase notice without it.

use super::*;

impl Cli {
    /// Promote a tree's structural facts into Quipu: emit Turtle, SHACL-validate it
    /// in-process, and write it iff it conforms (FR-19/20/21).
    #[cfg(feature = "quipu")]
    #[allow(clippy::unused_self)] // method form for call-site symmetry with the stub arm
    pub(super) fn promote(
        &self,
        path: &Path,
        commit: &str,
        to: Option<&str>,
        repo: Option<&str>,
    ) -> anyhow::Result<()> {
        // `--to` overrides config for a one-off; otherwise the deployment's
        // configured endpoint. Empty on both is a refusal, never a guessed graph.
        let cfg = self.load_config(path)?;
        let endpoint = match to {
            Some(t) => t.to_string(),
            None if !cfg.quipu.endpoint.is_empty() => cfg.quipu.endpoint.clone(),
            None => anyhow::bail!(
                "no Quipu endpoint: pass --to <url> or set [hank.quipu] endpoint in config. \
                 Refusing rather than guessing a graph to write into."
            ),
        };
        let endpoint = endpoint.as_str();
        // Repo identity is DATA IDENTITY: it is a segment of every promoted IRI.
        // Explicit --repo wins; otherwise the origin remote names the repository.
        // With neither, REFUSE — the old fallback (directory basename) minted
        // `code/<worktree-dir>/…` islands from agent worktrees and CI workspaces,
        // structurally fragmenting one repo into unmergeable parallel graphs.
        let repo = match repo {
            Some(r) => r.to_string(),
            None => crate::git::origin_repo_name(path).ok_or_else(|| {
                anyhow::anyhow!(
                    "cannot determine repository identity: no `origin` remote at {}. \
                     Pass --repo <name>. Refusing rather than deriving identity from \
                     the directory name, which fragments the graph.",
                    path.display()
                )
            })?,
        };
        // Promotion reads the COMMITTED tree at `commit` (FR-22): uncommitted
        // working-tree churn — an in-flight overlay edit, an unsaved buffer —
        // must never reach Quipu. Checked here, after the config preconditions
        // (endpoint/identity) and just before the read, so it refuses a phantom
        // ref rather than promoting against one.
        if crate::git::resolve_commit(path, commit).is_none() {
            anyhow::bail!(
                "cannot promote `{commit}`: it does not resolve to a commit at {}. \
                 Refusing rather than promoting a phantom ref.",
                path.display()
            );
        }
        let turtle = crate::export::to_turtle_at(path, &repo, commit)?;
        // A promotion that extracted NOTHING is not a promotion — it is a green
        // empty write (measured: a Python repo "promoted: 0 triples" with a
        // SUCCESS exit while analyze saw 1647 symbols, and the scheduler wrote
        // its done-marker over the void). Say so and refuse (exit 2, the
        // could-not-promote code), so a marker-disciplined caller retries
        // instead of booking emptiness as done.
        if !turtle.contains("bobbin:CodeModule") {
            eprintln!(
                "hank promote: extracted NOTHING from {} — no parseable source                  files under this tree for the grammars in this build. Refusing                  to promote an empty graph as success. (Is the language behind                  the `langs-extra` feature? Is the path right?)",
                path.display()
            );
            std::process::exit(2);
        }
        // Provenance carries the RESOLVED commit SHA, not the ref spelling —
        // `--commit main` and `--commit <sha>` promoting the same tree record
        // the same source (partial FR-21; full bitemporal fields are a #15
        // follow-up).
        let resolved =
            crate::git::resolve_commit(path, commit).unwrap_or_else(|| commit.to_string());
        let source = format!("hank promote {repo}@{resolved} (cli)");
        let outcome = crate::promote::promote(endpoint, &turtle, &source)?;
        let mut out = std::io::stdout();
        let wrote = outcome.report(&mut out)?;
        // A refusal is a could-not-promote, not a success: exit non-zero so a script
        // cannot read a rejected promotion as a landed one.
        if !wrote {
            std::process::exit(2);
        }
        Ok(())
    }

    /// Without the `quipu` feature, promotion is unbuilt — say so honestly,
    /// and exit non-zero: the same rule as the feature arm's refusal path. A
    /// stub that exits 0 reads as a landed promotion to any script — measured
    /// 2026-07-23 (aegis-ucoh): the quipu-ingest cron ran a feature-less
    /// binary, took exit 0 as success, and advanced its promote marker past a
    /// commit that was never promoted.
    #[cfg(not(feature = "quipu"))]
    #[allow(clippy::unused_self)] // method form for call-site symmetry with the quipu arm
    pub(super) fn promote(
        &self,
        _path: &Path,
        _commit: &str,
        _to: Option<&str>,
        _repo: Option<&str>,
    ) -> anyhow::Result<()> {
        self.planned(
            "promote",
            4,
            "Quipu promotion needs `--features quipu` (this binary was built without it)",
        );
        std::process::exit(2);
    }
}
