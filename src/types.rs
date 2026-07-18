//! The typed fact model Hank serves.
//!
//! Every served fact carries a [`Tier`] (how it was derived) and a
//! [`Freshness`] tag (how current it is) — the "confidence tag" the spec
//! requires (FR-3) so a consumer never mistakes a tree-sitter approximation for
//! an LSP-precise fact.

use serde::{Deserialize, Serialize};

/// Provenance/precision of a fact — which extractor produced it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Tier {
    /// Fast, build-free, approximate (tree-sitter).
    TreeSitter,
    /// Precise defs/refs/types where a build resolves (LSP).
    Lsp,
    /// Control/data dependence from the code property graph.
    Cpg,
}

/// How current a served fact is relative to the tenant's working copy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Freshness {
    /// Reflects the latest observed edit.
    Fresh,
    /// Known to be behind a pending edit.
    Stale,
    /// A recompute is in flight.
    Recomputing,
}

/// The kind of a named code symbol. Values mirror the enumeration in Quipu's
/// `shapes/code-entities.ttl` (`bobbin:symbolKind`) so promoted facts validate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SymbolKind {
    /// A free function.
    Function,
    /// A method associated with a type.
    Method,
    /// A class.
    Class,
    /// An interface / trait.
    Interface,
    /// An enum.
    Enum,
    /// A struct.
    Struct,
    /// A variable binding.
    Variable,
    /// A constant.
    Constant,
    /// A module.
    Module,
    /// A property.
    Property,
    /// A field.
    Field,
    /// A constructor.
    Constructor,
    /// A type alias.
    TypeAlias,
}

/// A structural edge between two symbols or modules. These become predicates in
/// the `bobbin:` code ontology on promotion (see `docs/hank-spec.md` §9.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EdgeKind {
    /// Caller invokes callee.
    Calls,
    /// A use site of a definition.
    References,
    /// A symbol is defined in a module.
    DefinedIn,
    /// A module depends on another module.
    Imports,
    /// A data-dependence edge (CPG).
    DataDependsOn,
    /// A control-dependence edge (CPG).
    ControlDependsOn,
}

/// A named symbol extracted from a source file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Symbol {
    /// The symbol's name.
    pub name: String,
    /// What kind of symbol it is.
    pub kind: SymbolKind,
    /// 1-based line where the symbol begins.
    pub start_line: usize,
    /// 1-based line where the symbol ends.
    pub end_line: usize,
    /// How this symbol was derived.
    pub tier: Tier,
}

/// A single structural fact: `subject —edge→ object`, tagged with provenance.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fact {
    /// The subject identifier.
    pub subject: String,
    /// The relationship.
    pub edge: EdgeKind,
    /// The object identifier.
    pub object: String,
    /// How this fact was derived.
    pub tier: Tier,
    /// How current the fact is.
    pub freshness: Freshness,
}
