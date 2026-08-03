//! Phase-4 projection: a hot, one-directional cache of quipu's structural
//! policies (`quipu` feature).
//!
//! **Evidence locality** (`docs/book/src/design/policy-edit-hooks.md`): a
//! `tree-sitter`-tier policy is *defined* in quipu but *evaluated* in Hank, where
//! the code structure is hot. Hank never originates a policy — it projects
//! quipu's canonical `boundary:"action"` structural policies into the same
//! [`Rule`](crate::rules::Rule) shape the local config uses, and evaluates them at
//! the pre-edit seam. The projection is strictly one-directional (quipu canonical
//! → hank cache); if it diverged, Hank could allow what quipu would deny, so a
//! projected verdict always declares the cache's [`Freshness`].
//!
//! Like promotion (`src/promote.rs`), this talks to quipu over HTTP — quipu's
//! `POST /query` with the W3C-standard `application/sparql-results+json` shape —
//! so it needs no `quipu` *crate* dependency, only a blocking client. The decode
//! ([`decode_policies`]) is pure and testable against a canned result; the fetch
//! ([`fetch_policies`]) is the thin network wrapper.

use crate::errors::{Error, Result};
use crate::rules::Rule;
use crate::textrules::TextRule;
use crate::types::Freshness;

pub use crate::project_decode::{decode_policies, decode_text_rules};
pub use crate::project_queries::{EXPOSURE_POLICY_IRI, POLICY_QUERY, TEXT_POLICY_QUERY};

/// A policy projected from quipu: the [`Rule`] Hank evaluates, plus the governed
/// `effect` that decides what a violation does (independent of the local
/// `[hank.policy] mode`, so a quipu `deny` denies and a `warn` advises).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectedPolicy {
    /// The structural rule, decoded into the same shape local config uses.
    pub rule: Rule,
    /// The governed effect: `deny`, `warn`, `require-approval`, `escalate`,
    /// `record`, `throttle`, or `allow`. The RESPONSE when the rule fires —
    /// distinct from `rule.class`, which is what kind of bound it is.
    pub effect: String,
    /// The per-constraint latency budget quipu declared (SARC §5.1), if any.
    /// Carried rather than enforced today: hank's guard already has a
    /// whole-run `deadline_ms`, and a per-rule budget only becomes meaningful
    /// once rules can individually exceed it.
    pub latency_budget_ms: Option<u64>,
    /// The layer quipu CLAIMS this constraint is enforced at (SARC I6).
    /// A claim, not a fact — [`crate::hosting`] compares it to the layer that
    /// actually evaluates the rule, and reports when the claim is the stronger
    /// of the two.
    pub hosted_at_layer: Option<crate::hosting::HostingLayer>,
}

/// Fetch and decode the governed text-rule catalogue over HTTP.
pub fn fetch_text_rules(endpoint: &str) -> Result<Vec<TextRule>> {
    decode_text_rules(&query(endpoint, TEXT_POLICY_QUERY)?)
}

/// The ceiling on ANY single projection HTTP call. This path runs inside the
/// PRE-EDIT hook, so an unbounded call does not fail — it HANGS EVERY EDIT
/// (measured: a transiently wedged quipu held the guard for the full two
/// minutes a caller was willing to wait; only the harness's own hook timeout
/// stood between that and a frozen fleet). Two seconds is generous for a LAN
/// round-trip and keeps the worst case (two queries + one exposure check)
/// inside the harness's kill window — past it, the projection fails OPEN,
/// loudly, like every other gap on this path.
const HTTP_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(2);

/// POST a SPARQL query to quipu's `/query`, returning the raw body.
fn query(endpoint: &str, sparql: &str) -> Result<String> {
    let url = format!("{}/query", endpoint.trim_end_matches('/'));
    let body = serde_json::json!({ "query": sparql }).to_string();
    let resp = ureq::post(&url)
        .timeout(HTTP_TIMEOUT)
        .set("Content-Type", "application/json")
        .set("Accept", "application/sparql-results+json")
        .send_string(&body)
        .map_err(|e| Error::Projection(format!("POST {url} failed: {e}")))?;
    resp.into_string()
        .map_err(|e| Error::Projection(format!("could not read /query response: {e}")))
}

