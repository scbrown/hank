//! Tests for the Σ-derived trace record. Size-exempt (`_test.rs`).

use super::*;

fn unsatisfied(id: &str) -> ConstraintEvaluation {
    ConstraintEvaluation::new(id, Outcome::Unsatisfied, Response::Blocked)
}

#[test]
fn an_evaluation_carries_the_four_fields_the_audit_checker_needs() {
    // SARC Definition 2's passes need, per constraint: which one, where it was
    // evaluated, what it concluded, and what was done about it. A record missing
    // any of the four makes one of the four passes unwritable.
    let e = unsatisfied("no-ticket-in-comment")
        .placed(Some(ConstraintClass::Hard), Some(VerificationPoint::Pag));
    let json = serde_json::to_value(&e).unwrap();
    assert_eq!(json["id"], "no-ticket-in-comment");
    assert_eq!(json["class"], "hard");
    assert_eq!(json["verification_point"], "PAG");
    assert_eq!(json["outcome"], "unsatisfied");
    assert_eq!(json["response"], "blocked");
}

#[test]
fn an_undeclared_class_is_absent_from_the_record_not_guessed() {
    // The spool discipline: an omitted field is honestly silent, where a
    // defaulted one would be replayed later as if someone had declared it.
    let json = serde_json::to_value(unsatisfied("local-rule")).unwrap();
    assert!(
        json.get("class").is_none(),
        "class must be absent, not null or a default: {json}"
    );
    assert!(json.get("verification_point").is_none());
}

#[test]
fn outcome_distinguishes_unknown_from_unsatisfied() {
    // SARC's distinction, and the reason it is load-bearing: `unknown` means no
    // evidence was available, and collapsing it into `unsatisfied` makes an
    // unevaluated check indistinguishable from a passing one in the record the
    // auditor reads.
    let unknown = serde_json::to_value(ConstraintEvaluation::new(
        "tests-green",
        Outcome::Unknown,
        Response::NoAction,
    ))
    .unwrap();
    assert_eq!(unknown["outcome"], "unknown");
    assert_ne!(unknown["outcome"], "unsatisfied");
}

#[test]
fn response_is_recorded_separately_from_outcome() {
    // Pass (iii) of the checker is "does the recorded response match the one the
    // policy declared". If the two collapsed into one field, a constraint that
    // fired and was ignored would be indistinguishable from one that fired and
    // blocked.
    let warned =
        ConstraintEvaluation::new("todo-needs-ticket", Outcome::Unsatisfied, Response::Warned);
    let blocked = unsatisfied("todo-needs-ticket");
    assert_eq!(warned.outcome, blocked.outcome);
    assert_ne!(warned.response, blocked.response);
}

#[test]
fn no_action_is_representable_and_distinct_from_logged() {
    // The state to grep for. A constraint that fired and drew no response is a
    // real thing that happens (a soft rule under a runtime with nowhere to put
    // it) and the record has to be able to say so rather than rounding it to
    // "logged", which reads as a deliberate choice.
    let json = serde_json::to_value(ConstraintEvaluation::new(
        "c",
        Outcome::Unsatisfied,
        Response::NoAction,
    ))
    .unwrap();
    assert_eq!(json["response"], "no-action");
}

#[test]
fn the_constraint_set_serialises_as_one_array() {
    // Not flattened into sibling keys: a reader must never have to reassemble
    // which class went with which id by position.
    let value = to_json(&[
        unsatisfied("a").placed(Some(ConstraintClass::Hard), Some(VerificationPoint::Pag)),
        ConstraintEvaluation::new("b", Outcome::Satisfied, Response::Logged),
    ]);
    let array = value.as_array().expect("an array");
    assert_eq!(array.len(), 2);
    assert_eq!(array[0]["id"], "a");
    assert_eq!(array[1]["outcome"], "satisfied");
}

#[test]
fn the_legacy_rule_field_lists_only_what_actually_fired() {
    // The spool's `rule` field is what live dashboards group on, so it survives
    // this change unchanged in meaning: the ids that were UNSATISFIED, sorted
    // and deduplicated so the same set always renders the same string.
    let field = legacy_rule_field(&[
        unsatisfied("zeta"),
        ConstraintEvaluation::new("alpha", Outcome::Satisfied, Response::Logged),
        unsatisfied("alpha"),
        unsatisfied("zeta"),
    ]);
    assert_eq!(field.as_deref(), Some("alpha+zeta"));
}

#[test]
fn nothing_fired_means_no_legacy_rule_field_at_all() {
    // Absent, not empty-string: a reader distinguishing "no rule fired" from
    // "the field was blank" would otherwise have to consult the host's config.
    assert!(legacy_rule_field(&[]).is_none());
    assert!(
        legacy_rule_field(&[ConstraintEvaluation::new(
            "a",
            Outcome::Satisfied,
            Response::Logged
        )])
        .is_none(),
        "a satisfied constraint did not fire and must not be named as a rule"
    );
}
