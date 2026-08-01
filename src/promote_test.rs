//! Tests for `promote` — SHACL validation, chunked writes, and the
//! all-or-nothing refusal contract. Child module of `promote` (`super::*`
//! reaches its private helpers); size-exempt (`_test.rs`).

use super::*;

const SHAPES: &str = CODE_EDGE_SHAPES;

// The round-trip that pins the exporter to the shapes (#13/#14): a REAL
// `export::to_turtle` projection of a real repo must SHACL-validate against
// the shipped shapes. The hand-written CONFORMING fixture only claims to
// mirror the emitter; this proves it, and catches emitter/shape drift (a
// new predicate, a dropped required field) at the exporter rather than as a
// Quipu refusal in production.
#[test]
fn a_real_export_projection_validates_against_the_shipped_shapes() {
    let dir = tempfile::tempdir().unwrap();
    // A repo exercising every emitted edge kind the shapes gate: a call
    // (mid→leaf), an import (b uses a), and a doc Section referencing a
    // symbol.
    std::fs::write(dir.path().join("a.rs"), "pub fn leaf() {}\n").unwrap();
    std::fs::write(
        dir.path().join("b.rs"),
        "use crate::a::leaf;\nfn mid() { leaf(); }\n",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("README.md"),
        "# Guide\n\nThe `leaf` function is the entry point.\n",
    )
    .unwrap();

    let turtle = crate::export::to_turtle(dir.path(), "demo").expect("export ran");
    assert!(
        turtle.contains("bobbin:calls"),
        "fixture must emit a call edge"
    );
    let v = validate(&turtle, SHAPES).expect("validation ran");
    assert!(
        v.conforms,
        "real export output must validate against the shipped shapes; violations: {:?}",
        v.violations
    );
}

#[test]
fn empty_bearer_token_is_unset_not_a_credential() {
    // An empty env value must behave like no token at all — sending
    // `Bearer ` would present a wrong credential and 401 confusingly.
    assert_eq!(normalize_token(None), None);
    assert_eq!(normalize_token(Some(String::new())), None);
    assert_eq!(
        normalize_token(Some("sekrit".into())),
        Some("sekrit".to_string())
    );
}

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
