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
            print_rule_set_status(&config);
        }
        Ok(())
    }
}

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
    };

    #[cfg(feature = "quipu")]
    {
        if !config.quipu.enabled || config.quipu.endpoint.is_empty() {
            return st;
        }
        st.graph_enabled = true;
        // The SAME path the guard takes (hook::rule_planes::governed_check), so
        // this can never report a rule set the guard would not use.
        let mut registry = crate::project::ProjectionRegistry::new(&config.quipu.endpoint);
        match registry.refresh() {
            Ok(()) => {
                st.text = registry.text_rules().len();
                st.structural = registry.policies().len();
                st.projected = Some(st.text + st.structural);
            }
            Err(e) => st.error = Some(e.to_string()),
        }
    }
    st
}

fn print_rule_set_status(config: &HankConfig) {
    let st = measure_rule_set(config);
    let local = st.local;

    {
        if !st.graph_enabled {
            println!(
                "  rule set    : {local} local structural rule(s); graph plane OFF \
                 (quipu disabled or not built in — nothing is projected)"
            );
            println!(
                "  rule provenance: local config only"
            );
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
                    "  rule set    : {} — could not project from {} ({e}); \
                     {local} local rule(s) only",
                    "COULD NOT TELL".red().bold(),
                    config.quipu.endpoint
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