/// How exposed is the repo an edit lands in? Three-valued BY DESIGN (the
/// mqnl seam): collapsing "not in the graph" into either answer is the bug.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RepoExposure {
    /// The graph says this repo has a public remote: block-tier rules block.
    Public,
    /// The graph knows this repo and it has no public remote: block-tier
    /// rules DOWNGRADE to warnings — the token is not leaking anywhere, but
    /// saying so keeps the habit honest.
    Internal,
    /// The graph does not know this repo (or could not be asked). Warn AND SAY
    /// SO — never block on a guess, never stay silent on ignorance. Carries
    /// the reason so the verdict can explain itself.
    Unknown(String),
}

/// Ask quipu whether `repo` (by label) is public, via the governed policy's
/// own `/policy/check` — the same signed-verdict seam every other consumer of
/// rule #1 uses, so hank and the pre-push gate can never disagree about what
/// "public" means. NEVER errors: any failure IS the `Unknown` answer, with the
/// reason carried.
///
/// `outcome` mapping (quipu's three-valued contract):
///   satisfied   -> the repo has a public remote        -> Public
///   unsatisfied -> known repo, no public remote        -> Internal
///   unknown     -> the evidence probe found no repo    -> Unknown
pub fn fetch_repo_exposure(endpoint: &str, repo: &str) -> RepoExposure {
    let url = format!("{}/policy/check", endpoint.trim_end_matches('/'));
    let target = format!("http://aegis.gastown.local/ontology/repo_{repo}");
    let body = serde_json::json!({ "policy": EXPOSURE_POLICY_IRI, "target": target }).to_string();
    let resp = match ureq::post(&url)
        .timeout(HTTP_TIMEOUT)
        .set("Content-Type", "application/json")
        .send_string(&body)
    {
        Ok(r) => r,
        Err(e) => return RepoExposure::Unknown(format!("POST {url} failed: {e}")),
    };
    let text = match resp.into_string() {
        Ok(t) => t,
        Err(e) => return RepoExposure::Unknown(format!("unreadable /policy/check reply: {e}")),
    };
    let value: serde_json::Value = match serde_json::from_str(&text) {
        Ok(v) => v,
        Err(e) => return RepoExposure::Unknown(format!("/policy/check reply is not JSON: {e}")),
    };
    match value.get("outcome").and_then(|o| o.as_str()) {
        Some("satisfied") => RepoExposure::Public,
        Some("unsatisfied") => RepoExposure::Internal,
        Some("unknown") | None => RepoExposure::Unknown(format!(
            "repo `{repo}` is not in the graph (no `repo_{repo}` entity with remote facts)"
        )),
        Some(other) => RepoExposure::Unknown(format!("unrecognised outcome `{other}`")),
    }
}

/// A violation of a projected policy: the model-facing message plus whether the
/// governed `effect` blocks the edit (as opposed to merely advising).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectedViolation {
    /// The rule's model-facing message.
    pub message: String,
    /// Whether the governed effect is a blocking one (`deny` / `require-approval`
    /// / `escalate`) rather than advisory (`warn` / `record` / `allow`).
    pub blocking: bool,
    /// The constraint's stable id.
    ///
    /// Carried so the trace record can name it. Before this, structural
    /// violations reached the spool as the literal string
    /// `"governed-structural"` and a count — the names existed only inside the
    /// composed model-facing message, so an operator could see that three
    /// governed rules fired and not which three. That is the same
    /// unattributable-record shape the audit field was added to fix.
    pub id: String,
    /// The declared class, when quipu declared one.
    pub class: Option<crate::constraint::ConstraintClass>,
    /// The declared verification point, when quipu declared one.
    pub verification_point: Option<crate::constraint::VerificationPoint>,
}

/// Whether a governed `effect` blocks an edit. Unknown effects are treated as
/// blocking — the conservative direction for a governed decision Hank does not
/// recognise.
///
/// `throttle` is advisory HERE and only here: it is the soft-class PAA response
/// (SARC §4.2), and a soft constraint that blocks at the pre-edit gate is a hard
/// constraint with a misleading name. Its actual backoff belongs at the
/// post-edit auditor, which is Phase 3 — until then a `throttle` policy warns,
/// and the verdict says the throttle was not applied rather than implying it was.
#[must_use]
pub fn effect_blocks(effect: &str) -> bool {
    !matches!(effect, "warn" | "record" | "allow" | "throttle")
}

