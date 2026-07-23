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
use crate::rules::{MatchType, Rule};
use crate::textrules::{TextRule, TextTier};
use crate::types::Freshness;

/// A policy projected from quipu: the [`Rule`] Hank evaluates, plus the governed
/// `effect` that decides what a violation does (independent of the local
/// `[hank.policy] mode`, so a quipu `deny` denies and a `warn` advises).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectedPolicy {
    /// The structural rule, decoded into the same shape local config uses.
    pub rule: Rule,
    /// The governed effect: `deny`, `warn`, `require-approval`, `escalate`,
    /// `record`, or `allow`.
    pub effect: String,
}

/// The SPARQL SELECT that pulls every `boundary:"action"`, `tree-sitter`-tier
/// structural policy out of quipu, joined to its Selector and Predicate atoms.
///
/// Only policies that carry BOTH atoms and a selector language are returned — a
/// committed-tier (SPARQL-`claim`-only) policy has no structural evidence to
/// project and is left for quipu's own write gate.
pub const POLICY_QUERY: &str = "\
PREFIX aegis: <http://aegis.gastown.local/ontology/>
PREFIX rdfs: <http://www.w3.org/2000/01/rdf-schema#>
SELECT ?name ?language ?query ?pattern ?matchType ?gate ?effect WHERE {
  ?policy a aegis:Policy ;
          aegis:boundary \"action\" ;
          aegis:selector ?sel ;
          aegis:predicate ?pred .
  ?sel aegis:evidenceSource ?query ;
       aegis:language ?language ;
       aegis:tier \"tree-sitter\" .
  ?pred aegis:evidenceSource ?pattern ;
        aegis:matchType ?matchType .
  OPTIONAL { ?pred aegis:gate ?gate }
  OPTIONAL { ?policy rdfs:label ?name }
  OPTIONAL { ?policy aegis:effect ?effect }
}";

/// The SPARQL SELECT that pulls the governed TEXT-rule catalogue
/// (`aegis:InternalIdentifierPattern`, aegis-mqnl) out of quipu.
///
/// This is the vocabulary the first real governed rule actually shipped in —
/// measured against the live graph, not designed at a whiteboard: per-pattern
/// regex, `enforcementTier` (block|warn), optional `exemptPathRegex`, class and
/// rationale. It is deliberately a SECOND projection query rather than a
/// reshaping of [`POLICY_QUERY`]: a text rule has no Selector (no language, no
/// tree-sitter tier), so forcing it through the structural vocabulary would
/// either invent fake Selector atoms in the graph or silently drop the
/// catalogue — which is exactly what happened: both sides shipped, and the seam
/// returned 0 rows.
pub const TEXT_POLICY_QUERY: &str = "\
PREFIX aegis: <http://aegis.gastown.local/ontology/>
PREFIX rdfs: <http://www.w3.org/2000/01/rdf-schema#>
SELECT ?s ?label ?regex ?class ?tier ?exempt ?rationale WHERE {
  ?s a aegis:InternalIdentifierPattern ;
     aegis:regex ?regex ;
     aegis:enforcementTier ?tier .
  OPTIONAL { ?s rdfs:label ?label }
  OPTIONAL { ?s aegis:identifierClass ?class }
  OPTIONAL { ?s aegis:exemptPathRegex ?exempt }
  OPTIONAL { ?s rdfs:comment ?rationale }
}";

