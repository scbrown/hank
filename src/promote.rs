//! Phase-4 promotion: validate a Turtle projection in-process, then write it to
//! Quipu (#14 FR-20, #15 FR-19/21/22). Gated behind the `quipu` feature.
//!
//! THE ORDER IS THE CONTRACT. `export::to_turtle` produces the facts; this module
//! SHACL-validates them against the shipped shapes BEFORE any write, and refuses
//! the whole promotion on a single violation (all-or-nothing per commit, §6.3).
//! Validation is always whole-graph. The WRITE is chunked when the payload would
//! exceed Quipu's request-body limit (axum defaults to 2 MiB and the deployed
//! server sets no override — a 2.28 MB projection of the quipu repo itself came
//! back 413, aegis-hbiw): entity blocks are split across multiple `/knot` posts,
//! each under the limit, prefixes replicated. A chunked write is NOT atomic
//! across chunks — if chunk k fails, chunks 0..k are landed — but every IRI is
//! deterministic and `/knot` supersedes, so a re-run converges to the same graph
//! rather than duplicating. The failure message names exactly what landed.
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
pub fn write_knot(endpoint: &str, turtle: &str, source: &str) -> Result<KnotResult> {
    let url = format!("{}/knot", endpoint.trim_end_matches('/'));
    // Provenance on every write (promotion tail item 4): quipu records actor +
    // source per transaction; an anonymous writer is unauditable, and hank was
    // the only anonymous one left.
    let body =
        serde_json::json!({ "turtle": turtle, "actor": "hank", "source": source }).to_string();

    // Quipu is known to flap (transient 503 "no available server", recovering in
    // seconds). Ride through TRANSIENT failures — 5xx and transport errors — with
    // a short backoff; a 4xx is a real answer and fails immediately. The
    // all-or-nothing guarantee is unaffected: every attempt is the same full
    // idempotent write, and exhausting retries still fails loud, never partial.
    const ATTEMPTS: u32 = 3;
    let mut resp = None;
    let mut last_err = String::new();
    for attempt in 1..=ATTEMPTS {
        match ureq::post(&url)
            .set("Content-Type", "application/json")
            .send_string(&body)
        {
            Ok(r) => {
                resp = Some(r);
                break;
            }
            Err(ureq::Error::Status(code, _)) if code < 500 => {
                return Err(Error::Promote(format!("POST {url} failed: status {code}")));
            }
            Err(e) => {
                last_err = e.to_string();
                if attempt < ATTEMPTS {
                    std::thread::sleep(std::time::Duration::from_secs(2 * u64::from(attempt)));
                }
            }
        }
    }
    let resp = resp.ok_or_else(|| {
        Error::Promote(format!(
            "POST {url} failed after {ATTEMPTS} attempts (transient errors retried): {last_err}"
        ))
    })?;

    let text = resp
        .into_string()
        .map_err(|e| Error::Promote(format!("could not read /knot response: {e}")))?;
    // Quipu can REFUSE the write server-side: its persistent shape registry,
    // when loaded, validates independently of hank's in-process gate, and a
    // shape the server holds that hank's copy lacks surfaces HERE as HTTP 200
    // with conforms:false (seen live: a stored symbolKind maxCount(1) refused
    // a projection hank's shapes accepted). That is a real refusal and must
    // read as one — not as a JSON parse error on a missing `count` field.
    if let Ok(refusal) = serde_json::from_str::<KnotRefusal>(&text) {
        if !refusal.conforms {
            let issues = refusal
                .issues
                .iter()
                .map(|i| format!("{} {} on {}", i.component, i.message, i.focus_node))
                .collect::<Vec<_>>()
                .join("; ");
            return Err(Error::Promote(format!(
                "quipu refused the write (server-side SHACL, {} violation(s)): {issues}. \
                 hank's own shapes ACCEPTED this projection — the two shape sets have \
                 drifted; reconcile shapes/code-edges.ttl with quipu's stored registry.",
                refusal.violations
            )));
        }
    }
    let parsed: KnotResult = serde_json::from_str(&text)
        .map_err(|e| Error::Promote(format!("unexpected /knot response {text:?}: {e}")))?;
    Ok(parsed)
}

