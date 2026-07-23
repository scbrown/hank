//! The `PreToolUse` policy guard — a blocking, capability-scoped edit check.
//!
//! This is where the §5.8 trust boundary becomes concrete: an agent's edit tool
//! call is intercepted *before* it lands, sized against the tenant's capability
//! scope (FR-25), and denied with a readable reason when it exceeds it.
//!
//! Everything here is arranged around one invariant: **fail open**. The harness
//! launches every crew agent through this hook, so a guard that fails closed
//! bricks the fleet the moment Hank is unavailable. Only a policy decision
//! blocks an edit; every error, timeout, and unknown degrades to "allow". See
//! `docs/book/src/reference/policy-guard.md` for the pinned contract.

use std::ffi::OsStr;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use super::measure::{measure_within, relative};
use super::{deny_envelope, first_notice_for_session, system_message, HookInput};
use crate::config::HankConfig;
use crate::extract::language_for_extension;
use crate::policy::{BlastRadius, Mode};
use crate::types::Freshness;

/// What the guard decided — the value the CLI turns into stdout.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// Allow the edit, silently. Exit 0, empty stdout.
    Allow,
    /// Block the edit with this model-facing reason.
    Deny(String),
    /// Allow the edit but tell the user something (advise mode, or fail-open).
    Notify(String),
}

/// Run the `pre-edit` guard: read the harness payload from stdin, decide, and
/// print at most one JSON object. Always returns `Ok` — the process must exit 0
/// so the harness never treats the guard as a fail-closed block.
pub fn run_pre_edit(tenant: Option<&str>, config_override: Option<&Path>) -> anyhow::Result<()> {
    let mut buf = String::new();
    std::io::stdin().lock().read_to_string(&mut buf).ok();
    let root = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));

    match guard(&buf, &root, tenant, config_override) {
        Outcome::Allow => {}
        Outcome::Deny(reason) => println!("{}", deny_envelope(&reason)),
        Outcome::Notify(message) => println!("{}", system_message(&message)),
    }
    Ok(())
}

/// Decide an edit, and SPOOL the decision (aegis-0nng): one `guard` metrics
/// line per invocation — result, duration, extension — through the
/// fail-silent spool, after the outcome is already fixed. Measurement rides
/// behind the decision; it can never lean on it.
#[must_use]
pub fn guard(
    input_json: &str,
    default_root: &Path,
    tenant: Option<&str>,
    config_override: Option<&Path>,
) -> Outcome {
    let started = Instant::now();
    let outcome = guard_inner(input_json, default_root, tenant, config_override);
    let result = match &outcome {
        Outcome::Allow => "allow",
        Outcome::Deny(_) => "deny",
        Outcome::Notify(_) => "notify",
    };
    let ext = HookInput::parse(input_json)
        .and_then(|i| i.tool_input.file_path)
        .and_then(|f| {
            Path::new(&f)
                .extension()
                .and_then(OsStr::to_str)
                .map(str::to_string)
        })
        .unwrap_or_default();
    // The MODE rides every guard line (soak hygiene): the enforce-flip gate is
    // "zero false positives measured over ambient ADVISE traffic", and the
    // first live window was unusable because operator test bursts under an
    // enforce config were indistinguishable from fleet lines. The mode is the
    // filter that makes the soak evidence clean.
    let mode =
        HankConfig::resolve(config_override, default_root).map_or("?", |c| match c.policy.mode {
            Mode::Off => "off",
            Mode::Advise => "advise",
            Mode::Enforce => "enforce",
        });
    crate::metrics::emit(
        "guard",
        &[
            ("result", result.into()),
            ("mode", mode.into()),
            (
                "duration_ms",
                u64::try_from(started.elapsed().as_millis())
                    .unwrap_or(u64::MAX)
                    .into(),
            ),
            ("ext", ext.into()),
        ],
    );
    outcome
}

