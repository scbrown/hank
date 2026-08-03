//! The Σ-derived trace record — what an action tells an auditor afterwards.
//!
//! SARC (arXiv:2605.07728) invariant I3 requires that every execution emit a
//! structured trace **whose schema is derived from the specification**, holding
//! the pre-state, the action, the post-state, *the constraints evaluated and
//! their outcomes*, and an attribution tuple. I8 then makes the audit decidable:
//! given Σ and a trace T, check `T ⊨ Σ` mechanically. The check needs four
//! things per action — which constraints applied, where each was evaluated, what
//! outcome it produced, and what response was taken — and none of them can be
//! recovered from a record that says only which rules fired.
//!
//! That was the shape of the spool before this module: `rule` was a `+`-joined
//! string of names. It answers "what fired" and cannot answer "was this
//! evaluated at a point compatible with its class", which is exactly pass (ii)
//! of the audit checker. Two rules that fired identically but at different
//! points produced byte-identical records.
//!
//! ## What is honestly recordable today
//!
//! [`ConstraintEvaluation`] is the full `E_i` for one constraint. The
//! attribution tuple `α` is deliberately PARTIAL: SARC §9.6 wants
//! `⟨P, planner, executor, tool, auth, C_eval⟩`, and hank can supply `tool`,
//! `executor` and `C_eval` honestly today. The principal-and-agent chain `P` and
//! the intersected authority `auth` need multi-tenancy quipu does not yet have
//! (`group_id` is provenance-only), so they are ABSENT from the record rather
//! than filled with the single agent id — a one-element chain asserted where a
//! real chain belongs would read as "this action had one principal" to exactly
//! the auditor the field exists for.

use serde::{Deserialize, Serialize};

use crate::constraint::{ConstraintClass, VerificationPoint};

/// What a constraint evaluation concluded. Mirrors quipu's `aegis:outcome`
/// enum, and keeps SARC's distinction between the two ways a constraint fails
/// to be satisfied.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Outcome {
    /// The constraint holds.
    Satisfied,
    /// The evidence says it does not hold.
    Unsatisfied,
    /// There was no evidence to decide on. NOT a soft `unsatisfied`: a
    /// constraint that could not be evaluated has told you nothing, and
    /// collapsing the two is how an unevaluated check reads as a passing one.
    Unknown,
}

/// What the guard actually did about a fired constraint.
///
/// Recorded separately from [`Outcome`] because they answer different audit
/// questions — the outcome is what the predicate concluded, the response is what
/// the runtime did with it — and because pass (iii) of the audit checker is
/// precisely "does the recorded response match the one the policy declared".
/// Collapsing them would make that check unwritable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Response {
    /// The action was refused.
    Blocked,
    /// The violation was reported to the model without blocking (advise mode,
    /// or a soft constraint).
    Warned,
    /// Recorded only; the model was told nothing.
    Logged,
    /// Routed to a human. Not reachable until the escalation router lands;
    /// present so the vocabulary does not have to change when it does.
    Escalated,
    /// The constraint fired and the runtime did nothing about it. Distinct from
    /// `Logged`: this is the state to grep for, not one to design toward.
    NoAction,
}

/// One constraint's evaluation, for one action — SARC's `E_i` element.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConstraintEvaluation {
    /// The constraint's stable id — the same name its verdict cites.
    pub id: String,
    /// The declared class, when the constraint declared one. `None` for a
    /// locally-configured rule or a policy projected from a pre-Phase-1 catalog;
    /// absent rather than guessed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub class: Option<ConstraintClass>,
    /// Where it was evaluated. Absent when undeclared, for the same reason.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verification_point: Option<VerificationPoint>,
    /// What the predicate concluded.
    pub outcome: Outcome,
    /// What the runtime did about it.
    pub response: Response,
}

impl ConstraintEvaluation {
    /// An evaluation of `id` that concluded `outcome` and drew `response`.
    pub fn new(id: impl Into<String>, outcome: Outcome, response: Response) -> Self {
        Self {
            id: id.into(),
            class: None,
            verification_point: None,
            outcome,
            response,
        }
    }

    /// Attach the declared class and point, when the constraint carried them.
    #[must_use]
    pub fn placed(
        mut self,
        class: Option<ConstraintClass>,
        verification_point: Option<VerificationPoint>,
    ) -> Self {
        self.class = class;
        self.verification_point = verification_point;
        self
    }
}

/// Render a set of evaluations as the spool's `constraints` field.
///
/// Serialised as one JSON array rather than flattened into sibling keys so a
/// reader never has to reassemble which class went with which id — the
/// reassembly-by-position bug that a `+`-joined `rule` string plus a separate
/// count would invite.
#[must_use]
pub fn to_json(evaluations: &[ConstraintEvaluation]) -> serde_json::Value {
    serde_json::to_value(evaluations).unwrap_or(serde_json::Value::Null)
}

/// The `+`-joined rule id string the spool carried before this module.
///
/// Retained because a live spool reader and the existing soak dashboards group
/// on it: dropping the field would silently empty every panel built on it, and
/// the migration is a separate change from the one that adds the structure.
/// Sorted and deduplicated, so the same set of rules always renders the same
/// string.
#[must_use]
pub fn legacy_rule_field(evaluations: &[ConstraintEvaluation]) -> Option<String> {
    let mut ids: Vec<&str> = evaluations
        .iter()
        .filter(|e| e.outcome == Outcome::Unsatisfied)
        .map(|e| e.id.as_str())
        .collect();
    if ids.is_empty() {
        return None;
    }
    ids.sort_unstable();
    ids.dedup();
    Some(ids.join("+"))
}

#[cfg(test)]
#[path = "trace_test.rs"]
mod tests;
