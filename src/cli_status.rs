//! `hank status` and its policy rendering, split out of `cli` for size
//! (hank #83). See `cli_analyze` for why this is a child module.

use super::*;
use crate::types::Tier;

impl Cli {
    /// Print base ref, tier availability, and config.
    pub(super) fn status(&self) -> anyhow::Result<()> {
        let root = std::env::current_dir()?;
        let config = self.load_config(&root)?;
        let tenant = self.tenant.as_deref().unwrap_or("(single-tenant)");
        // Resolve the configured base ref to a concrete commit (None outside a
        // repo / unresolved ref — degrade, never fail).
        let base_commit = crate::git::resolve_commit(&root, &config.base_ref);

        let policy = config.policy.status_for(self.tenant.as_deref());

        let rule_set = measure_rule_set(&config);
        if self.json {
            let out = serde_json::json!({
                "base_ref": config.base_ref,
                "base_commit": base_commit,
                "tenant": tenant,
                "tiers": Tier::served(),
                "quipu": { "enabled": config.quipu.enabled, "branch_model": config.quipu.branch_model },
                "policy": policy,
                // Whether guard records will carry their subject (hank #77). An
                // operator has to be able to confirm this from OUTSIDE the
                // process: "recording is on" believed but untrue looks exactly
                // like "nothing was denied" in the spool.
                "metrics": { "record_paths": config.metrics.record_paths },
                // MEASURED, not asserted — see measure_rule_set. This object used
                // to be four hardcoded literals claiming nothing was ever loaded,
                // on every box, whatever the graph held.
                "rule_set": {
                    "local": rule_set.local,
                    "graph_enabled": rule_set.graph_enabled,
                    "projected": rule_set.projected,
                    "structural": rule_set.structural,
                    "text": rule_set.text,
                    "error": rule_set.error,
                    // The field `st doctor` gates on. `degraded` is the state in
                    // which the guard FAILS OPEN, and it is deliberately not the
                    // same value as `empty` (aegis-hac0).
                    "state": rule_set.state().as_str(),
                    // WHICH rule set, not just how many.
                    "digest": rule_set.digest,
                    "attempts": rule_set.attempts,
                    // No signed cache => nothing is verified. Said out loud
                    // rather than omitted, so the field a signed cache will fill
                    // exists and reads honestly today.
                    "verification": "unsigned",
                },
                // The SIGNED resident cache still does not exist.
                // Its absence is real and stays reported — but as PROVENANCE of
                // the rules above, which is a different fact from "there are no
                // rules". Conflating the two is the bug this replaced.
                "signed_rule_set": { "loaded": false, "state": "never-loaded",
                    "note": "rules above are an unsigned live projection; the resident signed cache is not yet available" },
            });
            println!("{}", serde_json::to_string_pretty(&out)?);
        } else {
            let commit = base_commit.as_deref().map_or_else(
                || "(unresolved — not a git repo or ref absent)".to_string(),
                |c| c[..c.len().min(12)].to_string(),
            );
            println!("{}", "hank status".bold());
            println!("  base ref    : {}", config.base_ref);
            println!("  base commit : {commit}");
            println!("  tenant      : {tenant}");
            println!("  tiers       : {}", Tier::served().join(", "));
            println!(
                "  quipu       : enabled={} branch_model={}",
                config.quipu.enabled, config.quipu.branch_model
            );
            println!(
                "  audit       : record_paths={}",
                match config.metrics.record_paths {
                    crate::audit::PathRecording::Off => "off (guard records carry no path)",
                    crate::audit::PathRecording::Relative => "relative",
                    crate::audit::PathRecording::Absolute => "absolute",
                }
            );
            print_policy_status(&policy);
            print_rule_set_status(&config, &rule_set);
        }

        // THE RULE SET IS A FAILURE SURFACE, NOT A LINE OF PROSE (aegis-hac0).
        //
        // `degraded` means the guard could not fetch the rules it enforces and
        // therefore FAILS OPEN on every edit until the graph answers again. That
        // is precisely the state the ruling refuses to let anyone learn from
        // silence — "you write to quipu, believe the rule is live, and cannot
        // tell". Before this, `hank status` printed COULD NOT TELL in red and
        // exited 0, so no script could gate on it and no human had to notice.
        //
        // Only `degraded` exits non-zero. `empty` does not: an empty graph is a
        // true, quiet answer about policy, not a fault of this deployment, and
        // failing on it would make the exit code useless in every tree that has
        // no governed rules yet.
        if rule_set.state() == RuleSetState::Degraded {
            std::process::exit(EXIT_RULE_SET_DEGRADED);
        }
        Ok(())
    }
}