/// Whether a projected policy blocks this edit, given the ambient mode.
///
/// The DECLARED CLASS decides when quipu supplied one; the `effect` is the
/// fallback for a policy projected from a catalog that predates
/// `aegis:constraintClass`. Class-first is the point of Phase 1: `effect` says
/// what happens when a rule fires, `class` says whether the action was ever
/// admissible, and reading the response as if it were the class is what made
/// "is this a hard constraint?" unanswerable.
///
/// Two consequences worth stating, because both are behaviour changes:
///
/// - a **soft** policy never blocks, even with `effect "deny"`. That combination
///   is contradictory, and quipu's placement check now refuses to define it —
///   but a store that predates the check can still hold one, and honouring the
///   class is the reading that matches what the author declared it to BE.
/// - a policy declared at a point hank does not host at pre-edit (`PAA`, `ATM`)
///   does not fire here at all. Evaluating a PAA rule at the gate would block on
///   evidence its author said should be judged after the fact.
#[must_use]
pub fn policy_blocks(policy: &ProjectedPolicy, mode: crate::policy::Mode) -> bool {
    match policy.rule.class {
        Some(class) => class.blocks(mode),
        // No declared class: pre-Phase-1 behaviour, governed effect decides,
        // still under the mode ceiling.
        None => mode == crate::policy::Mode::Enforce && effect_blocks(&policy.effect),
    }
}

/// Whether this policy is evaluated at hank's pre-edit gate.
///
/// An undeclared verification point means "wherever hank evaluates it", which
/// is the pre-edit seam — the behaviour before the field existed. A DECLARED
/// point is honoured: a `PAA` policy is skipped here and belongs to the
/// post-edit auditor.
#[must_use]
pub fn runs_at_pre_edit(policy: &ProjectedPolicy) -> bool {
    policy
        .rule
        .verification_point
        .is_none_or(crate::constraint::VerificationPoint::is_pre_edit)
}

/// Evaluate every projected policy that runs at the pre-edit gate against the
/// introduced text, tagging each violation with whether it blocks under `mode`.
/// The rule engine is the same one local config uses — congruence means a
/// projected policy is just a [`Rule`].
///
/// Policies declared at a point hank does not host here ([`runs_at_pre_edit`])
/// are SKIPPED, not evaluated-and-ignored: a PAA rule has nothing to say about
/// a proposed edit, and reporting it at the gate would tell the model to fix
/// something the rule's author scoped to after the fact.
#[must_use]
pub fn evaluate_projected(
    policies: &[ProjectedPolicy],
    source: &str,
    language: &str,
    rel: &str,
    mode: crate::policy::Mode,
) -> Vec<ProjectedViolation> {
    let mut out = Vec::new();
    for policy in policies.iter().filter(|p| runs_at_pre_edit(p)) {
        let blocking = policy_blocks(policy, mode);
        for violation in
            crate::rules::evaluate(std::slice::from_ref(&policy.rule), source, language, rel)
        {
            out.push(ProjectedViolation {
                message: violation.message,
                blocking,
                id: violation.rule,
                class: policy.rule.class,
                verification_point: policy.rule.verification_point,
            });
        }
    }
    out
}

/// A hot, one-directional cache of quipu's structural policies, with the sync
/// state every projected verdict must declare.
///
/// [`Freshness::Fresh`] after a successful refresh; [`Freshness::Stale`] when a
/// refresh failed (the last-known policies are retained, but a verdict computed
/// against them is honestly stale) or before the first refresh.
#[derive(Debug, Clone)]
pub struct ProjectionRegistry {
    /// The quipu base URL (e.g. `http://localhost:8080`); `/query` is appended.
    endpoint: String,
    /// Last-known projected policies.
    policies: Vec<ProjectedPolicy>,
    /// Last-known governed text rules (aegis-mqnl catalogue).
    text_rules: Vec<TextRule>,
    /// Whether the projected sets reflect a successful, current sync.
    freshness: Freshness,
}

impl ProjectionRegistry {
    /// A registry pointed at `endpoint`, not yet synced — so it starts
    /// [`Freshness::Stale`] with no policies, never a fresh-looking empty set.
    #[must_use]
    pub fn new(endpoint: &str) -> Self {
        Self {
            endpoint: endpoint.to_string(),
            policies: Vec::new(),
            text_rules: Vec::new(),
            freshness: Freshness::Stale,
        }
    }

