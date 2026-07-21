//! Phase-4 promotion: validate a Turtle projection in-process, then write it to
//! Quipu (#14 FR-20, #15 FR-19/21/22). Gated behind the `quipu` feature.
//!
//! THE ORDER IS THE CONTRACT. `export::to_turtle` produces the facts; this module
//! SHACL-validates them against the shipped shapes BEFORE any write, and refuses
//! the whole promotion on a single violation (all-or-nothing per commit, §6.3).
//! There are no partial writes: a promotion either validates and lands whole, or
//! is refused whole with the specific violations.
//!
//! WHY IN-PROCESS VALIDATION, NOT QUIPU'S. Quipu exposes `/validate`, and it works.
//! But validating against the same server you are about to write to proves only
//! that the server agrees with itself. FR-20 wants hank to hold its own copy of the
//! shapes and check independently, so a shape drift between hank and Quipu is caught
//! at hank rather than discovered as bad data already in the graph. rudof_lib is
//! that independent checker; `scripts/check-code-shapes.sh` is the cross-check that
//! the two engines still agree.
//!
//! WHY `/knot` OVER HTTP, NOT THE `quipu` CRATE. FR-21 names three promotion
//! surfaces — `quipu_knot` (MCP) / `POST /knot` (REST) / `Store::transact`
//! (in-process). The REST surface needs no `quipu` crate dependency (still
//! rev-unpinned, Cargo.toml), and hank explicitly does NOT stand up its own triple
//! store (§14.4). So promotion is an HTTP POST of validated Turtle. `/knot` is
//! bitemporal: a re-promotion of the same facts supersedes rather than duplicating,
//! which is why re-running is idempotent BY TRIPLE COUNT, not by write count.

use std::io::Write;

use crate::errors::{Error, Result};

/// The code-edge SHACL shapes hank ships and validates against. Compiled in so a
/// promotion can never run against shapes that drift from the binary — the file on
/// disk is for humans and `check-code-shapes.sh`; THIS is what actually gates a
/// write.
pub const CODE_EDGE_SHAPES: &str = include_str!("../shapes/code-edges.ttl");

/// The outcome of validating a Turtle projection against the code shapes.
#[derive(Debug, Clone)]
pub struct Validation {
    /// Did the projection satisfy every shape?
    pub conforms: bool,
    /// Human-readable violation messages, empty iff `conforms`.
    pub violations: Vec<String>,
}

/// SHACL-validate `data_ttl` against `shapes_ttl`, in-process, via `rudof_lib`.
///
/// Returns the conformance verdict and, when it does not conform, the specific
/// violations. A parse failure of either input is itself a non-conformance we can
/// name, never a silent pass — an unparseable projection must not reach Quipu.
pub fn validate(data_ttl: &str, shapes_ttl: &str) -> Result<Validation> {
    use rudof_lib::formats::{DataFormat, InputSpec, ResultShaclValidationFormat, ShaclFormat};
    use rudof_lib::{Rudof, RudofConfig};

    let mut rudof = Rudof::new(RudofConfig::default());

    rudof
        .load_data()
        .with_data(&[InputSpec::str(data_ttl)])
        .with_data_format(&DataFormat::Turtle)
        .execute()
        .map_err(|e| Error::Promote(format!("promotion data is not valid Turtle: {e}")))?;

    rudof
        .load_shacl_shapes()
        .with_shacl_schema(&InputSpec::str(shapes_ttl))
        .with_shacl_schema_format(&ShaclFormat::Turtle)
        .execute()
        .map_err(|e| Error::Promote(format!("SHACL shapes did not load: {e}")))?;

    rudof
        .validate_shacl()
        .execute()
        .map_err(|e| Error::Promote(format!("SHACL validation failed to run: {e}")))?;

    // The report lives in rudof's private state; serialize it to Turtle and read
    // `sh:conforms` / `sh:resultMessage` out. This is the only exposed path to the
    // verdict — there is no public `conforms()` accessor on Rudof.
    let mut buf: Vec<u8> = Vec::new();
    rudof
        .serialize_shacl_validation_results(&mut buf)
        .with_result_shacl_validation_format(&ResultShaclValidationFormat::Turtle)
        .execute()
        .map_err(|e| Error::Promote(format!("could not read validation report: {e}")))?;
    let report = String::from_utf8_lossy(&buf);

    Ok(parse_report(&report))
}

/// Read `sh:conforms` and any `sh:resultMessage`s out of a SHACL report in Turtle.
fn parse_report(report: &str) -> Validation {
    let conforms = report.contains("sh:conforms true") || report.contains("sh:conforms  true");
    let mut violations = Vec::new();
    for line in report.lines() {
        let l = line.trim();
        if let Some(rest) = l.strip_prefix("sh:resultMessage") {
            // sh:resultMessage "MinCount(1) not satisfied" ;
            let msg = rest.trim().trim_end_matches(';').trim().trim_matches('"');
            if !msg.is_empty() {
                violations.push(msg.to_string());
            }
        }
    }
    // Belt and braces: a report that does not conform but whose messages we failed
    // to parse must still be non-empty, or a caller could read "conforms=false,
    // violations=[]" as "nothing wrong". A refusal must always carry a reason.
    if !conforms && violations.is_empty() {
        violations.push("SHACL validation reported non-conformance (see report)".to_string());
    }
    Validation {
        conforms,
        violations,
    }
}

