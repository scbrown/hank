//! Bodies for the two heaviest `hank_*` tool handlers, lifted out of `server`
//! for size (hank #83).
//!
//! `#[tool_router]` collects every `#[tool]` method from ONE impl block, so the
//! handlers themselves cannot move — only their bodies. Each is a plain function
//! taking the server, and `server.rs` keeps a thin wrapper carrying the tool
//! attribute and its description. Neither body awaits anything, so neither needs
//! to be async.
//!
//! A child module of `server`, so `internal`, `json_result` and `graph_tier`
//! reach it through `use super::*` and lifting these out changed no visibility.

use super::*;

/// Body of `hank_promote`.
pub(super) fn promote(
    server: &HankMcpServer,
    req: PromoteRequest,
) -> Result<CallToolResult, McpError> {
    #[cfg(not(feature = "quipu"))]
    {
        // Both arguments go unread on the refusal path; the method took `self`
        // implicitly before the body moved here, so `server` joins `req` in the
        // existing discard rather than becoming an `_server` in the signature.
        let _ = (&req, server);
        return Err(internal(crate::errors::Error::Config(
            "hank_promote needs the `quipu` feature; this server was built without it".to_string(),
        )));
    }
    #[cfg(feature = "quipu")]
    {
        let config =
            HankConfig::resolve(server.config.as_deref(), &server.root).map_err(internal)?;
        // Promotion is a WRITE — honour the same guard the CLI does. Refused
        // before any work, so read_only means read_only even here.
        config.write_guard("promotion").map_err(internal)?;

        let endpoint = req
            .endpoint
            .filter(|e| !e.is_empty())
            .or_else(|| Some(config.quipu.endpoint.clone()).filter(|e| !e.is_empty()))
            .ok_or_else(|| {
                internal(crate::errors::Error::Promote(
                    "no Quipu endpoint: set [hank.quipu] endpoint or pass one in the \
                     request. Refusing rather than guessing a graph."
                        .to_string(),
                ))
            })?;

        let base = req
            .path
            .as_ref()
            .map_or_else(|| server.root.clone(), |p| server.root.join(p));
        // Repo identity is a segment of every promoted IRI. Request value wins;
        // otherwise the origin remote names the repository. With neither,
        // REFUSE — the old dir-basename fallback minted `code/<worktree-dir>/…`
        // islands (an agent worktree promoted an entire graph as `code/gennaro`).
        let repo = match req.repo.as_deref() {
            Some(r) => r.to_string(),
            None => crate::git::origin_repo_name(&base).ok_or_else(|| {
                internal(crate::errors::Error::Promote(format!(
                    "cannot determine repository identity: no `origin` remote at {}. \
                     Pass `repo` in the request. Refusing rather than deriving \
                     identity from the directory name, which fragments the graph.",
                    base.display()
                )))
            })?,
        };
        let turtle = crate::export::to_turtle(&base, &repo).map_err(internal)?;

        let source = format!("hank promote {repo} (mcp)");
        let response =
            match crate::promote::promote(&endpoint, &turtle, &source).map_err(internal)? {
                crate::promote::Promotion::Wrote(k) => PromoteResponse {
                    wrote: true,
                    count: Some(k.count),
                    tx_id: k.tx_ids.last().copied(),
                    chunks: Some(k.chunks),
                    violations: Vec::new(),
                },
                crate::promote::Promotion::Refused(violations) => PromoteResponse {
                    wrote: false,
                    count: None,
                    tx_id: None,
                    chunks: None,
                    violations,
                },
            };
        json_result(&response)
    }
}

/// Body of `hank_dataflow`.
pub(super) fn dataflow(
    server: &HankMcpServer,
    req: &DataflowRequest,
) -> Result<CallToolResult, McpError> {
    let base = req
        .path
        .as_ref()
        .map_or_else(|| server.root.clone(), |p| server.root.join(p));
    let flow = Dataflow::build(&base).map_err(internal)?;
    let found = flow.has_function(&req.function);

    let (direction, steps, edges) = match &req.var {
        Some(var) => {
            let dir = if req.forward.unwrap_or(false) {
                FlowDir::FlowsInto
            } else {
                FlowDir::DependsOn
            };
            let steps = flow
                .flow(&req.function, var, dir, req.hops.unwrap_or(5))
                .into_iter()
                .map(|s| FlowStepItem {
                    name: s.name,
                    distance: s.distance,
                })
                .collect();
            (Some(dir.as_str().to_string()), steps, Vec::new())
        }
        None => {
            let edges = flow
                .edges(&req.function)
                .iter()
                .map(|e| DepEdgeItem {
                    dependent: e.dependent.clone(),
                    depends_on: e.depends_on.clone(),
                    line: e.line,
                })
                .collect();
            (None, Vec::new(), edges)
        }
    };

    let response = DataflowResponse {
        function: req.function.clone(),
        found,
        direction,
        var: req.var.clone(),
        flow: steps,
        edges,
        tier: graph_tier(),
    };
    json_result(&response)
}