/// `hank status` exit code for a rule plane that could not be loaded.
///
/// A dedicated code, not `1`: `1` is "the command itself failed" and `2` is
/// clap's argument error (and Claude Code's hook-deny), so a caller that gates
/// on this can tell "hank could not tell me about the rules" apart from "hank
/// did not run". The whole point is that the two stop looking alike.
pub(super) const EXIT_RULE_SET_DEGRADED: i32 = 3;

/// Render the policy section of `hank status`.
///
/// Shows the enforcement mode, whether a scope applies to this tenant and its
/// ceilings, and — loudly — two states an operator must never learn from
/// silence: an `enforce` mode with no scope for the tenant (armed-looking, inert),
/// and the absence of a signed rule set (aegis-hac0).
fn print_policy_status(policy: &crate::policy::PolicyStatus) {
    let scope = match &policy.scope {
        Some(s) => {
            let ceiling = |c: Option<usize>| c.map_or_else(|| "—".to_string(), |n| n.to_string());
            format!(
                "configured (allow={} deny={} sym≤{} files≤{})",
                s.allow_paths,
                s.deny_paths,
                ceiling(s.max_impacted_symbols),
                ceiling(s.max_impacted_files),
            )
        }
        None => "none for this tenant".to_string(),
    };
    println!("  policy      : mode={}  scope={scope}", policy.mode);

    if policy.enforcing_without_scope {
        println!(
            "  {} enforce mode but NO scope for this tenant — nothing is enforced",
            "⚠".yellow().bold()
        );
    }
}

/// What `hank status` says about the RULE SET — measured, not asserted.
///
/// THIS LINE USED TO BE A STRING LITERAL. It printed
/// `rule set : none — never loaded (local config only)` unconditionally, on
/// every box, whatever the graph held. The intent was to report the absence of
/// the *signed resident cache*, which genuinely does not exist yet — but the
/// words it chose describe the RULE SET, and they were read the way they read.
///
/// The cost was not hypothetical. An operator ran `hank status`, saw
/// "none — never loaded", and concluded the governed rule plane was empty and
/// that every claim of change-time enforcement in this deployment was false. A
/// P1 was filed to build what already existed. Measured at that moment: seven
/// rules in the graph, projected on every edit, verifiably firing.
///
/// This repo is careful about controls that report health they never measured.
/// This is the mirror image — a control reporting FAILURE it never measured —
/// and it is not the harmless direction. A false red burns the time of whoever
/// believes it, and teaches everyone else to discount the surface.
///
/// So: the signed-cache line stays (its absence is real and worth reporting),
/// but it says PROVENANCE, and the rule-set line now counts what is actually
/// loaded, through the same projection the guard itself uses. One reader, no
/// second opinion — a status that could disagree with the hook would be a third
/// thing to keep in sync.
pub(super) struct RuleSetStatus {
    pub local: usize,
    /// None = the graph plane is off for this build/config.
    pub projected: Option<usize>,
    pub structural: usize,
    pub text: usize,
    /// Set when the projection was attempted and FAILED — never conflated with
    /// a successful projection of zero rules.
    pub error: Option<String>,
    pub graph_enabled: bool,
    /// WHICH rule set is live, as a content digest over the projected rules
    /// (aegis-hac0 acceptance: "hank status reports rule-set version").
    ///
    /// There is no signed cache yet, so there is no authoritative VERSION to
    /// report — and inventing a version number for an unauthenticated fetch
    /// would be the false-confidence direction this file already got burned on.
    /// A digest is the honest form of the same question: it answers "is the rule
    /// set I am enforcing the same one as an hour ago / on that other host?"
    /// without claiming provenance it does not have. When the signed cache
    /// lands it carries a real version and this becomes the thing that version
    /// is checked against.
    pub digest: Option<String>,
    /// How many projection attempts it took (see [`PROJECTION_ATTEMPTS`]).
    pub attempts: usize,
}

