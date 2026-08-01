//! Tests for `project` — the quipu policy projection registry and its
//! freshness/exposure semantics. Child module of `project` (`super::*` reaches
//! its private helpers); size-exempt (`_test.rs`).

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
