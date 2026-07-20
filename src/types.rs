//! The typed fact model Hank serves.
//!
//! Every served fact carries a [`Tier`] (how it was derived) — the confidence
//! tag FR-3 requires so a consumer never mistakes a tree-sitter approximation
//! for an LSP-precise fact.
//!
//! [`Freshness`] (how current a fact is) is FR-3's OTHER half, and it is **not
//! yet served**. Freshness state (fresh/stale/recomputing) is a property of the
//! Phase-3 resident graph + file-watcher (FR-16/17); the current serve path
//! rebuilds the graph on demand per request from the working copy, where there
//! is no cached fact that could be stale. So [`Freshness`] and [`Fact`] (the
//! subject–edge–object carrier that pairs a value with both tags) are defined
//! here as the Phase-3 carrier but have no caller on the served path yet.
//!
//! This docstring previously asserted that every served fact carried a freshness
//! tag. It did not — freshness was served nowhere (aegis-8yrn). Correcting the
//! claim, rather than stamping a hardcoded `fresh` that would imply a tracking
//! system that does not exist, is the FR-3 honesty rule applied to the spec
//! itself: a fact that cannot state its own freshness says so, it does not
//! default to looking current.

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

impl Tier {
    /// The lowercase string form used on the wire and in the ontology.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Tier::TreeSitter => "treesitter",
            Tier::Lsp => "lsp",
            Tier::Cpg => "cpg",
        }
    }
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

impl SymbolKind {
    /// The lowercase string form used on the wire and in the ontology
    /// (`bobbin:symbolKind`).
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            SymbolKind::Function => "function",
            SymbolKind::Method => "method",
            SymbolKind::Class => "class",
            SymbolKind::Interface => "interface",
            SymbolKind::Enum => "enum",
            SymbolKind::Struct => "struct",
            SymbolKind::Variable => "variable",
            SymbolKind::Constant => "constant",
            SymbolKind::Module => "module",
            SymbolKind::Property => "property",
            SymbolKind::Field => "field",
            SymbolKind::Constructor => "constructor",
            SymbolKind::TypeAlias => "type_alias",
        }
    }
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
///
/// The Phase-3 freshness carrier (see the module docs): it pairs a value with
/// both a [`Tier`] and a [`Freshness`], but nothing on the served path constructs
/// one yet — freshness lands with the resident graph + watcher (FR-16/17).
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
