//! Small rendering helpers shared by CLI commands.

use colored::Colorize;

use crate::graph::Reached;

/// Render reached symbols as a JSON array.
pub(crate) fn reached_json(items: &[Reached]) -> Vec<serde_json::Value> {
    items
        .iter()
        .map(|r| {
            serde_json::json!({
                "name": r.name,
                "file": r.file,
                "start_line": r.start_line,
                "distance": r.distance,
                "via": r.via,
            })
        })
        .collect()
}

/// Print a labeled list of reached symbols for the human-readable output.
pub(crate) fn print_reached(header: &str, items: &[Reached], quiet: bool) {
    if items.is_empty() {
        if !quiet {
            println!("{header}: (none)");
        }
        return;
    }
    println!("{header}:");
    for item in items {
        println!("  {}:{} {}", item.file, item.start_line, item.name.cyan());
    }
}
