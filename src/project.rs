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
    /// Whether [`Self::policies`] reflects a successful, current sync.
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
            freshness: Freshness::Stale,
        }
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
        match fetch_policies(&self.endpoint) {
            Ok(policies) => {
                self.policies = policies;
                self.freshness = Freshness::Fresh;
                Ok(())
            }
            Err(e) => {
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
}

/// Fetch and decode quipu's structural policies over HTTP.
///
/// Mirrors `promote::write_knot`: a blocking `POST` to quipu's `/query`, asking
/// for the W3C-standard `application/sparql-results+json` shape so the decode is
/// version-stable.
pub fn fetch_policies(endpoint: &str) -> Result<Vec<ProjectedPolicy>> {
    let url = format!("{}/query", endpoint.trim_end_matches('/'));
    let body = serde_json::json!({ "query": POLICY_QUERY }).to_string();
    let resp = ureq::post(&url)
        .set("Content-Type", "application/json")
        .set("Accept", "application/sparql-results+json")
        .send_string(&body)
        .map_err(|e| Error::Projection(format!("POST {url} failed: {e}")))?;
    let text = resp
        .into_string()
        .map_err(|e| Error::Projection(format!("could not read /query response: {e}")))?;
    decode_policies(&text)
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
