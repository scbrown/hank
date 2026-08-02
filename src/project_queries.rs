//! The SPARQL the projection sends, and the ontology IRIs it names.
//!
//! Split out of [`crate::project`] purely for file size — that module was over
//! this repo's 500-line hard limit before anything in it was touched, and the
//! queries are the part with no logic in them, so moving them changes what a
//! reader has to hold in their head and nothing else.
//!
//! They stay `pub` and re-exported from `project`, so every existing path
//! (`project::TEXT_POLICY_QUERY`) still resolves: this is a move, not an API
//! change.

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

/// The governed policy whose claim decides repo exposure — mqnl's rule #1,
/// live in the graph. The IRI is data about the deployment's ontology, like
/// the `aegis:` prefix in the queries above: one namespace, one policy plane.
pub const EXPOSURE_POLICY_IRI: &str =
    "http://aegis.gastown.local/ontology/policy_no-internal-ids-in-public-repos";