/// Quipu's `/knot` refusal shape (HTTP 200, `conforms:false`).
#[derive(Debug, serde::Deserialize)]
struct KnotRefusal {
    conforms: bool,
    #[serde(default)]
    violations: u64,
    #[serde(default)]
    issues: Vec<KnotIssue>,
}

#[derive(Debug, serde::Deserialize)]
struct KnotIssue {
    #[serde(default)]
    component: String,
    #[serde(default)]
    message: String,
    #[serde(default)]
    focus_node: String,
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

/// Stay safely under axum's 2 MiB default body limit: the Turtle is JSON-string
/// encoded (quotes/newlines escape to two bytes) before it travels, so leave
/// headroom for that inflation plus the JSON envelope.
const CHUNK_LIMIT: usize = 1_500_000;

/// Split a Turtle document into chunks of whole statements, each chunk carrying
/// the prefix header. `to_turtle` separates entity blocks with blank lines and
/// never emits blank nodes, so a blank line is always a safe split point — but
/// the call/reference EDGE sections are contiguous single-line statements with
/// no blank lines between them (bobbin's edge section alone is ~6.9 MB), so an
/// oversized block is split further at statement boundaries: a line ending in
/// `.` completes a Turtle statement in this exporter's output.
///
/// Errors only if a single STATEMENT exceeds `limit` — that genuinely cannot be
/// split, and silently posting it would just 413 downstream.
fn chunk_turtle(turtle: &str, limit: usize) -> Result<Vec<String>> {
    if turtle.len() <= limit {
        return Ok(vec![turtle.to_string()]);
    }
    let mut blocks = turtle.split("\n\n");
    // The first "block" is the @prefix header `to_turtle` puts at the top.
    let header = blocks.next().unwrap_or_default();
    let mut chunks = Vec::new();
    let mut cur = String::from(header);

    // Append one statement-complete piece, starting a new chunk when full.
    let mut push_piece = |cur: &mut String, chunks: &mut Vec<String>, piece: &str| -> Result<()> {
        if header.len() + 2 + piece.len() > limit {
            return Err(Error::Promote(format!(
                "a single Turtle statement is {} bytes, over the {limit} byte chunk limit — cannot split below statement granularity",
                piece.len()
            )));
        }
        if cur.len() + 2 + piece.len() > limit {
            chunks.push(std::mem::replace(cur, String::from(header)));
        }
        cur.push_str("\n\n");
        cur.push_str(piece);
        Ok(())
    };

    for block in blocks {
        if block.trim().is_empty() {
            continue;
        }
        if header.len() + 2 + block.len() <= limit {
            push_piece(&mut cur, &mut chunks, block)?;
            continue;
        }
        // Oversized block: regroup its lines into statement-complete pieces
        // (a line ending in `.` closes a statement in to_turtle's output).
        let mut piece = String::new();
        for line in block.lines() {
            if !piece.is_empty() {
                piece.push('\n');
            }
            piece.push_str(line);
            if line.trim_end().ends_with('.') && header.len() + 2 + piece.len() > limit / 2 {
                push_piece(&mut cur, &mut chunks, &piece)?;
                piece.clear();
            }
        }
        if !piece.trim().is_empty() {
            push_piece(&mut cur, &mut chunks, &piece)?;
        }
    }
    if cur.len() > header.len() {
        chunks.push(cur);
    }
    Ok(chunks)
}

/// The aggregated result of a (possibly chunked) promotion write.
#[derive(Debug, Clone)]
pub struct WriteSummary {
    /// Sum of the per-chunk `count` fields — the idempotence signal (a re-run
    /// returns the same total, not a larger one).
    pub count: u64,
    /// Every transaction id Quipu returned, in write order.
    pub tx_ids: Vec<u64>,
    /// How many `/knot` posts the write took (1 = the classic single-post path).
    pub chunks: usize,
}

/// The full promotion: validate the WHOLE graph, then write iff it conforms —
/// in one `/knot` post when it fits, in idempotent chunks when it would 413.
/// On non-conformance it writes NOTHING and returns the violations.
pub fn promote(endpoint: &str, turtle: &str, source: &str) -> Result<Promotion> {
    let v = validate(turtle, CODE_EDGE_SHAPES)?;
    if !v.conforms {
        return Ok(Promotion::Refused(v.violations));
    }
    let chunks = chunk_turtle(turtle, CHUNK_LIMIT)?;
    let total = chunks.len();
    let mut summary = WriteSummary {
        count: 0,
        tx_ids: Vec::new(),
        chunks: total,
    };
    for (i, chunk) in chunks.iter().enumerate() {
        let knot = write_knot(endpoint, chunk, source).map_err(|e| {
            Error::Promote(format!(
                "chunk {}/{total} failed after {} chunk(s) landed — re-running is safe (deterministic IRIs supersede): {e}",
                i + 1,
                i
            ))
        })?;
        summary.count += knot.count;
        if let Some(t) = knot.tx_id {
            summary.tx_ids.push(t);
        }
    }
    Ok(Promotion::Wrote(summary))
}

/// The result of a full promotion: it either wrote, or refused whole.
#[derive(Debug)]
pub enum Promotion {
    /// Validated and written; carries the aggregated write result.
    Wrote(WriteSummary),
    /// Did not pass SHACL; carries the violations and wrote nothing.
    Refused(Vec<String>),
}

impl Promotion {
    /// Render for a human, and set the process exit intent: a refusal is exit-2
    /// (could-not-promote), never a silent success.
    pub fn report(&self, w: &mut impl Write) -> std::io::Result<bool> {
        match self {
            Promotion::Wrote(k) => {
                let txs = match k.tx_ids.as_slice() {
                    [] => String::new(),
                    [one] => format!(" (tx {one})"),
                    [first, .., last] => format!(" (tx {first}..{last})"),
                };
                let chunked = if k.chunks > 1 {
                    format!(" in {} chunks", k.chunks)
                } else {
                    String::new()
                };
                writeln!(w, "  promoted: {} triples present{txs}{chunked}", k.count)?;
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
    // The conforming fixture mirrors what the emitter ACTUALLY produces — a
    // symbol carries name + definedIn, and its module carries filePath + repo +
    // language — because the synced node shapes (quipu's registry) now require
    // them. The old label-and-tier-only symbol predates the sync and fails
    // MinCount x2: a "conforming" fixture thinner than any real emission tests
    // a projection hank never writes.
    const CONFORMING: &str = r#"
@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
@prefix bobbin: <http://aegis.gastown.local/ontology/> .
bobbin:code_mod a bobbin:CodeModule ;
  rdfs:label "m.rs" ; bobbin:filePath "m.rs" ;
  bobbin:repo "fixture" ; bobbin:language "rust" .
bobbin:code_x a bobbin:CodeSymbol ;
  rdfs:label "x" ; bobbin:name "x" ; bobbin:hasTier "lsp" ;
  bobbin:definedIn bobbin:code_mod ;
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
        let out = promote("http://127.0.0.1:1", VIOLATING, "test").expect("no write attempted");
        match out {
            Promotion::Refused(vs) => assert!(!vs.is_empty()),
            Promotion::Wrote(_) => panic!("wrote invalid facts to Quipu"),
        }
    }

    /// Build a synthetic Turtle doc in `to_turtle`'s shape: prefix header, then
    /// entity blocks separated by blank lines.
    fn synthetic_turtle(blocks: usize, block_bytes: usize) -> String {
        let header = "@prefix bobbin: <http://aegis.gastown.local/ontology/> .";
        let mut t = String::from(header);
        for i in 0..blocks {
            let pad = "x".repeat(block_bytes.saturating_sub(60));
            t.push_str(&format!(
                "\n\nbobbin:code_{i} a bobbin:CodeSymbol ;\n  rdfs:label \"{pad}\" ."
            ));
        }
        t
    }

    #[test]
    fn under_limit_turtle_is_a_single_untouched_chunk() {
        let t = synthetic_turtle(3, 100);
        let chunks = chunk_turtle(&t, 1_000_000).expect("chunked");
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0], t, "single-chunk path must be byte-identical");
    }