/// Post validated Turtle to Quipu's `/knot`. Returns the number of triples the
/// transaction reports as present for these facts — the count that makes
/// idempotence checkable (a re-promotion returns the same count, not a larger one).
///
/// `endpoint` is the Quipu base URL (e.g. from `--to` / config); this appends
/// `/knot`. NEVER defaulted to a hardcoded host — a promotion that silently picks a
/// graph is how facts land in the wrong one.
pub fn write_knot(endpoint: &str, turtle: &str) -> Result<KnotResult> {
    let url = format!("{}/knot", endpoint.trim_end_matches('/'));
    let body = serde_json::json!({ "turtle": turtle }).to_string();

    let resp = ureq::post(&url)
        .set("Content-Type", "application/json")
        .send_string(&body)
        .map_err(|e| Error::Promote(format!("POST {url} failed: {e}")))?;

    let text = resp
        .into_string()
        .map_err(|e| Error::Promote(format!("could not read /knot response: {e}")))?;
    let parsed: KnotResult = serde_json::from_str(&text)
        .map_err(|e| Error::Promote(format!("unexpected /knot response {text:?}: {e}")))?;
    Ok(parsed)
}

/// Quipu `/knot` response. `conforms` here is Quipu's OWN field and is NOT the
/// validation gate — Quipu's persistent shape registry may be empty, in which case
/// it reports `conforms:true` for anything. hank's gate is [`validate`] above,
/// which ran before this. `count` is the load-bearing field for idempotence.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct KnotResult {
    /// Triples present for these facts after the write — the idempotence signal.
    pub count: u64,
    /// Quipu's monotonic transaction id, when returned.
    #[serde(default)]
    pub tx_id: Option<u64>,
}

/// The full promotion: validate, then write iff it conforms. On non-conformance it
/// writes NOTHING and returns the violations — the all-or-nothing guarantee.
pub fn promote(endpoint: &str, turtle: &str) -> Result<Promotion> {
    let v = validate(turtle, CODE_EDGE_SHAPES)?;
    if !v.conforms {
        return Ok(Promotion::Refused(v.violations));
    }
    let knot = write_knot(endpoint, turtle)?;
    Ok(Promotion::Wrote(knot))
}

/// The result of a full promotion: it either wrote, or refused whole.
#[derive(Debug)]
pub enum Promotion {
    /// Validated and written; carries Quipu's transaction result.
    Wrote(KnotResult),
    /// Did not pass SHACL; carries the violations and wrote nothing.
    Refused(Vec<String>),
}

impl Promotion {
    /// Render for a human, and set the process exit intent: a refusal is exit-2
    /// (could-not-promote), never a silent success.
    pub fn report(&self, w: &mut impl Write) -> std::io::Result<bool> {
        match self {
            Promotion::Wrote(k) => {
                writeln!(
                    w,
                    "  promoted: {} triples present{}",
                    k.count,
                    k.tx_id.map(|t| format!(" (tx {t})")).unwrap_or_default()
                )?;
                Ok(true)
            }
            Promotion::Refused(vs) => {
                writeln!(
                    w,
                    "  REFUSED — promotion did not pass SHACL, wrote nothing:"
                )?;
                for v in vs {
                    writeln!(w, "    - {v}")?;
                }
                Ok(false)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHAPES: &str = CODE_EDGE_SHAPES;

    // A promotion whose shape is correct: an IRI-valued `calls`, a known tier.
    const CONFORMING: &str = r#"
@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
@prefix bobbin: <http://aegis.gastown.local/ontology/> .
bobbin:code_x a bobbin:CodeSymbol ;
  rdfs:label "x" ; bobbin:hasTier "lsp" ;
  bobbin:calls bobbin:code_y .
"#;

    // Two violations: `calls` points at a literal (must be an IRI); tier is bogus.
    const VIOLATING: &str = r#"
@prefix bobbin: <http://aegis.gastown.local/ontology/> .
bobbin:code_bad a bobbin:CodeSymbol ;
  bobbin:calls "not-an-iri" ;
  bobbin:hasTier "vibes" .
"#;

    #[test]
    fn conforming_projection_validates() {
        let v = validate(CONFORMING, SHAPES).expect("validation ran");
        assert!(v.conforms, "expected conformance, got {:?}", v.violations);
        assert!(v.violations.is_empty());
    }

    #[test]
    fn violating_projection_is_refused_with_reasons() {
        let v = validate(VIOLATING, SHAPES).expect("validation ran");
        assert!(!v.conforms, "a malformed projection must not conform");
        assert!(
            !v.violations.is_empty(),
            "a refusal must always carry at least one reason"
        );
    }

    #[test]
    fn a_refusal_never_reads_as_empty_success() {
        // The specific bug this guards: conforms=false with no messages reads to a
        // caller as "nothing wrong". parse_report must never produce that.
        let empty_nonconformance = parse_report("[] a sh:ValidationReport ; sh:conforms false .");
        assert!(!empty_nonconformance.conforms);
        assert!(!empty_nonconformance.violations.is_empty());
    }

    #[test]
    fn promote_refuses_without_writing_when_invalid() {
        // endpoint is deliberately unreachable; a valid refusal must return BEFORE
        // any network call, so this must not error on the bad endpoint.
        let out = promote("http://127.0.0.1:1", VIOLATING).expect("no write attempted");
        match out {
            Promotion::Refused(vs) => assert!(!vs.is_empty()),
            Promotion::Wrote(_) => panic!("wrote invalid facts to Quipu"),
        }
    }
}
