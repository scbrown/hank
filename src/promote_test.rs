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

// The IRI-collision shape, in the form the emitter really produces it: ONE
// symbol IRI carrying two distinct `symbolKind` values, which is what
// `CodeSymbolShape`'s maxCount exists to catch. Taken from a real refusal —
// two mutually-exclusive `#[cfg]` declarations of one name, which the extractor
// sees both of (aegis-4ba2e). Kept here because this is the projection whose
// refusal used to be unreadable.
const COLLIDING: &str = r#"
@prefix bobbin: <http://aegis.gastown.local/ontology/> .
bobbin:code_mod a bobbin:CodeModule ;
  bobbin:filePath "m.rs" ; bobbin:repo "fixture" ; bobbin:language "rust" .
bobbin:code_dup a bobbin:CodeSymbol ;
  bobbin:name "Dup" ; bobbin:definedIn bobbin:code_mod ;
  bobbin:symbolKind "struct" .
bobbin:code_dup a bobbin:CodeSymbol ;
  bobbin:name "Dup" ; bobbin:definedIn bobbin:code_mod ;
  bobbin:symbolKind "type_alias" .
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
    // A distinctive source keeps this test's dump off any other test's name.
    let out = promote(
        "http://127.0.0.1:1",
        VIOLATING,
        "promote-refuses-without-writing-fixture",
    )
    .expect("no write attempted");
    match out {
        Promotion::Refused {
            violations,
            payload,
        } => {
            assert!(!violations.is_empty());
            // The refusal must leave the document behind: without it the reader
            // has a constraint name and no way to find what tripped it.
            let p = payload.expect("a refused promotion must retain its payload");
            assert_eq!(
                std::fs::read_to_string(&p).expect("dump is readable"),
                VIOLATING,
                "the retained payload must be the exact projection that was refused"
            );
            std::fs::remove_file(&p).ok();
        }
        Promotion::Wrote(_) => panic!("wrote invalid facts to Quipu"),
    }
}

#[test]
fn a_retained_payload_is_byte_identical_and_overwrites_in_place() {
    // Two failures of the same promotion must not accumulate files — a
    // diagnostic that grows without bound is its own outage — and the retained
    // bytes must be the projection itself, not a summary of it.
    let dir = tempfile::tempdir().unwrap();
    let first = dump_payload_to(dir.path(), CONFORMING, "hank promote demo@abc (cli)")
        .expect("dump written");
    let second = dump_payload_to(dir.path(), VIOLATING, "hank promote demo@abc (cli)")
        .expect("dump written");
    assert_eq!(first, second, "the same source must reuse one dump path");
    assert_eq!(std::fs::read_to_string(&second).unwrap(), VIOLATING);
    assert!(second.starts_with(dir.path()), "dump escaped its directory");
    let files: Vec<_> = std::fs::read_dir(dir.path()).unwrap().collect();
    assert_eq!(files.len(), 1, "a re-run must overwrite, not accumulate");
}

#[test]
fn a_refused_promotion_names_the_offending_node_and_property() {
    // The regression this pins (aegis-o8rq8): violations used to read only
    // "MaxCount(1) not satisfied" — true of every maxCount shape in the file and
    // identifying nothing. A scheduled promotion refused for a day on one
    // symbol and the log never named it. Focus node and path must survive.
    let v = validate(COLLIDING, SHAPES).expect("validation ran");
    assert!(!v.conforms, "two symbolKinds on one IRI must be refused");
    let joined = v.violations.join(" | ");
    assert!(
        joined.contains("code_dup"),
        "a violation must name its focus node; got {joined:?}"
    );
    assert!(
        joined.contains("symbolKind"),
        "a violation must name the property path; got {joined:?}"
    );
}