/// What state the rule plane is in — the field an operator and `st doctor` gate
/// on, and the reason `hank status` can exit non-zero (aegis-hac0).
///
/// These are kept DISTINCT for the reason the whole bead exists: "the graph
/// projected no rules" and "I could not reach the graph" produce the same
/// number of enforced rules (zero) and mean opposite things. Collapsing them is
/// how a policy layer goes green-and-dead.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum RuleSetState {
    /// The graph plane is off for this build/config — nothing is projected, and
    /// that is a configuration, not a fault.
    Off,
    /// Rules projected and in force.
    Loaded,
    /// The graph answered, and answered with nothing. Armed and empty.
    Empty,
    /// The graph could not be reached or its answer did not decode. The guard
    /// FAILS OPEN in this state, so it is a failure surface, not a note.
    Degraded,
}

impl RuleSetState {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            RuleSetState::Off => "off",
            RuleSetState::Loaded => "loaded",
            RuleSetState::Empty => "empty",
            RuleSetState::Degraded => "degraded",
        }
    }
}

impl RuleSetStatus {
    pub(super) fn state(&self) -> RuleSetState {
        if !self.graph_enabled {
            return RuleSetState::Off;
        }
        match (self.projected, &self.error) {
            (_, Some(_)) => RuleSetState::Degraded,
            (Some(0), None) => RuleSetState::Empty,
            (Some(_), None) => RuleSetState::Loaded,
            (None, None) => RuleSetState::Degraded,
        }
    }
}

/// MEASURE the rule set. Split from rendering so the JSON and human surfaces
/// cannot disagree — two renderers of one measurement, never two measurements.
pub(super) fn measure_rule_set(config: &HankConfig) -> RuleSetStatus {
    let local = config.policy.rules.len();
    // `mut` is used only under the quipu feature; without it the graph plane
    // does not exist and the struct is returned as built.
    #[allow(unused_mut)]
    let mut st = RuleSetStatus {
        local,
        projected: None,
        structural: 0,
        text: 0,
        error: None,
        graph_enabled: false,
        digest: None,
        attempts: 0,
    };

    #[cfg(feature = "quipu")]
    {
        if !config.quipu.enabled || config.quipu.endpoint.is_empty() {
            return st;
        }
        st.graph_enabled = true;
        // The SAME path the guard takes (hook::rule_planes::governed_check), so
        // this can never report a rule set the guard would not use.
        for attempt in 1..=PROJECTION_ATTEMPTS {
            st.attempts = attempt;
            let mut registry = crate::project::ProjectionRegistry::new(&config.quipu.endpoint);
            match registry.refresh() {
                Ok(()) => {
                    st.text = registry.text_rules().len();
                    st.structural = registry.policies().len();
                    st.projected = Some(st.text + st.structural);
                    st.digest = Some(rule_set_digest(&registry));
                    st.error = None;
                    break;
                }
                Err(e) => {
                    st.error = Some(e.to_string());
                    if attempt < PROJECTION_ATTEMPTS {
                        std::thread::sleep(PROJECTION_BACKOFF);
                    }
                }
            }
        }
    }
    st
}

/// How many times `hank status` retries the projection before calling the plane
/// DEGRADED — and why `status` retries when the hook deliberately does not.
///
/// MEASURED 2026-08-01 on this deployment: the governed text-rule query returns
/// in 0.19s median (15 samples, max 0.35s), but quipu intermittently stalls past
/// the guard's 2s ceiling — two spikes of 2.7s and 2.8s were caught inside one
/// short session, and the first `hank status` run of that session reported
/// COULD NOT TELL purely because it landed on one.
///
/// The hook must NOT retry: it runs on every Edit/Write/MultiEdit across the
/// fleet and its whole latency budget is the reason the ceiling exists. `status`
/// is a human/`st doctor` surface off the hot path, so it can afford three tries
/// over a couple of seconds — and it must, because this command is about to gate
/// an exit code. A red that appears ~1 run in 10 from a transient blip is the
/// failure mode this repo keeps writing down: a control that cries wolf gets
/// routed around, and then the real red is invisible too.
///
/// Note what this does NOT do: it does not hide the flap. `attempts` is
/// reported, so "it took three tries" is visible rather than smoothed away.
#[cfg(feature = "quipu")]
const PROJECTION_ATTEMPTS: usize = 3;
#[cfg(feature = "quipu")]
const PROJECTION_BACKOFF: std::time::Duration = std::time::Duration::from_millis(250);

