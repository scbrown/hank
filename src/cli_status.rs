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
                // The signed rule set (aegis-hac0) does not exist yet; report its
                // ABSENCE explicitly rather than omitting it, so the day it lands
                // the surface is already here and its absence was never silent.
                "signed_rule_set": { "loaded": false, "state": "never-loaded",
                    "note": "local config only; resident signed cache not yet available" },
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

    // The signed rule set does not exist yet (aegis-hac0). Its absence is a
    // reported state, not silence: a never-loaded rule set is a failure surface.
    println!(
        "  rule set    : {} (local config only)",
        "none — never loaded".yellow()
    );
}
