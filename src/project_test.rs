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

// --- one entity is one rule (the OPTIONAL cross-product) ----------------------

/// A pattern entity returned TWICE because it carries two `rdfs:comment`s.
/// SPARQL returns the cross product of the query's OPTIONALs, so this is what
/// the live catalogue actually sends — not a contrived shape.
fn two_comments_json() -> String {
    let row = |comment: &str| {
        serde_json::json!({
            "s": { "type": "uri", "value": "http://example.invalid/ontology/pattern_demo" },
            "regex": { "type": "literal", "value": "\\bwidget\\b" },
            "tier": { "type": "literal", "value": "block" },
            "class": { "type": "literal", "value": "hostname" },
            "rationale": { "type": "literal", "value": comment }
        })
    };
    serde_json::json!({
        "head": { "vars": ["s", "regex", "tier", "class", "rationale"] },
        "results": { "bindings": [row("the reason"), row("the exemption note")] }
    })
    .to_string()
}

#[test]
fn one_entity_with_two_comments_is_one_rule() {
    // Measured on the live catalogue before this fix: 7 pattern entities
    // projected as 11 rules. The duplicates fired twice for one edit, and the
    // two copies carried different rationales — an advisory arguing with itself.
    let rules = decode_text_rules(&two_comments_json()).unwrap();
    assert_eq!(
        rules.len(),
        1,
        "the OPTIONAL cross product produced duplicates"
    );
    assert_eq!(rules[0].name, "pattern_demo");
}

#[test]
fn merging_keeps_every_distinct_rationale() {
    // Dropping one would lose the author's reasoning, and picking by row order
    // would be arbitrary AND unstable across graph writes.
    let rules = decode_text_rules(&two_comments_json()).unwrap();
    let rationale = rules[0].rationale.as_deref().unwrap();
    assert!(rationale.contains("the reason"));
    assert!(rationale.contains("the exemption note"));
}

#[test]
fn an_identical_repeated_optional_does_not_duplicate_itself() {
    let row = serde_json::json!({
        "s": { "type": "uri", "value": "http://example.invalid/ontology/pattern_demo" },
        "regex": { "type": "literal", "value": "\\bwidget\\b" },
        "tier": { "type": "literal", "value": "warn" },
        "rationale": { "type": "literal", "value": "same" }
    });
    let body = serde_json::json!({
        "head": { "vars": ["s", "regex", "tier", "rationale"] },
        "results": { "bindings": [row.clone(), row] }
    })
    .to_string();
    let rules = decode_text_rules(&body).unwrap();
    assert_eq!(rules.len(), 1);
    assert_eq!(rules[0].rationale.as_deref(), Some("same"));
}

#[test]
fn a_conflicting_required_field_is_refused_not_merged() {
    // Two different regexes under one name are two rules wearing one name.
    // Picking either silently is how a guard enforces something nobody wrote.
    let mk = |regex: &str| {
        serde_json::json!({
            "s": { "type": "uri", "value": "http://example.invalid/ontology/pattern_demo" },
            "regex": { "type": "literal", "value": regex },
            "tier": { "type": "literal", "value": "block" }
        })
    };
    let body = serde_json::json!({
        "head": { "vars": ["s", "regex", "tier"] },
        "results": { "bindings": [mk("\\bone\\b"), mk("\\btwo\\b")] }
    })
    .to_string();
    let err = decode_text_rules(&body).unwrap_err();
    assert!(
        format!("{err}").contains("conflicting definitions"),
        "a conflicting rule must refuse loudly, got: {err}"
    );
}

#[test]
fn distinct_entities_are_still_distinct_rules() {
    // The dedup keys on the entity, never on the pattern: two entities may
    // legitimately share a regex while differing in tier or exemptions.
    let mk = |name: &str| {
        serde_json::json!({
            "s": { "type": "uri", "value": format!("http://example.invalid/ontology/{name}") },
            "regex": { "type": "literal", "value": "\\bwidget\\b" },
            "tier": { "type": "literal", "value": "warn" }
        })
    };
    let body = serde_json::json!({
        "head": { "vars": ["s", "regex", "tier"] },
        "results": { "bindings": [mk("pattern_a"), mk("pattern_b")] }
    })
    .to_string();
    assert_eq!(decode_text_rules(&body).unwrap().len(), 2);
}