/// The decision itself. Pure apart from reading the repo, so it is directly
/// testable without a spool in the way.
#[must_use]
fn guard_inner(
    input_json: &str,
    default_root: &Path,
    tenant: Option<&str>,
    config_override: Option<&Path>,
) -> Outcome {
    let started = Instant::now();

    // An unparseable payload is an ALLOW: the guard only speaks up about edits
    // it genuinely understands.
    let Some(input) = HookInput::parse(input_json) else {
        return Outcome::Allow;
    };
    let Some(file_path) = input.tool_input.file_path.clone() else {
        return Outcome::Allow;
    };
    let root = input.root(default_root);

    // Honour `--config` if the operator scoped the guard at a specific file. A
    // bad override path errors here and lands in `fail_open` — a loud allow,
    // never a silent revert to the ambient config the operator meant to bypass.
    let config = match HankConfig::resolve(config_override, &root) {
        Ok(config) => config,
        Err(e) => return fail_open(&input, "config", &format!("unreadable config ({e})")),
    };

    let file = PathBuf::from(&file_path);
    let rel = relative(&file, &root);

    // Structural rules (tree-sitter tier) govern the TEXT an edit introduces and
    // are NOT per-tenant: a "no ticket id in a comment" rule holds for everyone.
    // Evaluate them before the tenant-scope gate so they apply even to an
    // unconstrained tenant. A rule Deny/Notify short-circuits the scope checks —
    // one guard decision per edit.
    if let Some(outcome) = rule_check(&config, &input, &rel) {
        return outcome;
    }

    // Governed policies projected from quipu (Phase 4), evaluated where the
    // evidence is hot — BOTH planes, one registry refresh, one verdict:
    // language-independent text rules (aegis-mqnl's catalogue: the plane the
    // measured leaks needed, since they were in .md/.yml files no grammar
    // covers) and language-gated structural rules. Opt-in (quipu.enabled +
    // endpoint), and behind the `quipu` feature so a default build carries
    // none of it.
    #[cfg(feature = "quipu")]
    if let Some(outcome) = governed_check(&config, &input, &root, &rel) {
        return outcome;
    }

    // No scope for this tenant — mode is off, or the tenant is unconstrained.
    let (Some(tenant), Some(scope)) = (tenant, config.policy.scope_for(tenant)) else {
        return Outcome::Allow;
    };

    // A scope whose globs do not compile is misconfigured; say so rather than
    // quietly under-enforcing it.
    let glob_errors = scope.glob_errors();
    if !glob_errors.is_empty() {
        let detail: Vec<String> = glob_errors
            .iter()
            .map(|(pattern, why)| format!("`{pattern}` ({why})"))
            .collect();
        return fail_open(
            &input,
            "globs",
            &format!(
                "scope for tenant `{tenant}` has malformed path globs: {}",
                detail.join(", ")
            ),
        );
    }

    // 1. Path scope — cheap, no graph needed, so it runs even under a blown
    //    deadline. This is the check that must never be skipped.
    if let Some(violation) = scope.check_path(&rel, tenant) {
        return decide(config.policy.mode, violation.message);
    }

    // 2. Blast radius — expensive. Bounded by whatever remains of the budget.
    let budget = Duration::from_millis(config.policy.deadline_ms)
        .checked_sub(started.elapsed())
        .unwrap_or_default();
    if scope.max_impacted_symbols.is_none() && scope.max_impacted_files.is_none() {
        return Outcome::Allow; // Nothing to measure against.
    }

    // Size the edit against the RESIDENT daemon when one is expected and usable,
    // else the transient build (FR-31 thin-client cutover). Both paths yield the
    // same `MeasureReply`; `daemon_absent` is set when a daemon was EXPECTED but
    // could not be used, so we can be LOUD about it below.
    let (reply, daemon_absent) = blast_reply(&config, &root, &file, &rel, &input, budget);

    let radius = if reply.measured {
        BlastRadius {
            symbols: reply.symbols,
            files: reply.files,
        }
    } else {
        // NOT MEASURED. Allowing is still the contract — the guard is fail-open by
        // design and a language we cannot parse must not brick an agent's edits.
        // But allowing SILENTLY is the defect: an unparseable file and a
        // genuinely-clean edit produced identical empty stdout, so a rule that
        // could not be evaluated looked exactly like a rule that passed. Say it
        // instead. Rate-limited to once per session by the same gate the
        // fail-open notice uses, because a per-edit message would be scrolled
        // past and then ignored.
        let reason = reply
            .reason
            .clone()
            .unwrap_or_else(|| "unmeasured".to_string());
        eprintln!("hank: blast radius UNMEASURED for `{rel}`: {reason}");
        let kind = format!("unmeasured-{}-{rel}", reply.kind);
        if first_notice_for_session(input.session_id.as_deref(), &kind) {
            return Outcome::Notify(format!(
                "hank: blast-radius rules were NOT EVALUATED for `{rel}` — \
                 {reason}. The edit is allowed (the guard fails open), but \
                 tenant `{tenant}`'s ceilings did not apply to it. Treat this \
                 file as UNGUARDED by blast radius, not as within limits."
            ));
        }
        return Outcome::Allow;
    };

    let verdict = match scope.check_blast(radius, &rel, tenant) {
        Some(violation) => decide(config.policy.mode, violation.message),
        None => Outcome::Allow,
    };

    // DAEMON EXPECTED BUT DOWN — the cheapest-bypass scenario the daemon exists to
    // prevent. Always log it; and when the edit is otherwise ALLOWED, surface it to
    // the model once per session, because an allowed edit while the resident guard
    // is down is exactly the silent bypass we must not let pass quietly. A Deny wins
    // (blocking the edit is the priority; the daemon-down stays on stderr); an
    // UNMEASURED Notify already returned above. The fail-open is intact: the edit was
    // still guarded, by the transient rebuild.
    if let Some(reason) = daemon_absent {
        eprintln!("hank: resident guard daemon EXPECTED but unusable: {reason}");
        if matches!(verdict, Outcome::Allow)
            && first_notice_for_session(input.session_id.as_deref(), "daemon-absent")
        {
            return Outcome::Notify(format!(
                "hank: the resident guard daemon is DOWN ({reason}). This edit was guarded by a \
                 transient rebuild and ALLOWED — but the daemon a caller could kill to bypass \
                 the guard on every edit is not running. Restart it."
            ));
        }
    }
    verdict
}

/// Size an edit into a [`MeasureReply`], from the resident daemon when one is
/// EXPECTED and usable, else the transient build. Returns `(reply, daemon_absent)`;
/// `daemon_absent` is `Some(reason)` only when a daemon was expected but could not
/// be used — so the caller can be loud about it without ever treating "daemon down"
/// as "allow".
///
/// This is the whole cutover, and its shape is the safety contract:
/// - No daemon expected (`use_daemon = false`, the default and every case today):
///   transient build, `None`. Absence is normal and stays silent.
/// - Daemon expected and it answered: use its reply, `None`.
/// - Daemon expected and it could NOT answer (down, or serving a different repo so
///   `/measure` 400s): FALL BACK to the transient build, and return the reason so
///   the caller warns. The guard still runs — fail-open is preserved.
fn blast_reply(
    config: &HankConfig,
    root: &Path,
    file: &Path,
    rel: &str,
    input: &HookInput,
    budget: Duration,
) -> (crate::daemon::MeasureReply, Option<String>) {
    let transient = || {
        crate::daemon::MeasureReply::from_sizing(&measure_within(
            root,
            file,
            rel,
            input,
            config.policy.max_hops,
            budget,
        ))
    };

    if !config.serve.use_daemon {
        return (transient(), None);
    }

    let anchors: Vec<String> = input
        .replaced_texts()
        .into_iter()
        .map(str::to_string)
        .collect();
    match crate::daemon::client::fetch_measure(
        &config.serve.bind_address,
        config.serve.mcp_http_port,
        &file.to_string_lossy(),
        rel,
        &anchors,
        config.policy.max_hops,
        budget,
    ) {
        Ok(reply) => (reply, None),
        Err(reason) => (transient(), Some(reason)),
    }
}

/// Evaluate the structural rule set against the text this edit introduces.
///
/// Returns `Some(outcome)` when the rules had something to say — a Deny/Notify
/// for a violation under the active mode, or a loud fail-open when the rule set
/// itself does not compile — and `None` when they were clean or inapplicable, so
/// the caller falls through to the capability-scope checks.
///
/// Rules read only the text the edit ADDS (the `Write` content, or the
/// `new_string`s of an `Edit`/`MultiEdit`), so an agent is answerable for what it
/// writes, not for pre-existing debt elsewhere in the file — the same "size the
/// touched region" discipline the blast-radius path uses.
fn rule_check(config: &HankConfig, input: &HookInput, rel: &str) -> Option<Outcome> {
    // Mode::Off disarms the whole guard, rules included.
    if config.policy.mode == Mode::Off {
        return None;
    }
    let rules = &config.policy.rules;
    if rules.is_empty() {
        return None;
    }

    // A rule set that does not compile is misconfigured; fail open LOUDLY rather
    // than quietly under-enforce (the malformed-glob discipline, for rules).
    let errors = crate::rules::errors(rules);
    if !errors.is_empty() {
        let detail: Vec<String> = errors
            .iter()
            .map(|(name, why)| format!("`{name}` ({why})"))
            .collect();
        return Some(fail_open(
            input,
            "rules",
            &format!("policy rules do not compile: {}", detail.join(", ")),
        ));
    }

    // Rules are language-specific: a file this build cannot parse simply has no
    // applicable rule, so there is nothing to evaluate (not a gap to report).
    let ext = Path::new(rel).extension().and_then(OsStr::to_str)?;
    let language = language_for_extension(ext)?;
    let introduced = introduced_text(input)?;

    let violations = crate::rules::evaluate(rules, &introduced, language, rel);
    if violations.is_empty() {
        return None;
    }
    // FR-3 freshness: a rule verdict declares how current it is. Rules loaded from
    // local config are the authoritative source (not a cache), and evidence is the
    // exact proposed edit — so the verdict is genuinely FRESH. When rules instead
    // come from a projected quipu registry (Phase 4), the registry's own sync state
    // is threaded here so a verdict computed against a stale projection is tagged
    // STALE, never silently fresh.
    let message = rule_verdict_message(&violations, Freshness::Fresh);
    Some(decide(config.policy.mode, message))
}