/// Decode the [`TEXT_POLICY_QUERY`] result into text rules. Same contract as
/// [`decode_policies`]: a row missing a required binding, or carrying a tier
/// this build does not recognise, is an [`Error::Projection`] — never a
/// silently dropped rule.
pub fn decode_text_rules(sparql_json: &str) -> Result<Vec<TextRule>> {
    let value: serde_json::Value = serde_json::from_str(sparql_json)
        .map_err(|e| Error::Projection(format!("results are not JSON: {e}")))?;
    let bindings = rows_of(&value)?;

    let mut out = Vec::with_capacity(bindings.len());
    for (i, binding) in bindings.iter().enumerate() {
        let get = |key: &str| -> Option<String> { binding_value(binding, key) };
        let required = |key: &str| -> Result<String> {
            get(key).ok_or_else(|| {
                Error::Projection(format!(
                    "text-rule row {i}: missing required binding `{key}`"
                ))
            })
        };
        let tier = match required("tier")?.as_str() {
            "block" => TextTier::Block,
            "warn" => TextTier::Warn,
            other => {
                // An unrecognised tier blocks nothing silently and allows
                // nothing silently — it is a projection error the guard
                // surfaces as a loud fail-open (the conservative reading of a
                // governed decision this build cannot interpret).
                return Err(Error::Projection(format!(
                    "text-rule row {i}: unknown enforcementTier `{other}`"
                )));
            }
        };
        // The IRI tail is the stable rule name; verdicts cite it.
        let iri = required("s")?;
        let name = iri.rsplit('/').next().unwrap_or(&iri).to_string();
        out.push(TextRule {
            name,
            label: get("label"),
            pattern: required("regex")?,
            tier,
            class: get("class"),
            exempt_path_regex: get("exempt"),
            rationale: get("rationale"),
        });
    }
    Ok(out)
}

/// The `results.bindings` array of a W3C SPARQL-results body, or a projection
/// error naming what was malformed.
fn rows_of(value: &serde_json::Value) -> Result<&Vec<serde_json::Value>> {
    value
        .get("results")
        .and_then(|r| r.get("bindings"))
        .and_then(|b| b.as_array())
        .ok_or_else(|| Error::Projection("results have no `results.bindings` array".to_string()))
}