/// A stable content digest of the projected rule set.
///
/// Sorted and built from the fields that decide ENFORCEMENT — a rule's identity,
/// what it matches, how hard it bites, and where it is exempt. Two hosts
/// enforcing the same rules produce the same digest; a rule whose regex or tier
/// was edited in the graph produces a different one, which is exactly the
/// question "did the policy change under me?" that the projection alone cannot
/// answer. Deliberately NOT a hash of the raw HTTP body: that would also change
/// on binding order or a rationale typo, and a digest that changes for reasons
/// nobody cares about is one nobody watches.
#[cfg(feature = "quipu")]
fn rule_set_digest(registry: &crate::project::ProjectionRegistry) -> String {
    use sha2::{Digest, Sha256};
    let mut ids: Vec<String> = registry
        .text_rules()
        .iter()
        .map(|r| {
            format!(
                "text\u{1f}{}\u{1f}{}\u{1f}{:?}\u{1f}{}",
                r.name,
                r.pattern,
                r.tier,
                r.exempt_path_regex.as_deref().unwrap_or("")
            )
        })
        .collect();
    ids.extend(registry.policies().iter().map(|p| {
        format!(
            "struct\u{1f}{}\u{1f}{}\u{1f}{}",
            p.rule.name, p.rule.pattern, p.effect
        )
    }));
    ids.sort();
    format!("sha256:{}", hex::encode(Sha256::digest(ids.join("\u{1e}"))))
}

/// Render the rule-set section from an ALREADY-TAKEN measurement.
///
/// It takes `&RuleSetStatus` rather than re-measuring, and that is a fix, not a
/// refactor: `status()` measured once for the JSON surface and this function
/// measured AGAIN for the text surface, so a text-mode `hank status` ran the
/// projection twice — two HTTP round-trips per invocation, and two measurements
/// that could disagree. The struct's own doc comment forbids exactly that ("two
/// renderers of one measurement, never two measurements"); the second call had
/// quietly reintroduced it. On a plane that flaps (see `PROJECTION_ATTEMPTS`)
/// the two could genuinely differ, and the one printed was not the one the
/// exit code would have been computed from.
fn print_rule_set_status(config: &HankConfig, st: &RuleSetStatus) {
    let local = st.local;

    {
        if !st.graph_enabled {
            println!(
                "  rule set    : {local} local structural rule(s); graph plane OFF \
                 (quipu disabled or not built in — nothing is projected)"
            );
            println!("  rule provenance: local config only");
            return;
        }
        // The SAME path the guard takes (hook::rule_planes::governed_check), so
        // this can never report a rule set the guard would not use.
        match (st.projected, &st.error) {
            (_, Some(e)) => {
                // Fail LOUD, same discipline as the guard's fail-open: a rule
                // set we could not fetch is never reported as a rule set we do
                // not have.
                println!(
                    "  rule set    : {} — could not project from {} after {} attempt(s) ({e}); \
                     {local} local rule(s) only",
                    "COULD NOT TELL".red().bold(),
                    config.quipu.endpoint,
                    st.attempts,
                );
                // Name the CONSEQUENCE, not just the fault. "could not project"
                // is a fact about hank; "every edit is sailing through" is the
                // fact the reader has to act on.
                println!(
                    "  {} the guard is FAILING OPEN for governed rules until this clears \
                     (exit {EXIT_RULE_SET_DEGRADED})",
                    "⚠".red().bold()
                );
            }
            (Some(0), None) => {
                // Genuinely empty IS sayable — loudly, and only when true.
                println!(
                    "  rule set    : {}",
                    format!("0 projected from quipu + {local} local").yellow()
                );
                println!(
                    "  {} the graph projected NO rules — the plane is armed and empty",
                    "⚠".yellow().bold()
                );
            }
            (Some(total), None) => {
                println!(
                    "  rule set    : {}",
                    format!(
                        "{total} projected from quipu ({} structural, {} text) + {local} local",
                        st.structural, st.text
                    )
                    .green()
                );
                // WHICH rule set. Two hosts that disagree here are enforcing
                // different policy, which is invisible in the counts alone.
                if let Some(d) = &st.digest {
                    println!("  rule digest : {} (unsigned)", &d[..d.len().min(23)]);
                }
            }
            (None, None) => {}
        }
    }

    // The SIGNED resident cache still does not exist, and its
    // absence is still worth reporting — but as what it is: the provenance of
    // the rules above is an unauthenticated fetch, not a verified cache. That is
    // a real caveat and a different sentence from "there are no rules".
    println!(
        "  rule provenance: unsigned live projection — the signed resident cache \
         does not exist yet, so rules are trusted on transport alone"
    );
}