/// Combine rule violations into one model-facing verdict and DECLARE its freshness
/// (FR-3). The freshness is the policy registry's currency: `Fresh` for the local
/// config source, or the projection's real sync state once Phase 4 wires it — never
/// a silent assumption of `fresh`.
fn rule_verdict_message(
    violations: &[crate::rules::RuleViolation],
    freshness: Freshness,
) -> String {
    let body = violations
        .iter()
        .map(|v| v.message.clone())
        .collect::<Vec<_>>()
        .join("\n");
    let note = match freshness {
        Freshness::Fresh => {
            "verdict freshness: fresh (evaluated against the exact proposed \
             edit and the loaded policy set)"
        }
        Freshness::Stale => {
            "verdict freshness: STALE — the projected policy registry could not \
             be refreshed from quipu, so this verdict may not reflect the latest governed policy"
        }
        Freshness::Recomputing => {
            "verdict freshness: recomputing — the policy registry is \
             mid-refresh"
        }
    };
    format!("{body}\n({note})")
}

/// Evaluate the governed policies projected from quipu (Phase 4) — both
/// planes, one registry refresh, ONE verdict per edit.
///
/// TEXT plane first (aegis-mqnl's `InternalIdentifierPattern` catalogue):
/// deliberately NOT extension- or language-gated — that gate is the reason the
/// first governed rule never ran, since the identifiers it exists to stop
/// leaked through `.md`/`.yml` files no grammar covers. A block-tier text rule
/// blocks only when the REPO IS PUBLIC, and publicness is asked of the graph
/// itself (the governed policy's own `/policy/check`, three-valued): a repo
/// the graph does not know gets a warning THAT SAYS SO — never a block on a
/// guess, never silence (mqnl's design constraint, verbatim).
///
/// STRUCTURAL plane second (`boundary:"action"` Selector/Predicate policies),
/// language-gated as before; a governed `deny` effect blocks under
/// [`Mode::Enforce`].
///
/// Violations from both planes merge into one message; the edit blocks if ANY
/// violation blocks under the active mode. [`Mode::Advise`] never blocks (the
/// local advise-first ceiling), and a projection that cannot be fetched fails
/// OPEN loudly — governed policy that could not be projected must be visible,
/// never a silent pass.
///
/// The verdict declares the projection's [`Freshness`] — a successful sync is
/// `Fresh`. NB: this fetches per edit; the resident/async cache that makes it
/// cheap (FR-31) is the daemon-side form of `H-PROJECTION` / the hac0 signed
/// cache, and that seam is deliberately untouched here.
#[cfg(feature = "quipu")]
fn governed_check(
    config: &HankConfig,
    input: &HookInput,
    root: &Path,
    rel: &str,
) -> Option<Outcome> {
    use crate::project::RepoExposure;

    if config.policy.mode == Mode::Off || !config.quipu.enabled || config.quipu.endpoint.is_empty()
    {
        return None;
    }
    let introduced = introduced_text(input)?;

    let mut registry = crate::project::ProjectionRegistry::new(&config.quipu.endpoint);
    if let Err(e) = registry.refresh() {
        return Some(fail_open(
            input,
            "projection",
            &format!("could not project governed policy from quipu: {e}"),
        ));
    }

    // A projected rule set that does not compile is a broken sync; fail open
    // loudly — both planes, same discipline.
    let rules: Vec<crate::rules::Rule> =
        registry.policies().iter().map(|p| p.rule.clone()).collect();
    let mut errors = crate::rules::errors(&rules);
    errors.extend(crate::textrules::errors(registry.text_rules()));
    if !errors.is_empty() {
        let detail: Vec<String> = errors
            .iter()
            .map(|(name, why)| format!("`{name}` ({why})"))
            .collect();
        return Some(fail_open(
            input,
            "projection-rules",
            &format!("projected policies do not compile: {}", detail.join(", ")),
        ));
    }

    let mut messages: Vec<String> = Vec::new();
    let mut any_blocking = false;
    let mut structural_count = 0usize;

    // --- TEXT plane: every file, no grammar required ------------------------
    let text_violations = crate::textrules::evaluate(registry.text_rules(), &introduced, rel);
    if !text_violations.is_empty() {
        // Exposure is resolved ONCE per edit, from the graph, via the governed
        // policy itself — so hank and every other consumer of rule #1 share one
        // definition of "public". Any failure to ask IS the Unknown answer.
        let exposure = match crate::git::origin_repo_name(root) {
            Some(repo) => crate::project::fetch_repo_exposure(&config.quipu.endpoint, &repo),
            None => RepoExposure::Unknown(
                "this tree has no `origin` remote, so its exposure cannot be resolved".into(),
            ),
        };
        let (text_messages, text_blocks) = text_plane(&text_violations, &exposure);
        messages.extend(text_messages);
        any_blocking |= text_blocks;
    }

    // --- STRUCTURAL plane: language-gated, as before ------------------------
    if let Some(language) = Path::new(rel)
        .extension()
        .and_then(OsStr::to_str)
        .and_then(language_for_extension)
    {
        let violations =
            crate::project::evaluate_projected(registry.policies(), &introduced, language, rel);
        structural_count = violations.len();
        for v in &violations {
            if v.blocking {
                any_blocking = true;
            }
            messages.push(v.message.clone());
        }
    }

    if messages.is_empty() {
        return None;
    }
    // The leverage headline (aegis-0nng): WHICH governed rules fire, by name.
    // Text rules carry names; structural violations ride as a count (their
    // names live only inside composed messages today).
    let rule_names: Vec<serde_json::Value> = text_violations
        .iter()
        .map(|v| serde_json::Value::from(v.rule.clone()))
        .collect();
    crate::metrics::emit(
        "governed",
        &[
            ("rules", rule_names.into()),
            (
                "mode",
                match config.policy.mode {
                    Mode::Off => "off",
                    Mode::Advise => "advise",
                    Mode::Enforce => "enforce",
                }
                .into(),
            ),
            ("structural", (structural_count as u64).into()),
            ("blocking", any_blocking.into()),
        ],
    );
    let blocks = config.policy.mode == Mode::Enforce && any_blocking;
    let message = rule_verdict_message_from(&messages.join("\n"), registry.freshness());
    if blocks {
        Some(Outcome::Deny(message))
    } else {
        Some(Outcome::Notify(format!(
            "hank (governed, not blocking): {message}"
        )))
    }
}