/// One binding's `.value` string, if present.
fn binding_value(binding: &serde_json::Value, key: &str) -> Option<String> {
    binding
        .get(key)
        .and_then(|v| v.get("value"))
        .and_then(|v| v.as_str())
        .map(str::to_string)
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

/// Decode a W3C `application/sparql-results+json` body (the result of
/// [`POLICY_QUERY`]) into projected policies.
///
/// Pure and testable without a live quipu. A row missing a required binding
/// (`language`/`query`/`pattern`/`matchType`) is a malformed projection and is an
/// [`Error::Projection`] — never silently dropped, so a broken sync cannot look
/// like "quipu has no policies".
pub fn decode_policies(sparql_json: &str) -> Result<Vec<ProjectedPolicy>> {
    let value: serde_json::Value = serde_json::from_str(sparql_json)
        .map_err(|e| Error::Projection(format!("results are not JSON: {e}")))?;
    let bindings = value
        .get("results")
        .and_then(|r| r.get("bindings"))
        .and_then(|b| b.as_array())
        .ok_or_else(|| Error::Projection("results have no `results.bindings` array".to_string()))?;

    let mut out = Vec::with_capacity(bindings.len());
    for (i, binding) in bindings.iter().enumerate() {
        let required = |key: &str| -> Result<String> {
            binding
                .get(key)
                .and_then(|v| v.get("value"))
                .and_then(|v| v.as_str())
                .map(str::to_string)
                .ok_or_else(|| {
                    Error::Projection(format!("row {i}: missing required binding `{key}`"))
                })
        };
        let optional = |key: &str| -> Option<String> {
            binding
                .get(key)
                .and_then(|v| v.get("value"))
                .and_then(|v| v.as_str())
                .map(str::to_string)
        };

        let match_type = match required("matchType")?.as_str() {
            "must-match" => MatchType::MustMatch,
            "must-not-match" => MatchType::MustNotMatch,
            "must-exist" => MatchType::MustExist,
            other => {
                return Err(Error::Projection(format!(
                    "row {i}: unknown matchType `{other}`"
                )))
            }
        };
        let language = required("language")?;
        let query = required("query")?;
        let pattern = required("pattern")?;
        // A policy with no label still needs a stable name for its verdicts.
        let name = optional("name").unwrap_or_else(|| format!("quipu-policy-{i}"));
        let effect = optional("effect").unwrap_or_else(|| "warn".to_string());

        out.push(ProjectedPolicy {
            rule: Rule {
                name,
                language,
                query,
                gate: optional("gate"),
                match_type,
                pattern,
                applies_to: Vec::new(),
                message: None,
            },
            effect,
        });
    }
    Ok(out)
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

/// The governed policy whose claim decides repo exposure — mqnl's rule #1,
/// live in the graph. The IRI is data about the deployment's ontology, like
/// the `aegis:` prefix in the queries above: one namespace, one policy plane.
pub const EXPOSURE_POLICY_IRI: &str =
    "http://aegis.gastown.local/ontology/policy_no-internal-ids-in-public-repos";

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
}

/// Whether a governed `effect` blocks an edit. Unknown effects are treated as
/// blocking — the conservative direction for a governed decision Hank does not
/// recognise.
#[must_use]
pub fn effect_blocks(effect: &str) -> bool {
    !matches!(effect, "warn" | "record" | "allow")
}

/// Evaluate every projected policy against the introduced text, tagging each
/// violation with whether its governed effect blocks. The rule engine is the same
/// one local config uses — congruence means a projected policy is just a [`Rule`].
#[must_use]
pub fn evaluate_projected(
    policies: &[ProjectedPolicy],
    source: &str,
    language: &str,
    rel: &str,
) -> Vec<ProjectedViolation> {
    let mut out = Vec::new();
    for policy in policies {
        let blocking = effect_blocks(&policy.effect);
        for violation in
            crate::rules::evaluate(std::slice::from_ref(&policy.rule), source, language, rel)
        {
            out.push(ProjectedViolation {
                message: violation.message,
                blocking,
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

/// Fetch and decode quipu's structural policies over HTTP.
///
/// Mirrors `promote::write_knot`: a blocking `POST` to quipu's `/query`, asking
/// for the W3C-standard `application/sparql-results+json` shape so the decode is
/// version-stable.
pub fn fetch_policies(endpoint: &str) -> Result<Vec<ProjectedPolicy>> {
    decode_policies(&query(endpoint, POLICY_QUERY)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A standard SPARQL-results JSON body with the two shipped policies.
    fn catalog_json() -> String {
        serde_json::json!({
            "head": { "vars": ["name", "language", "query", "pattern", "matchType", "gate", "effect"] },
            "results": { "bindings": [
                {
                    "name": { "type": "literal", "value": "todo-needs-ticket" },
                    "language": { "type": "literal", "value": "rust" },
                    "query": { "type": "literal", "value": "(line_comment) @c" },
                    "pattern": { "type": "literal", "value": "\\b[A-Z]+-[0-9]+\\b" },
                    "matchType": { "type": "literal", "value": "must-match" },
                    "gate": { "type": "literal", "value": "\\bTODO\\b" },
                    "effect": { "type": "literal", "value": "warn" }
                },
                {
                    "name": { "type": "literal", "value": "no-ticket-in-comment" },
                    "language": { "type": "literal", "value": "rust" },
                    "query": { "type": "literal", "value": "(line_comment) @c" },
                    "pattern": { "type": "literal", "value": "\\b[A-Z]+-[0-9]+\\b" },
                    "matchType": { "type": "literal", "value": "must-not-match" },
                    "effect": { "type": "literal", "value": "deny" }
                }
            ]}
        })
        .to_string()
    }

    #[test]
    fn decodes_the_catalog_into_projected_policies() {
        let policies = decode_policies(&catalog_json()).unwrap();
        assert_eq!(policies.len(), 2);

        let todo = &policies[0];
        assert_eq!(todo.rule.name, "todo-needs-ticket");
        assert_eq!(todo.rule.language, "rust");
        assert_eq!(todo.rule.match_type, MatchType::MustMatch);
        assert_eq!(todo.rule.gate.as_deref(), Some("\\bTODO\\b"));
        assert_eq!(todo.effect, "warn");

        let ban = &policies[1];
        assert_eq!(ban.rule.match_type, MatchType::MustNotMatch);
        assert_eq!(ban.effect, "deny");
        assert!(ban.rule.gate.is_none());
    }

    #[test]
    fn a_decoded_rule_actually_evaluates() {
        // The whole point of congruence: a projected policy is a Rule that runs.
        let policies = decode_policies(&catalog_json()).unwrap();
        let rules: Vec<Rule> = policies.into_iter().map(|p| p.rule).collect();
        let violations = crate::rules::evaluate(&rules, "// see ABC-123\n", "rust", "src/a.rs");
        assert!(
            violations.iter().any(|v| v.rule == "no-ticket-in-comment"),
            "the projected no-ticket rule must fire on a ticket comment"
        );
    }

    #[test]
    fn a_missing_required_binding_is_an_error_not_a_dropped_row() {
        // A broken sync must be loud, never look like "quipu has no policies".
        let bad = serde_json::json!({
            "results": { "bindings": [
                { "language": { "value": "rust" }, "query": { "value": "(line_comment) @c" } }
            ]}
        })
        .to_string();
        let err = decode_policies(&bad).unwrap_err();
        assert!(matches!(err, Error::Projection(_)));
    }

    #[test]
    fn an_unknown_match_type_is_rejected() {
        let bad = serde_json::json!({
            "results": { "bindings": [{
                "language": { "value": "rust" },
                "query": { "value": "(line_comment) @c" },
                "pattern": { "value": "x" },
                "matchType": { "value": "must-implode" }
            }]}
        })
        .to_string();
        assert!(matches!(
            decode_policies(&bad).unwrap_err(),
            Error::Projection(_)
        ));
    }

    #[test]
    fn evaluate_projected_tags_blocking_by_governed_effect() {
        let policies = decode_policies(&catalog_json()).unwrap();
        // The no-ticket policy has effect "deny" (blocking); todo-needs-ticket is
        // "warn" (advisory). A comment carrying a ticket trips the deny policy.
        let violations = evaluate_projected(&policies, "// see ABC-123\n", "rust", "src/a.rs");
        assert_eq!(violations.len(), 1);
        assert!(violations[0].blocking, "a deny-effect policy must block");
        assert!(violations[0].message.contains("no-ticket-in-comment"));
    }

    #[test]
    fn effect_blocks_maps_governed_effects() {
        assert!(effect_blocks("deny"));
        assert!(effect_blocks("require-approval"));
        assert!(effect_blocks("escalate"));
        assert!(!effect_blocks("warn"));
        assert!(!effect_blocks("record"));
        assert!(!effect_blocks("allow"));
        // Unknown effects are conservatively blocking.
        assert!(effect_blocks("mystery"));
    }

    #[test]
    fn a_fresh_registry_starts_stale_and_empty() {
        // Before the first sync there is nothing, and it is honestly stale — never
        // a fresh-looking empty policy set that would silently enforce nothing.
        let reg = ProjectionRegistry::new("http://localhost:8080");
        assert!(reg.policies().is_empty());
        assert_eq!(reg.freshness(), Freshness::Stale);
    }

    #[test]
    fn set_policies_marks_the_cache_fresh() {
        let mut reg = ProjectionRegistry::new("http://localhost:8080");
        reg.set_policies(decode_policies(&catalog_json()).unwrap());
        assert_eq!(reg.freshness(), Freshness::Fresh);
        assert_eq!(reg.policies().len(), 2);
    }

    #[test]
    fn a_failed_refresh_goes_stale_but_keeps_last_known_policies() {
        let mut reg = ProjectionRegistry::new("http://127.0.0.1:1"); // unreachable
        reg.set_policies(decode_policies(&catalog_json()).unwrap());
        assert_eq!(reg.freshness(), Freshness::Fresh);
        // The refresh fails (nothing is listening); the cache goes stale but the
        // last-known policies survive so the guard keeps enforcing, honestly stale.
        assert!(reg.refresh().is_err());
        assert_eq!(reg.freshness(), Freshness::Stale);
        assert_eq!(reg.policies().len(), 2);
    }
}
