//! Node lookups over a built [`CodeGraph`] — definition sites by name and the
//! symbol list of one file. Child module of `graph` so it can read the private
//! `graph`/`by_name` fields; kept out of `mod.rs` to hold the module at build +
//! traversal (file-size discipline, one responsibility per module).

use super::{CodeGraph, SymbolNode};

impl CodeGraph {
    /// Definition sites of `name`, from the resident node index. Zero results
    /// means the graph holds no symbol by that name — distinct from "the graph
    /// is empty", which [`Self::stats`] reports.
    #[must_use]
    pub fn definitions(&self, name: &str) -> Vec<&SymbolNode> {
        self.by_name.get(name).map_or_else(Vec::new, |ixs| {
            ixs.iter().map(|&ix| &self.graph[ix]).collect()
        })
    }

    /// Symbols defined in `rel` (a root-relative path), sorted by line.
    ///
    /// An empty result means the resident graph holds NO symbols for that path —
    /// which it cannot tell apart from "no such file" or "file not parseable":
    /// files contribute to the graph only through their symbols. Callers that
    /// report to a human must say "no symbols in the resident graph", never
    /// "the file is empty".
    #[must_use]
    pub fn file_symbols(&self, rel: &str) -> Vec<&SymbolNode> {
        let mut symbols: Vec<&SymbolNode> = self
            .graph
            .node_indices()
            .map(|ix| &self.graph[ix])
            .filter(|n| n.file == rel)
            .collect();
        symbols.sort_by_key(|n| n.start_line);
        symbols
    }
}

#[cfg(test)]
mod tests {
    use crate::graph::CodeGraph;

    fn repo() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.rs"), "fn shared() {}\nfn only_a() {}\n").unwrap();
        std::fs::write(dir.path().join("b.rs"), "fn shared() {}\n").unwrap();
        dir
    }

    #[test]
    fn definitions_returns_every_site_of_a_reused_name() {
        let graph = CodeGraph::build(repo().path()).unwrap();
        let defs = graph.definitions("shared");
        let mut files: Vec<&str> = defs.iter().map(|d| d.file.as_str()).collect();
        files.sort_unstable();
        assert_eq!(files, ["a.rs", "b.rs"], "both definition sites, not one");
        assert!(graph.definitions("absent").is_empty());
    }

    #[test]
    fn file_symbols_lists_one_files_symbols_in_line_order() {
        let graph = CodeGraph::build(repo().path()).unwrap();
        let names: Vec<&str> = graph
            .file_symbols("a.rs")
            .iter()
            .map(|s| s.name.as_str())
            .collect();
        assert_eq!(names, ["shared", "only_a"], "a.rs symbols in line order");
        assert!(graph.file_symbols("missing.rs").is_empty());
    }
}
