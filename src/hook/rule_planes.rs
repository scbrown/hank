//! The guard's two RULE planes, lifted out of `pre_edit` for size (hank #83).
//!
//! Both answer the same question — "does the text this edit introduces break a
//! rule?" — and both return a [`Decision`], so the guard applies one verdict per
//! edit whichever plane produced it:
//!
//! - **local** ([`rule_check`]) — the `[[hank.policy.rules]]` set from config,
//!   language-gated, evaluated against the added text. Not per-tenant: a rule
//!   like "no ticket id in a comment" governs the code, not who wrote it.
//! - **governed** ([`governed_check`], `quipu` feature) — the same shape, but
//!   projected from quipu's policy registry, spanning a text plane (no grammar
//!   needed) and a language-gated structural plane.
//!
//! A child module of `pre_edit` rather than a sibling: it reads that module's
//! private `Decision`, `decide`, `fail_open` and `introduced_text` through
//! `use super::*`, so lifting it out changed no visibility beyond the two entry
//! points below.

use super::*;

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
pub(super) fn rule_check(config: &HankConfig, input: &HookInput, rel: &str) -> Option<Decision> {
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
        return Some(
            fail_open(
                input,
                "rules",
                &format!("policy rules do not compile: {}", detail.join(", ")),
            )
            .into(),
        );
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
    // Name every rule that fired, not just the first: an edit can break several
    // at once, and a record that dropped the rest would send an operator to fix
    // one and hit the next.
    let fired = fired_rule_ids(violations.iter().map(|v| v.rule.as_str()));
    Some(Decision::ruled(decide(config.policy.mode, message), fired))
}

/// Join the ids of the rules that fired into one stable, deduplicated field
/// value. Sorted so the same set of rules always renders the same string —
/// a record an operator groups by must not vary with evaluation order.
fn fired_rule_ids<'a>(ids: impl Iterator<Item = &'a str>) -> String {
    let mut seen: Vec<&str> = ids.collect();
    seen.sort_unstable();
    seen.dedup();
    seen.join("+")
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
pub(super) fn governed_check(
    config: &HankConfig,
    input: &HookInput,
    root: &Path,
    rel: &str,
) -> Option<Decision> {
    use crate::project::RepoExposure;

    if config.policy.mode == Mode::Off || !config.quipu.enabled || config.quipu.endpoint.is_empty()
    {
        return None;
    }
    let introduced = introduced_text(input)?;

    let mut registry = crate::project::ProjectionRegistry::new(&config.quipu.endpoint);
    if let Err(e) = registry.refresh() {
        return Some(
            fail_open(
                input,
                "projection",
                &format!("could not project governed policy from quipu: {e}"),
            )
            .into(),
        );
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
        return Some(
            fail_open(
                input,
                "projection-rules",
                &format!("projected policies do not compile: {}", detail.join(", ")),
            )
            .into(),
        );
    }

    let mut messages: Vec<String> = Vec::new();
    let mut any_blocking = false;
    let mut structural_count = 0usize;

    // --- TEXT plane: every file, no grammar required ------------------------
    //
    // The governed plane judges the ARTIFACT, so both the path it matches path
    // scopes against and the exposure it grades tiers by come from the repo the
    // edited FILE belongs to — not from `root`, which is the session's cwd.
    // When the two coincide (an agent editing the tree it is standing in) this
    // is identical to the old behaviour; when they diverge it is the difference
    // between grading the edit and grading the session. See
    // [`crate::git::repo_root_containing`] for the measured failure.
    let target = target_file(input, root);
    let target_root = target.as_deref().and_then(crate::git::repo_root_containing);
    let target_rel = match (&target, &target_root) {
        (Some(file), Some(tr)) => relative(file, tr),
        _ => rel.to_string(),
    };
    let text_violations =
        crate::textrules::evaluate(registry.text_rules(), &introduced, &target_rel);
    // Carried onto the `governed` metrics line so the advise-mode soak is
    // ADJUDICABLE. `blocking` alone says an edit WOULD have been denied but not
    // whether that denial would have been right, and the spool records no path
    // (audit.record_paths is off by default), so a reviewer had nothing to
    // judge against. The repo NAME and the exposure verdict are the two facts
    // that decide it, and neither is a path.
    let mut exposure_label = "n/a";
    let mut target_repo: Option<String> = None;
    if !text_violations.is_empty() {
        // Exposure is resolved ONCE per edit, from the graph, via the governed
        // policy itself — so hank and every other consumer of rule #1 share one
        // definition of "public". Any failure to ask IS the Unknown answer.
        let repo = target_root.as_deref().and_then(crate::git::origin_repo_name);
        let exposure = match (&target_root, &repo) {
            (Some(_), Some(repo)) => {
                crate::project::fetch_repo_exposure(&config.quipu.endpoint, repo)
            }
            (Some(tr), None) => RepoExposure::Unknown(format!(
                "the tree containing this file ({}) has no `origin` remote, so \
                 its exposure cannot be resolved",
                tr.display()
            )),
            (None, _) => RepoExposure::Unknown(
                "this file is not inside a git work tree, so its exposure cannot \
                 be resolved"
                    .into(),
            ),
        };
        exposure_label = match exposure {
            RepoExposure::Public => "public",
            RepoExposure::Internal => "internal",
            RepoExposure::Unknown(_) => "unknown",
        };
        target_repo = repo;
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
            ("exposure", exposure_label.into()),
            (
                "repo",
                target_repo.unwrap_or_else(|| "unresolved".to_string()).into(),
            ),
        ],
    );
    let blocks = config.policy.mode == Mode::Enforce && any_blocking;
    let message = rule_verdict_message_from(&messages.join("\n"), registry.freshness());
    // The same rule names the `governed` line already reports, carried onto the
    // `guard` line too (hank #77). Without this a governed deny is the one deny
    // an operator has to JOIN two spool lines to attribute — and the join is by
    // timestamp, which is exactly the correlation that failed during the
    // incident that motivated the issue. Structural violations contribute a
    // count, not names (their names live only inside composed messages today),
    // so they are named as such rather than silently omitted.
    let mut fired: Vec<&str> = text_violations.iter().map(|v| v.rule.as_str()).collect();
    if structural_count > 0 {
        fired.push("governed-structural");
    }
    let outcome = if blocks {
        Outcome::Deny(message)
    } else {
        Outcome::Notify(format!("hank (governed, not blocking): {message}"))
    };
    Some(Decision::ruled(outcome, fired_rule_ids(fired.into_iter())))
}

/// The absolute path of the file this edit actually writes to, which is the
/// subject the governed plane grades. A relative `file_path` is resolved
/// against the session root — the only thing it can sensibly mean.
#[cfg(feature = "quipu")]
fn target_file(input: &HookInput, root: &Path) -> Option<PathBuf> {
    let raw = input.tool_input.file_path.as_deref()?;
    let path = Path::new(raw);
    Some(if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    })
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
pub(super) fn text_plane(
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