    /// The governed text rules and how current they are — same freshness
    /// contract as [`Self::policies`].
    #[must_use]
    pub fn text_rules(&self) -> &[TextRule] {
        &self.text_rules
    }

    /// The current policies and how current they are. A caller threads
    /// [`Self::freshness`] into the verdict so a stale projection is never served
    /// as fresh.
    #[must_use]
    pub fn policies(&self) -> &[ProjectedPolicy] {
        &self.policies
    }

    /// The cache's sync state.
    #[must_use]
    pub fn freshness(&self) -> Freshness {
        self.freshness
    }

    /// Refresh from quipu. On success, replaces the policies and marks the cache
    /// [`Freshness::Fresh`]; on failure, KEEPS the last-known policies but marks
    /// them [`Freshness::Stale`] and returns the error, so a caller can both keep
    /// enforcing (fail-open on the network, not on the policy) and report the
    /// staleness.
    pub fn refresh(&mut self) -> Result<()> {
        // BOTH catalogues or neither: a refresh that replaced the structural
        // policies but silently kept stale text rules would let the two planes
        // disagree about which sync they reflect — one freshness, one sync.
        match (
            fetch_policies(&self.endpoint),
            fetch_text_rules(&self.endpoint),
        ) {
            (Ok(policies), Ok(text_rules)) => {
                // I6, at the seam where both halves are known at once: what the
                // catalog CLAIMS about its hosting layer, against the layer that
                // will actually evaluate it. Once per refresh rather than per
                // edit — a metadata defect repeated on every guard line is a
                // notice people learn to scroll past.
                report_overclaims(&policies);
                self.policies = policies;
                self.text_rules = text_rules;
                self.freshness = Freshness::Fresh;
                Ok(())
            }
            (Err(e), _) | (_, Err(e)) => {
                self.freshness = Freshness::Stale;
                Err(e)
            }
        }
    }

    /// Install decoded policies directly (test/daemon seam), marking the cache
    /// fresh — refresh over HTTP is [`Self::refresh`].
    pub fn set_policies(&mut self, policies: Vec<ProjectedPolicy>) {
        self.policies = policies;
        self.freshness = Freshness::Fresh;
    }

    /// Install decoded text rules directly (test/daemon seam).
    pub fn set_text_rules(&mut self, text_rules: Vec<TextRule>) {
        self.text_rules = text_rules;
        self.freshness = Freshness::Fresh;
    }
}

/// Report every policy claiming a hosting layer stronger than the one that will
/// evaluate it (SARC I6, `H-SARC-I6`).
///
/// A LOUD FAIL-OPEN: the mismatch is recorded and the policy is projected
/// anyway. Refusing to project it would disable a rule that does still work —
/// trading a documentation defect for an enforcement gap, which is the worse of
/// the two by a wide margin. The rule then evaluates at the layer that is real,
/// and the trace records that layer, so the audit checker verifies the claim
/// against the record instead of taking the claim.
fn report_overclaims(policies: &[ProjectedPolicy]) {
    let claims: Vec<(String, Option<crate::hosting::HostingLayer>)> = policies
        .iter()
        .map(|p| (p.rule.name.clone(), p.hosted_at_layer))
        .collect();
    let notices = crate::hosting::audit_projection(&claims, crate::hosting::HANK_HOSTS_AT);
    if notices.is_empty() {
        return;
    }
    crate::metrics::emit(
        "hosting_overclaim",
        &[
            (
                "actual_layer",
                crate::hosting::HANK_HOSTS_AT.as_str().into(),
            ),
            ("count", (notices.len() as u64).into()),
            ("notices", notices.into()),
        ],
    );
}

/// Fetch and decode quipu's structural policies over HTTP.
///
/// Mirrors `promote::write_knot`: a blocking `POST` to quipu's `/query`, asking
/// for the W3C-standard `application/sparql-results+json` shape so the decode is
/// version-stable.
pub fn fetch_policies(endpoint: &str) -> Result<Vec<ProjectedPolicy>> {
    decode_policies(&query(endpoint, POLICY_QUERY)?)
}

#[cfg(test)]
#[path = "project_test.rs"]
mod project_test;
