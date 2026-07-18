//! Implementations of the call-graph and dataflow CLI commands.
//!
//! These live outside `cli.rs` to keep that file small; they take the two
//! output flags they need (`json`, `quiet`) rather than the whole `Cli`.

use std::collections::BTreeSet;
use std::path::Path;

use colored::Colorize;

use crate::dataflow::{Dataflow, FlowDir};
use crate::export;
use crate::graph::{CodeGraph, Dir};
use crate::reconcile::reconcile;
use crate::render::{print_reached, reached_json};

/// `hank callers` — direct callers and callees of a symbol.
pub(crate) fn callers(json: bool, quiet: bool, symbol: &str, path: &Path) -> anyhow::Result<()> {
    let graph = CodeGraph::build(path)?;
    if !graph.has_symbol(symbol) {
        return not_found(json, quiet, symbol, "call graph");
    }
    let callers = graph.direct(symbol, Dir::Callers);
    let callees = graph.direct(symbol, Dir::Callees);

    if json {
        let out = serde_json::json!({
            "symbol": symbol,
            "callers": reached_json(&callers),
            "callees": reached_json(&callees),
        });
        println!("{}", serde_json::to_string_pretty(&out)?);
    } else {
        print_reached(&format!("callers of {symbol}"), &callers, quiet);
        print_reached(&format!("callees of {symbol}"), &callees, quiet);
    }
    Ok(())
}

/// `hank impact` — the blast radius (transitive callers) of a symbol,
/// optionally reconciled against a caller-supplied co-change set (FR-11).
pub(crate) fn impact(
    json: bool,
    quiet: bool,
    symbol: &str,
    path: &Path,
    hops: u32,
    cochange: Option<&Path>,
) -> anyhow::Result<()> {
    let graph = CodeGraph::build(path)?;
    if !graph.has_symbol(symbol) {
        return not_found(json, quiet, symbol, "call graph");
    }
    let reached = graph.reachable(symbol, Dir::Callers, hops);
    let structural_files: BTreeSet<String> = reached.iter().map(|r| r.file.clone()).collect();
    let cochange_set = cochange.map(read_cochange).transpose()?;

    if json {
        let mut out = serde_json::json!({
            "symbol": symbol,
            "hops": hops,
            "direction": "callers",
            "count": reached.len(),
            "reachable": reached_json(&reached),
            "structural_files": structural_files.iter().collect::<Vec<_>>(),
        });
        if let Some(cochange_set) = &cochange_set {
            let recon = reconcile(&structural_files, cochange_set);
            out["reconciliation"] = serde_json::json!({
                "corroborated": recon.corroborated,
                "structural_only": recon.structural_only,
                "cochange_only": recon.cochange_only,
            });
        }
        println!("{}", serde_json::to_string_pretty(&out)?);
        return Ok(());
    }

    if reached.is_empty() {
        if !quiet {
            println!("nothing calls {symbol} (blast radius empty)");
        }
        return Ok(());
    }
    println!(
        "{} {} symbol(s) affected by changing {symbol}:",
        "impact".green().bold(),
        reached.len()
    );
    for item in &reached {
        println!(
            "  {}:{} {} (hop {})",
            item.file,
            item.start_line,
            item.name.cyan(),
            item.distance
        );
    }
    if let Some(cochange_set) = &cochange_set {
        let recon = reconcile(&structural_files, cochange_set);
        println!(
            "\nreconciled with {} co-changed file(s):",
            cochange_set.len()
        );
        print_bucket("corroborated (real coupling)", &recon.corroborated, quiet);
        print_bucket(
            "structural only (new/unexercised)",
            &recon.structural_only,
            quiet,
        );
        print_bucket(
            "co-change only (refactoring smell)",
            &recon.cochange_only,
            quiet,
        );
    }
    Ok(())
}

/// Read a co-change file: a JSON array of paths or a newline-separated list.
fn read_cochange(path: &Path) -> anyhow::Result<BTreeSet<String>> {
    let text = std::fs::read_to_string(path)?;
    if let Ok(list) = serde_json::from_str::<Vec<String>>(&text) {
        return Ok(list.into_iter().collect());
    }
    Ok(text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(str::to_string)
        .collect())
}

/// Print one reconciliation bucket.
fn print_bucket(label: &str, files: &[String], quiet: bool) {
    if files.is_empty() {
        if !quiet {
            println!("  {label}: (none)");
        }
        return;
    }
    println!("  {}: {}", label.bold(), files.join(", "));
}

/// `hank dataflow` — intra-procedural data dependence within a function.
pub(crate) fn dataflow(
    json: bool,
    quiet: bool,
    function: &str,
    path: &Path,
    var: Option<&str>,
    forward: bool,
    hops: u32,
) -> anyhow::Result<()> {
    let flow = Dataflow::build(path)?;
    if !flow.has_function(function) {
        return not_found(json, quiet, function, "dataflow");
    }
    let dir = if forward {
        FlowDir::FlowsInto
    } else {
        FlowDir::DependsOn
    };

    match var {
        Some(var) => {
            let steps = flow.flow(function, var, dir, hops);
            if json {
                let out = serde_json::json!({
                    "function": function,
                    "var": var,
                    "direction": dir.as_str(),
                    "count": steps.len(),
                    "flow": steps.iter().map(|s| serde_json::json!({ "name": s.name, "distance": s.distance })).collect::<Vec<_>>(),
                });
                println!("{}", serde_json::to_string_pretty(&out)?);
            } else if steps.is_empty() {
                if !quiet {
                    println!("{var} has no {} edges in {function}", dir.as_str());
                }
            } else {
                println!("{} of {var} in {function}:", dir.as_str());
                for step in &steps {
                    println!("  {} (hop {})", step.name.cyan(), step.distance);
                }
            }
        }
        None => {
            let edges = flow.edges(function);
            if json {
                let out = serde_json::json!({
                    "function": function,
                    "count": edges.len(),
                    "edges": edges.iter().map(|e| serde_json::json!({ "dependent": e.dependent, "depends_on": e.depends_on, "line": e.line })).collect::<Vec<_>>(),
                });
                println!("{}", serde_json::to_string_pretty(&out)?);
            } else if edges.is_empty() {
                if !quiet {
                    println!("no data-dependence edges in {function}");
                }
            } else {
                println!("data dependence in {function}:");
                for edge in edges {
                    println!(
                        "  {}:{} {} depends on {}",
                        function,
                        edge.line,
                        edge.dependent.cyan(),
                        edge.depends_on.cyan()
                    );
                }
            }
        }
    }
    Ok(())
}

/// `hank export` — emit the referential structure as Turtle.
pub(crate) fn export(path: &Path, repo: Option<&str>) -> anyhow::Result<()> {
    let repo = repo.map_or_else(
        || {
            path.canonicalize()
                .ok()
                .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
                .unwrap_or_else(|| "repo".to_string())
        },
        str::to_string,
    );
    let turtle = export::to_turtle(path, &repo)?;
    print!("{turtle}");
    Ok(())
}

/// Shared "not found" reporting for a missing symbol/function.
fn not_found(json: bool, quiet: bool, name: &str, what: &str) -> anyhow::Result<()> {
    if json {
        println!("{}", serde_json::json!({ "name": name, "found": false }));
    } else if !quiet {
        println!("{name} not found in the {what}");
    }
    Ok(())
}