    #[test]
    fn oversized_turtle_splits_on_block_boundaries_preserving_every_block() {
        let t = synthetic_turtle(40, 300);
        let chunks = chunk_turtle(&t, 2_000).expect("chunked");
        assert!(chunks.len() > 1, "expected a real split");
        for c in &chunks {
            assert!(c.len() <= 2_000, "chunk over limit: {} bytes", c.len());
            assert!(
                c.starts_with("@prefix bobbin:"),
                "every chunk must carry the prefix header"
            );
        }
        // Every block appears exactly once across all chunks, in order.
        let stitched: Vec<&str> = chunks
            .iter()
            .flat_map(|c| c.split("\n\n").skip(1))
            .collect();
        let original: Vec<&str> = t.split("\n\n").skip(1).collect();
        assert_eq!(stitched, original, "blocks lost, duplicated, or reordered");
    }

    /// The edge sections have NO blank lines — thousands of one-line statements
    /// in a single "block" (bobbin's is ~6.9 MB). They must chunk at statement
    /// boundaries, never error, and lose nothing.
    #[test]
    fn a_contiguous_edge_section_chunks_at_statement_boundaries() {
        let header = "@prefix bobbin: <http://aegis.gastown.local/ontology/> .";
        let mut t = String::from(header);
        t.push_str("\n\n");
        let edges: Vec<String> = (0..200)
            .map(|i| format!("<http://x/a{i}> bobbin:calls <http://x/b{i}> ."))
            .collect();
        t.push_str(&edges.join("\n"));
        let chunks = chunk_turtle(&t, 2_000).expect("edge section must chunk, not error");
        assert!(chunks.len() > 1, "expected a real split");
        let stitched: Vec<String> = chunks
            .iter()
            .flat_map(|c| c.lines())
            .filter(|l| l.contains("bobbin:calls"))
            .map(str::to_string)
            .collect();
        assert_eq!(
            stitched, edges,
            "edge statements lost, duplicated, or reordered"
        );
        for c in &chunks {
            assert!(c.len() <= 2_000, "chunk over limit: {} bytes", c.len());
            assert!(c.starts_with("@prefix"), "chunk missing prefix header");
        }
    }

    #[test]
    fn a_block_bigger_than_the_limit_errors_loudly() {
        let t = synthetic_turtle(2, 5_000);
        let err = chunk_turtle(&t, 1_000).expect_err("unsplittable block must error");
        let msg = format!("{err}");
        assert!(
            msg.contains("cannot split below statement granularity"),
            "error must name the cause, got: {msg}"
        );
    }

    #[test]
    fn multi_chunk_report_names_the_chunk_count() {
        let wrote = Promotion::Wrote(WriteSummary {
            count: 9329,
            tx_ids: vec![801, 802, 803],
            chunks: 3,
        });
        let mut out = Vec::new();
        wrote.report(&mut out).unwrap();
        let s = String::from_utf8(out).unwrap();
        assert!(s.contains("9329 triples"), "{s}");
        assert!(s.contains("tx 801..803"), "{s}");
        assert!(s.contains("in 3 chunks"), "{s}");
    }
}