#[test]
fn a_violation_message_does_not_leak_turtle_punctuation() {
    // Measured in production logs: `MaxCount(1) not satisfied" .` — the message
    // was the LAST property of its block, so it ended `.` not `;` and the old
    // `trim_end_matches(';')` left the quote and terminator in the text.
    let report = r#"
[] a sh:ValidationReport ;
   sh:conforms false ;
   sh:result [ a sh:ValidationResult ;
     sh:focusNode <http://example.invalid/n> ;
     sh:resultPath <http://example.invalid/p> ;
     sh:resultMessage "MaxCount(1) not satisfied" ] .
"#;
    let v = parse_report(report);
    assert!(!v.conforms);
    assert_eq!(v.violations.len(), 1);
    let msg = &v.violations[0];
    assert!(
        msg.starts_with("MaxCount(1) not satisfied"),
        "punctuation leaked into the message: {msg:?}"
    );
    assert!(!msg.contains('"'), "quote leaked into the message: {msg:?}");
    assert!(msg.contains("http://example.invalid/n"), "{msg:?}");
    assert!(msg.contains("http://example.invalid/p"), "{msg:?}");
}

#[test]
fn one_results_focus_node_never_bleeds_into_the_next() {
    // A result carrying no focusNode must not inherit the previous result's and
    // blame the wrong node — a misattributed violation is worse than a bare one.
    let report = r#"
[] a sh:ValidationReport ;
   sh:conforms false ;
   sh:result [ a sh:ValidationResult ;
     sh:focusNode <http://example.invalid/first> ;
     sh:resultMessage "First broke" ] ;
   sh:result [ a sh:ValidationResult ;
     sh:resultMessage "Second broke" ] .
"#;
    let v = parse_report(report);
    assert_eq!(v.violations.len(), 2);
    assert!(v.violations[0].contains("first"), "{:?}", v.violations);
    assert!(
        !v.violations[1].contains("first"),
        "focus node bled into the next result: {:?}",
        v.violations
    );
}

#[test]
fn a_dump_slug_cannot_escape_its_directory() {
    // `source` is caller-supplied and reaches a filename; a path separator or a
    // parent-dir hop must not steer the write outside the dump dir.
    let slug = payload_slug("hank promote ../../etc/passwd@HEAD (cli)");
    assert!(!slug.contains('/'), "{slug:?}");
    assert!(!slug.contains(".."), "{slug:?}");
    assert!(!slug.is_empty());
    assert_eq!(payload_slug(""), "payload");
    assert_eq!(payload_slug("///"), "payload");
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

#[test]
fn a_cfg_duplicate_source_tree_now_conforms_end_to_end() {
    // THE MONEY TEST for aegis-4ba2e. The two tests above prove the refusal is
    // now READABLE; this one proves there is nothing left to refuse. It runs the
    // real extractor over the real shape that froze hank — two mutually
    // exclusive `#[cfg]` declarations of one name, which tree-sitter sees both
    // of because it parses rather than evaluating cfg — and validates the
    // projection against the shipped shapes.
    //
    // Deliberately an END-TO-END test on a source tree, not a hand-written
    // Turtle constant: COLLIDING above is a fixture of what the emitter USED to
    // produce, so it can only ever prove SHACL still refuses that shape. Only
    // driving the extractor can prove the emitter stopped producing it.
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("state_tools.rs"),
        "#[cfg(feature = \"game-state\")]\n\
         pub type StateIngestRequest = crate::state::IngestRequest;\n\
         #[cfg(not(feature = \"game-state\"))]\n\
         pub struct StateIngestRequest { pub game_id: String }\n",
    )
    .unwrap();

    let turtle = crate::export::to_turtle(dir.path(), "demo").expect("export ran");
    assert_eq!(
        turtle.matches("bobbin:symbolKind").count(),
        1,
        "one IRI must carry exactly one symbolKind; got:\n{turtle}"
    );
    assert!(
        turtle.contains("more than one symbolKind"),
        "the collapse must be recorded in the payload, not done silently; got:\n{turtle}"
    );

    let v = validate(&turtle, SHAPES).expect("validation ran");
    assert!(
        v.conforms,
        "a cfg-duplicate tree must project a CONFORMING document — this is the \
         freeze aegis-4ba2e lifted; violations: {:?}",
        v.violations
    );
}