/// The text plane's decision, PURE: which messages, and whether anything
/// blocks, given the violations and the repo's exposure. Separated from
/// [`governed_check`] so the tier x exposure matrix — the part of this whole
/// circuit that must never be wrong in the blocking direction — is testable
/// without a network.
///
/// The matrix (mqnl's seam, verbatim):
///   block tier + Public   -> BLOCKS (this is the leak the rule exists for)
///   block tier + Internal -> warning, downgraded and saying why
///   block tier + Unknown  -> warning that SAYS the repo is unknown — never
///                            block on a guess, never be silent on ignorance
///   warn tier  + anything -> warning
#[cfg(feature = "quipu")]
fn text_plane(
    violations: &[crate::textrules::TextViolation],
    exposure: &crate::project::RepoExposure,
) -> (Vec<String>, bool) {
    use crate::project::RepoExposure;
    use crate::textrules::TextTier;

    let mut messages: Vec<String> = Vec::new();
    let mut any_blocking = false;
    for v in violations {
        if v.tier == TextTier::Block && *exposure == RepoExposure::Public {
            any_blocking = true;
        }
        messages.push(v.message.clone());
    }
    match exposure {
        RepoExposure::Public => messages.push(
            "[exposure: this repo has a PUBLIC remote (per the graph), so \
             block-tier rules block]"
                .to_string(),
        ),
        RepoExposure::Internal => messages.push(
            "[exposure: downgraded to a warning — the graph knows this repo and \
             it has no public remote, so the token leaks nowhere; the same edit \
             would BLOCK in a public repo]"
                .to_string(),
        ),
        RepoExposure::Unknown(reason) => messages.push(format!(
            "[exposure: warning, NOT blocking — the repo's exposure is unknown \
             ({reason}), and a governed rule never blocks on a guess. Add this \
             repo's `repo_<name>` entity and remote facts to quipu to pin it.]"
        )),
    }
    (messages, any_blocking)
}

/// Attach the FR-3 freshness declaration to an already-joined verdict body.
#[cfg(feature = "quipu")]
fn rule_verdict_message_from(body: &str, freshness: Freshness) -> String {
    let note = match freshness {
        Freshness::Fresh => "verdict freshness: fresh (governed policy projected from quipu)",
        Freshness::Stale => {
            "verdict freshness: STALE — the projected policy registry could not \
             be refreshed from quipu, so this verdict may not reflect the latest governed policy"
        }
        Freshness::Recomputing => {
            "verdict freshness: recomputing — the policy registry is \
             mid-refresh"
        }
    };
    format!("{body}\n({note})")
}

/// The text an edit INTRODUCES: the full `Write` content, else the `new_string`s
/// of an `Edit`/`MultiEdit` joined by newlines. `None` when the payload adds no
/// text (e.g. a pure deletion), in which case there is nothing for a rule to see.
fn introduced_text(input: &HookInput) -> Option<String> {
    if let Some(content) = &input.tool_input.content {
        return Some(content.clone());
    }
    let mut parts: Vec<&str> = Vec::new();
    if let Some(new) = input.tool_input.new_string.as_deref() {
        parts.push(new);
    }
    for edit in &input.tool_input.edits {
        if let Some(new) = edit.new_string.as_deref() {
            parts.push(new);
        }
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join("\n"))
    }
}

/// Turn a violation into an outcome according to the enforcement mode.
fn decide(mode: Mode, message: String) -> Outcome {
    match mode {
        Mode::Enforce => Outcome::Deny(message),
        // Advise: report what would have been denied, but never block.
        Mode::Advise => Outcome::Notify(format!("hank (advise, not blocking): {message}")),
        Mode::Off => Outcome::Allow,
    }
}

/// Degrade to "allow", loudly. Writes the stderr line the contract requires and,
/// once per session, a user-visible notice — because a hook's stderr is
/// surfaced only on exit `2`, so stderr alone would be silent in practice.
fn fail_open(input: &HookInput, kind: &str, reason: &str) -> Outcome {
    eprintln!("hank: policy guard failed open: {reason}");
    // The metric that separates "allowed clean" from "allowed because the
    // check could not run" — the two must never share a label (aegis-0nng).
    crate::metrics::emit("fail_open", &[("fail_kind", kind.into())]);
    if first_notice_for_session(input.session_id.as_deref(), kind) {
        return Outcome::Notify(format!(
            "hank: policy guard failed open ({reason}) — edits are UNGUARDED this session."
        ));
    }
    Outcome::Allow
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A repo where `leaf` is called from three other files.
    fn wide_repo() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("leaf.rs"), "fn leaf() {}\n").unwrap();
        for (i, name) in ["one", "two", "three"].iter().enumerate() {
            std::fs::write(
                dir.path().join(format!("caller{i}.rs")),
                format!("fn {name}() {{ leaf(); }}\n"),
            )
            .unwrap();
        }
        dir
    }

    fn write_policy(dir: &Path, body: &str) {
        let bobbin = dir.join(".bobbin");
        std::fs::create_dir_all(&bobbin).unwrap();
        std::fs::write(bobbin.join("config.toml"), body).unwrap();
    }

    /// A session id unique to this call, so the once-per-session fail-open
    /// marker (a file in the temp dir, which outlives the test process) cannot
    /// leak state between tests or between `cargo test` runs.
    fn unique_session() -> String {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        format!(
            "test-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        )
    }

    fn edit_payload(dir: &Path, file: &str, old: &str) -> String {
        serde_json::json!({
            "session_id": unique_session(),
            "cwd": dir.to_str().unwrap(),
            "tool_name": "Edit",
            "tool_input": {
                "file_path": dir.join(file).to_str().unwrap(),
                "old_string": old,
                "new_string": "fn leaf() { changed(); }",
            },
        })
        .to_string()
    }

    #[test]
    fn allows_when_no_policy_is_configured() {
        let dir = wide_repo();
        let payload = edit_payload(dir.path(), "leaf.rs", "fn leaf() {}");
        assert_eq!(guard(&payload, dir.path(), Some("t"), None), Outcome::Allow);
    }

    #[test]
    fn allows_when_mode_is_off_despite_a_scope() {
        let dir = wide_repo();
        write_policy(
            dir.path(),
            "[hank.policy]\nmode = \"off\"\n[hank.policy.scopes.t]\nmax_impacted_symbols = 0\n",
        );
        let payload = edit_payload(dir.path(), "leaf.rs", "fn leaf() {}");
        assert_eq!(guard(&payload, dir.path(), Some("t"), None), Outcome::Allow);
    }

    #[test]
    fn denies_an_out_of_scope_path() {
        let dir = wide_repo();
        write_policy(
            dir.path(),
            "[hank.policy]\nmode = \"enforce\"\n\
             [hank.policy.scopes.t]\nallow_paths = [\"caller*.rs\"]\n",
        );
        let payload = edit_payload(dir.path(), "leaf.rs", "fn leaf() {}");
        let Outcome::Deny(reason) = guard(&payload, dir.path(), Some("t"), None) else {
            panic!("expected a deny");
        };
        assert!(reason.contains("leaf.rs"));
        assert!(reason.contains("outside the writable capability scope"));
    }

    #[test]
    fn denies_an_edit_that_exceeds_the_blast_radius() {
        let dir = wide_repo();
        write_policy(
            dir.path(),
            "[hank.policy]\nmode = \"enforce\"\ndeadline_ms = 30000\n\
             [hank.policy.scopes.t]\nmax_impacted_files = 1\n",
        );
        let payload = edit_payload(dir.path(), "leaf.rs", "fn leaf() {}");
        let Outcome::Deny(reason) = guard(&payload, dir.path(), Some("t"), None) else {
            panic!("expected a deny");
        };
        // leaf is called from three files; the ceiling is one.
        assert!(reason.contains("3 files (ceiling 1)"), "got: {reason}");
    }

    #[test]
    fn allows_an_edit_within_the_blast_radius() {
        let dir = wide_repo();
        write_policy(
            dir.path(),
            "[hank.policy]\nmode = \"enforce\"\ndeadline_ms = 30000\n\
             [hank.policy.scopes.t]\nmax_impacted_files = 10\n",
        );
        let payload = edit_payload(dir.path(), "leaf.rs", "fn leaf() {}");
        assert_eq!(guard(&payload, dir.path(), Some("t"), None), Outcome::Allow);
    }

    #[test]
    fn advise_mode_never_denies() {
        let dir = wide_repo();
        write_policy(
            dir.path(),
            "[hank.policy]\nmode = \"advise\"\ndeadline_ms = 30000\n\
             [hank.policy.scopes.t]\nmax_impacted_files = 1\n",
        );
        let payload = edit_payload(dir.path(), "leaf.rs", "fn leaf() {}");
        let Outcome::Notify(message) = guard(&payload, dir.path(), Some("t"), None) else {
            panic!("expected an advisory, not a block");
        };
        assert!(message.contains("not blocking"));
    }

    #[test]
    fn an_untouched_tenant_is_unconstrained() {
        let dir = wide_repo();
        write_policy(
            dir.path(),
            "[hank.policy]\nmode = \"enforce\"\n\
             [hank.policy.scopes.other]\nallow_paths = [\"nothing/**\"]\n",
        );
        let payload = edit_payload(dir.path(), "leaf.rs", "fn leaf() {}");
        assert_eq!(guard(&payload, dir.path(), Some("t"), None), Outcome::Allow);
        // ...and so is an agent with no tenant at all.
        assert_eq!(guard(&payload, dir.path(), None, None), Outcome::Allow);
    }

    #[test]
    fn a_blown_deadline_allows_the_edit_and_says_so() {
        let dir = wide_repo();
        write_policy(
            dir.path(),
            "[hank.policy]\nmode = \"enforce\"\ndeadline_ms = 0\n\
             [hank.policy.scopes.t]\nmax_impacted_files = 1\n",
        );
        let payload = edit_payload(dir.path(), "leaf.rs", "fn leaf() {}");
        // The same edit is denied with a real budget (see the test above), so
        // this proves the deadline — not the policy — is what let it through.
        // CONTRACT CHANGE: it still ALLOWS (fail-open is deliberate), but it no
        // longer allows in SILENCE. This test previously asserted Outcome::Allow,
        // which is the same value a clean edit produces — so the suite could not
        // tell "we did not look" from "we looked and it was fine", and neither
        // could an operator.
        match guard(&payload, dir.path(), Some("t"), None) {
            Outcome::Notify(message) => {
                assert!(message.contains("NOT EVALUATED"), "{message}");
                assert!(message.contains("deadline_ms"), "{message}");
            }
            other => panic!("a blown deadline must be reported, got {other:?}"),
        }
    }

    #[test]
    fn a_path_check_still_applies_under_a_zero_deadline() {
        let dir = wide_repo();
        write_policy(
            dir.path(),
            "[hank.policy]\nmode = \"enforce\"\ndeadline_ms = 0\n\
             [hank.policy.scopes.t]\nallow_paths = [\"caller*.rs\"]\n",
        );
        let payload = edit_payload(dir.path(), "leaf.rs", "fn leaf() {}");
        assert!(matches!(
            guard(&payload, dir.path(), Some("t"), None),
            Outcome::Deny(_)
        ));
    }

    #[test]
    fn garbage_and_unknown_payloads_allow() {
        let dir = wide_repo();
        assert_eq!(
            guard("not json", dir.path(), Some("t"), None),
            Outcome::Allow
        );
        let no_file = serde_json::json!({ "tool_input": {} }).to_string();
        assert_eq!(guard(&no_file, dir.path(), Some("t"), None), Outcome::Allow);
    }

    #[test]
    fn a_malformed_glob_fails_open_loudly() {
        let dir = wide_repo();
        write_policy(
            dir.path(),
            "[hank.policy]\nmode = \"enforce\"\n\
             [hank.policy.scopes.t]\nallow_paths = [\"src/[\"]\n",
        );
        let session = unique_session();
        let payload = serde_json::json!({
            "session_id": session,
            "cwd": dir.path().to_str().unwrap(),
            "tool_name": "Edit",
            "tool_input": {
                "file_path": dir.path().join("leaf.rs").to_str().unwrap(),
                "old_string": "fn leaf() {}",
            },
        })
        .to_string();

        let Outcome::Notify(message) = guard(&payload, dir.path(), Some("t"), None) else {
            panic!("expected a fail-open notice, not a block");
        };
        assert!(message.contains("UNGUARDED"));
        assert!(message.contains("malformed path globs"));

        // Second edit in the same session: still allowed, but no longer noisy.
        assert_eq!(guard(&payload, dir.path(), Some("t"), None), Outcome::Allow);
        let _ = std::fs::remove_file(
            std::env::temp_dir().join(format!("hank-guard-failopen-{session}")),
        );
    }

    #[test]
    fn two_different_gaps_in_one_session_both_notify() {
        // Regression (aegis-nz2x): the once-per-session notice was keyed on the
        // session alone, so the FIRST fail-open of any kind consumed the marker and
        // every later, DIFFERENT gap in that session went silent. A config error in
        // one repo would mute a blown blast-radius deadline in another. Demonstrated
        // on the shipped binary before this fix: edit 1 (bad config) notified, edit 2
        // (blown deadline) emitted nothing at all.
        let session = unique_session();
        let mkpayload = |dir: &Path, file: &str| {
            serde_json::json!({
                "session_id": session,
                "cwd": dir.to_str().unwrap(),
                "tool_name": "Edit",
                "tool_input": {
                    "file_path": dir.join(file).to_str().unwrap(),
                    "old_string": "fn leaf() {}",
                    "new_string": "fn leaf() { x(); }",
                },
            })
            .to_string()
        };

        // Gap 1: an unreadable config.
        let a = tempfile::tempdir().unwrap();
        std::fs::write(a.path().join("leaf.rs"), "fn leaf() {}\n").unwrap();
        write_policy(a.path(), "this is not [[[ valid toml");
        let Outcome::Notify(m1) = guard(&mkpayload(a.path(), "leaf.rs"), a.path(), Some("t"), None)
        else {
            panic!("gap 1 (config) must notify");
        };
        assert!(m1.contains("failed open"), "{m1}");

        // Gap 2: SAME session, a blown blast-radius deadline — a different kind.
        let b = wide_repo();
        write_policy(
            b.path(),
            "[hank.policy]\nmode = \"enforce\"\ndeadline_ms = 0\n\
             [hank.policy.scopes.t]\nmax_impacted_files = 1\n",
        );
        match guard(&mkpayload(b.path(), "leaf.rs"), b.path(), Some("t"), None) {
            Outcome::Notify(m2) => assert!(m2.contains("NOT EVALUATED"), "{m2}"),
            other => {
                panic!("gap 2 (deadline) was SILENCED by gap 1's marker — the exact bug: {other:?}")
            }
        }

        for kind in ["config", "unmeasured-deadline-leaf.rs"] {
            let _ = std::fs::remove_file(
                std::env::temp_dir().join(format!("hank-guard-failopen-{session}-{kind}")),
            );
        }
    }

    #[test]
    fn an_unparseable_language_is_reported_unmeasured_not_silently_allowed() {
        let dir = wide_repo();
        std::fs::write(dir.path().join("notes.md"), "# hi\n").unwrap();
        write_policy(
            dir.path(),
            "[hank.policy]\nmode = \"enforce\"\ndeadline_ms = 30000\n\
             [hank.policy.scopes.t]\nmax_impacted_files = 0\n",
        );
        let payload = edit_payload(dir.path(), "notes.md", "# hi");
        // A zero ceiling denies anything measurable, so an Allow here means the
        // rule did not apply. Declining to measure is still right — but the
        // decline must be VISIBLE, or a rule that cannot be evaluated is
        // indistinguishable from one that passed. That is the whole bug: a fleet
        // was days from turning blocking on over ceilings that silently did not
        // apply to .py/.ts/.go.
        match guard(&payload, dir.path(), Some("t"), None) {
            Outcome::Notify(message) => {
                assert!(message.contains("NOT EVALUATED"), "{message}");
                assert!(message.contains("no grammar for `.md`"), "{message}");
                assert!(message.contains("UNGUARDED"), "{message}");
            }
            other => panic!("an unparseable language must be reported, got {other:?}"),
        }
    }

    /// THE regression test, at the guard level: a ceiling that denies a Rust edit
    /// must deny the identical edit in Python and TypeScript. Measured on the
    /// shipped v0.2.0 binary, both ALLOWED with empty stdout.
    #[cfg(feature = "langs-extra")]
    #[test]
    fn a_ceiling_that_denies_rust_denies_python_and_typescript_too() {
        let cases: &[(&str, &str, &str, &str)] = &[
            (
                "py",
                "def leaf():\n    return 1\n",
                "from leaf import leaf\ndef one():\n    return leaf()\n",
                "def leaf():",
            ),
            (
                "ts",
                "export function leaf(): number { return 1; }\n",
                "import { leaf } from \"./leaf\";\nexport function one() { return leaf(); }\n",
                "export function leaf(): number { return 1; }",
            ),
        ];
        for (ext, leaf_src, caller_src, anchor) in cases {
            let dir = tempfile::tempdir().unwrap();
            std::fs::write(dir.path().join(format!("leaf.{ext}")), leaf_src).unwrap();
            std::fs::write(dir.path().join(format!("one.{ext}")), caller_src).unwrap();
            write_policy(
                dir.path(),
                "[hank.policy]\nmode = \"enforce\"\ndeadline_ms = 30000\n\
                 [hank.policy.scopes.t]\nmax_impacted_files = 0\n",
            );
            let payload = edit_payload(dir.path(), &format!("leaf.{ext}"), anchor);
            match guard(&payload, dir.path(), Some("t"), None) {
                Outcome::Deny(message) => assert!(
                    message.contains("reaches"),
                    ".{ext}: denied, but not for reach: {message}"
                ),
                other => panic!(
                    ".{ext}: a zero ceiling did NOT deny an edit reaching another \
                     file — got {other:?}. The rule silently does not apply."
                ),
            }
        }
    }

    /// The load-bearing test for aegis-ll3p: a scope supplied ONLY via
    /// `--config` must actually govern the edit. The ambient config allows
    /// (no policy), so a deny here can only come from the override being read.
    #[test]
    fn a_config_override_scopes_the_guard() {
        let dir = wide_repo(); // no `.bobbin/config.toml` — ambient allows everything
        let scope_file = dir.path().join("elsewhere").join("scope.toml");
        std::fs::create_dir_all(scope_file.parent().unwrap()).unwrap();
        std::fs::write(
            &scope_file,
            "[hank.policy]\nmode = \"enforce\"\n\
             [hank.policy.scopes.t]\nallow_paths = [\"src/**\"]\n",
        )
        .unwrap();
        let payload = edit_payload(dir.path(), "leaf.rs", "fn leaf() {}");

        // Negative control: without the override, the ambient (absent) config
        // allows the edit.
        assert_eq!(guard(&payload, dir.path(), Some("t"), None), Outcome::Allow);

        // With the override, `leaf.rs` is outside `src/**` and is denied.
        let Outcome::Deny(reason) = guard(&payload, dir.path(), Some("t"), Some(&scope_file))
        else {
            panic!("the --config scope must govern the edit");
        };
        assert!(reason.contains("leaf.rs"));
        assert!(reason.contains("outside the writable capability scope"));
    }

    /// A `--config` path that does not exist must fail OPEN loudly, never
    /// silently revert to the ambient config — reverting is the disarm this
    /// override exists to prevent. Fail-open (allow) is still correct for a
    /// guard, but it must be the loud, once-per-session kind.
    #[test]
    fn a_missing_config_override_fails_open_loudly() {
        let dir = wide_repo();
        // An ambient policy that WOULD deny, to prove the fail-open is the
        // override erroring — not the ambient config quietly taking over.
        write_policy(
            dir.path(),
            "[hank.policy]\nmode = \"enforce\"\n\
             [hank.policy.scopes.t]\nallow_paths = [\"src/**\"]\n",
        );
        let missing = dir.path().join("does-not-exist.toml");
        let payload = edit_payload(dir.path(), "leaf.rs", "fn leaf() {}");

        let Outcome::Notify(message) = guard(&payload, dir.path(), Some("t"), Some(&missing))
        else {
            panic!("a bad --config must fail open loudly, not deny and not silently revert");
        };
        assert!(message.contains("UNGUARDED"));
        assert!(message.contains("does not exist"));
    }

    // --- Structural rules (tree-sitter tier) at the guard level -----------------

    /// A `[[hank.policy.rules]]` set banning ticket ids in comments. TOML literal
    /// (single-quote) strings keep the regex and query free of escape doubling.
    const NO_TICKET_RULE: &str = "[hank.policy]\nmode = \"enforce\"\n\n\
         [[hank.policy.rules]]\nname = \"no-ticket-in-comment\"\nlanguage = \"rust\"\n\
         query = '(line_comment) @c'\nmatch_type = \"must-not-match\"\n\
         pattern = '\\b[A-Z]+-[0-9]+\\b'\n";

    fn rule_edit_payload(dir: &Path, new_string: &str) -> String {
        serde_json::json!({
            "session_id": unique_session(),
            "cwd": dir.to_str().unwrap(),
            "tool_name": "Edit",
            "tool_input": {
                "file_path": dir.join("leaf.rs").to_str().unwrap(),
                "old_string": "fn leaf() {}",
                "new_string": new_string,
            },
        })
        .to_string()
    }

    #[test]
    fn a_rule_denies_an_edit_introducing_a_forbidden_comment() {
        let dir = wide_repo();
        write_policy(dir.path(), NO_TICKET_RULE);
        let payload = rule_edit_payload(dir.path(), "fn leaf() {} // see ABC-123");
        let Outcome::Deny(reason) = guard(&payload, dir.path(), Some("t"), None) else {
            panic!("a forbidden comment must be denied");
        };
        assert!(reason.contains("no-ticket-in-comment"), "{reason}");
        assert!(reason.contains("ABC-123"), "{reason}");
        // Honest about provenance AND freshness (FR-3): a local-config verdict is
        // computed against the exact proposed edit, so it declares itself fresh —
        // it never silently omits or fakes the tag.
        assert!(reason.contains("treesitter tier"), "{reason}");
        assert!(reason.contains("verdict freshness: fresh"), "{reason}");
    }

    #[test]
    fn a_rule_allows_a_clean_edit() {
        let dir = wide_repo();
        write_policy(dir.path(), NO_TICKET_RULE);
        let payload = rule_edit_payload(dir.path(), "fn leaf() {} // no ticket here");
        assert_eq!(guard(&payload, dir.path(), Some("t"), None), Outcome::Allow);
    }

    #[test]
    fn a_rule_applies_even_without_a_tenant_scope() {
        // Rules are global: no scopes table and no tenant, still enforced. This is
        // the whole reason they run BEFORE the tenant-scope gate.
        let dir = wide_repo();
        write_policy(dir.path(), NO_TICKET_RULE);
        let payload = rule_edit_payload(dir.path(), "fn leaf() {} // ABC-123");
        assert!(matches!(
            guard(&payload, dir.path(), None, None),
            Outcome::Deny(_)
        ));
    }

    #[test]
    fn advise_mode_reports_a_rule_without_blocking() {
        let dir = wide_repo();
        write_policy(dir.path(), &NO_TICKET_RULE.replace("enforce", "advise"));
        let payload = rule_edit_payload(dir.path(), "fn leaf() {} // ABC-123");
        let Outcome::Notify(msg) = guard(&payload, dir.path(), Some("t"), None) else {
            panic!("advise must not block");
        };
        assert!(msg.contains("not blocking"), "{msg}");
    }

    #[test]
    fn a_pre_existing_comment_is_not_re_flagged_only_the_introduced_text_is() {
        // The edit introduces a CLEAN comment; a bad comment already on disk is not
        // the agent's doing this turn, so the rule (which reads only introduced
        // text) allows.
        let dir = wide_repo();
        std::fs::write(
            dir.path().join("leaf.rs"),
            "// legacy XYZ-999\nfn leaf() {}\n",
        )
        .unwrap();
        write_policy(dir.path(), NO_TICKET_RULE);
        let payload = rule_edit_payload(dir.path(), "fn leaf() {} // clean now");
        assert_eq!(guard(&payload, dir.path(), Some("t"), None), Outcome::Allow);
    }

    #[test]
    fn a_malformed_rule_fails_open_loudly() {
        let dir = wide_repo();
        let bad = "[hank.policy]\nmode = \"enforce\"\n\n\
             [[hank.policy.rules]]\nname = \"broken\"\nlanguage = \"rust\"\n\
             query = '(nonexistent_node) @x'\nmatch_type = \"must-not-match\"\npattern = 'x'\n";
        write_policy(dir.path(), bad);
        let payload = rule_edit_payload(dir.path(), "fn leaf() {} // whatever");
        let Outcome::Notify(msg) = guard(&payload, dir.path(), Some("t"), None) else {
            panic!("a malformed rule must fail open loudly, not block and not pass silently");
        };
        assert!(msg.contains("UNGUARDED"), "{msg}");
        assert!(msg.contains("do not compile"), "{msg}");
    }

    /// Projection is opt-in and behind the `quipu` feature: when quipu cannot be
    /// reached, governed policy that could not be projected must be VISIBLE — a
    /// loud fail-open, never a silent pass. (An unreachable port stands in for a
    /// down quipu without a live server.)
    #[cfg(feature = "quipu")]
    #[test]
    fn an_unreachable_quipu_projection_fails_open_loudly() {
        let dir = wide_repo();
        write_policy(
            dir.path(),
            "[hank.policy]\nmode = \"enforce\"\n\
             [hank.quipu]\nenabled = true\nendpoint = \"http://127.0.0.1:1\"\n",
        );
        let payload = rule_edit_payload(dir.path(), "fn leaf() {} // whatever");
        let Outcome::Notify(msg) = guard(&payload, dir.path(), Some("t"), None) else {
            panic!("an unreachable quipu projection must fail open loudly, not pass silently");
        };
        assert!(msg.contains("UNGUARDED"), "{msg}");
        assert!(msg.contains("could not project governed policy"), "{msg}");
    }

    // --- FR-31 thin-client cutover: daemon expected vs. absent (aegis-1qze) ------
    //
    // Four quadrants of (daemon expected?) x (daemon reachable?). A closed port
    // (mcp_http_port = 1, never listening) stands in for "down" without a tokio
    // server. The "up and used" quadrant is covered at the client level by the
    // fetch_measure test against a live daemon in `daemon::http`.

    const ALLOWS: &str = "[hank.policy.scopes.t]\nmax_impacted_files = 50\n";
    const FORBIDS: &str = "[hank.policy.scopes.t]\nmax_impacted_files = 0\n";

    fn policy_with_daemon(scope: &str, use_daemon: bool) -> String {
        format!(
            "[hank.policy]\nmode = \"enforce\"\ndeadline_ms = 30000\n{scope}\n\
             [hank.serve]\nuse_daemon = {use_daemon}\nbind_address = \"127.0.0.1\"\n\
             mcp_http_port = 1\n"
        )
    }

    #[test]
    fn no_daemon_expected_is_silent_transient_the_normal_case() {
        // use_daemon = false (the default, and every case today). The guard builds
        // transiently and says NOTHING about a daemon — absence is normal.
        let dir = wide_repo();
        write_policy(dir.path(), &policy_with_daemon(ALLOWS, false));
        let payload = edit_payload(dir.path(), "leaf.rs", "fn leaf() {}");
        assert_eq!(
            guard(&payload, dir.path(), Some("t"), None),
            Outcome::Allow,
            "no daemon expected: transient build, allowed, silent"
        );
    }

    #[test]
    fn daemon_EXPECTED_but_DOWN_falls_back_and_is_LOUD_when_allowed() {
        // The cheapest-bypass case. A daemon is expected (use_daemon = true) but the
        // port is closed. The guard must STILL run (fail-open, via transient) and,
        // because the edit is allowed, say LOUDLY that the resident guard is down.
        let dir = wide_repo();
        write_policy(dir.path(), &policy_with_daemon(ALLOWS, true));
        let payload = edit_payload(dir.path(), "leaf.rs", "fn leaf() {}");
        let Outcome::Notify(msg) = guard(&payload, dir.path(), Some("t"), None) else {
            panic!("an allowed edit while the expected daemon is down must be LOUD, not silent");
        };
        assert!(msg.contains("daemon is DOWN"), "{msg}");
    }

    #[test]
    fn daemon_EXPECTED_but_DOWN_still_DENIES_a_violation_block_wins() {
        // Fail-open does not mean fail-quiet on a real violation: a down daemon must
        // not turn a Deny into an allow. The transient fallback still enforces, and a
        // Deny wins over the daemon-absent notice.
        let dir = wide_repo(); // leaf is called from three files
        write_policy(dir.path(), &policy_with_daemon(FORBIDS, true));
        let payload = edit_payload(dir.path(), "leaf.rs", "fn leaf() {}");
        assert!(
            matches!(
                guard(&payload, dir.path(), Some("t"), None),
                Outcome::Deny(_)
            ),
            "a violation must still be denied even with the expected daemon down"
        );
    }

    #[test]
    fn daemon_EXPECTED_but_DOWN_keeps_the_UNMEASURED_contract() {
        // An unparseable file is still UNMEASURED (not a silent zero), and that notice
        // takes precedence over the daemon-absent one — the transient fallback reports
        // it exactly as it would with no daemon configured.
        let dir = wide_repo();
        std::fs::write(dir.path().join("notes.md"), "# hi\n").unwrap();
        write_policy(dir.path(), &policy_with_daemon(ALLOWS, true));
        let payload = edit_payload(dir.path(), "notes.md", "# hi");
        let Outcome::Notify(msg) = guard(&payload, dir.path(), Some("t"), None) else {
            panic!("an unmeasurable file must stay UNMEASURED-loud");
        };
        assert!(msg.contains("NOT EVALUATED"), "{msg}");
    }
    // --- the governed TEXT plane's blocking matrix (aegis-m9ln / aegis-mqnl) ---
    //
    // The pure half of governed_check: which tier x exposure combinations
    // BLOCK. This is the part of the circuit that must never be wrong in the
    // blocking direction, so it is tested without a network. The projection
    // and evaluation halves have their own suites (project.rs, textrules.rs).

    #[cfg(feature = "quipu")]
    fn text_violation(tier: crate::textrules::TextTier) -> crate::textrules::TextViolation {
        crate::textrules::TextViolation {
            rule: "pattern_internal-lan-host".into(),
            tier,
            message: "governed text rule `pattern_internal-lan-host`: the edit \
                      introduces `dolt.lan` (hostname) — internal .lan hostname."
                .into(),
        }
    }

    #[cfg(feature = "quipu")]
    #[test]
    fn a_block_tier_hit_in_a_public_repo_blocks() {
        use crate::project::RepoExposure;
        let (messages, blocks) = text_plane(
            &[text_violation(crate::textrules::TextTier::Block)],
            &RepoExposure::Public,
        );
        assert!(blocks, "this is the exact leak the rule exists to stop");
        assert!(messages.iter().any(|m| m.contains("PUBLIC remote")));
    }

    #[cfg(feature = "quipu")]
    #[test]
    fn a_block_tier_hit_in_an_internal_repo_downgrades_and_says_why() {
        use crate::project::RepoExposure;
        let (messages, blocks) = text_plane(
            &[text_violation(crate::textrules::TextTier::Block)],
            &RepoExposure::Internal,
        );
        assert!(!blocks, "internal-only exposure must not block");
        assert!(messages.iter().any(|m| m.contains("downgraded")));
        assert!(messages
            .iter()
            .any(|m| m.contains("would BLOCK in a public repo")));
    }

    #[cfg(feature = "quipu")]
    #[test]
    fn an_unknown_repo_never_blocks_and_says_it_is_unknown() {
        // mqnl's constraint verbatim: warn AND SAY SO — never block on a guess.
        use crate::project::RepoExposure;
        let (messages, blocks) = text_plane(
            &[text_violation(crate::textrules::TextTier::Block)],
            &RepoExposure::Unknown("repo `hank` is not in the graph".into()),
        );
        assert!(!blocks);
        assert!(messages
            .iter()
            .any(|m| m.contains("never blocks on a guess")));
        assert!(messages.iter().any(|m| m.contains("not in the graph")));
        // The remedy is named: exposure is DATA, so the fix is a graph write.
        assert!(messages.iter().any(|m| m.contains("repo_<name>")));
    }

    #[cfg(feature = "quipu")]
    #[test]
    fn a_warn_tier_hit_never_blocks_even_in_a_public_repo() {
        use crate::project::RepoExposure;
        let (_, blocks) = text_plane(
            &[text_violation(crate::textrules::TextTier::Warn)],
            &RepoExposure::Public,
        );
        assert!(
            !blocks,
            "warn tier is advisory everywhere — per-pattern tier is data"
        );
    }
}
