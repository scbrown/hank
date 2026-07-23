# Changelog

All notable changes to this project will be documented in this file.

## [Unreleased]

## [0.3.0] - 2026-07-23

### Added

- *(change)* Answer what a CHANGE does, and name what it could not read (#39)([9d3fd2d](https://github.com/scbrown/hank/commit/9d3fd2d7d74a7d848bc8e4bb2c55e0c2d4ba30e9))
- *(shapes)* Code-edge SHACL shapes, proven able to accept AND refuse (#13)([e7e358f](https://github.com/scbrown/hank/commit/e7e358f601d5a5b7fcd740ef04e686e583b10dfb))
- *(status)* Make the policy layer observable in `hank status` (#45)([6cc696f](https://github.com/scbrown/hank/commit/6cc696fab6ba7491c1cb0e417f58803d6092b739))
- *(promote)* Wire the Quipu promotion write path — validate in-process, then write (#15/#14)([fb67ada](https://github.com/scbrown/hank/commit/fb67ada84284d43bc01ca3b6aa15aa3a6b43a263))
- *(mcp)* Hank_promote tool — the MCP surface of the promotion path (#15)([2d258cd](https://github.com/scbrown/hank/commit/2d258cd50510b0a4ecd8c6cd9f2a1705566a3801))
- *(cli)* Export --to quipu promotes — the second §15 spelling, one path (#15)([36c00e0](https://github.com/scbrown/hank/commit/36c00e0eef2f118c37105fd55969bb3aea1c2d19))
- *(daemon)* Resident graph process + liveness surface + loud-absence seam (hank #1, stage 1) (#53)([0fc5843](https://github.com/scbrown/hank/commit/0fc5843c09011451748bd75244ced04b47511283))
- *(daemon)* Graph-backed query endpoints over the resident graph (hank #1, stage 2) (#55)([e4c0cb3](https://github.com/scbrown/hank/commit/e4c0cb380c68e13b129b7386c0915b34c58e1fd6))
- *(daemon)* Resident-graph edit measurement + /measure endpoint (hank #1, stage 3a) (#56)([a3c2c30](https://github.com/scbrown/hank/commit/a3c2c30a94a45e61b9b05e71fedd2046693f982c))
- *(hook)* Pre-edit guard is a thin client of the resident daemon (hank #1, stage 3b) (#57)([14313f6](https://github.com/scbrown/hank/commit/14313f630a981267e434aa7048d228020c8a69aa))
- Tree-sitter structural edit rules (Selector + Predicate)([f927762](https://github.com/scbrown/hank/commit/f927762632e20603f25004f83b058c35bba448d2))
- Declare verdict freshness on structural rule verdicts (FR-3 slice)([3286eaa](https://github.com/scbrown/hank/commit/3286eaaf885fab081c0af1ef1d3376894b6dd685))
- Project quipu structural policies into the pre-edit guard (Phase 4)([8a72d7a](https://github.com/scbrown/hank/commit/8a72d7a51abb104bedeb3484f0092acf6ed8efcf))
- Ed25519 verdict signing + promotion (H-PROMOTE-VERDICT)([bfe144e](https://github.com/scbrown/hank/commit/bfe144e3f3684bc2968996bd5dea73441ca0eb75))
- *(mcp)* MCP graph tools are thin clients of the resident daemon (hank #1, stage 3c) (#58)([2d2e0cc](https://github.com/scbrown/hank/commit/2d2e0cc2e62467cf84fb70559e07e72c717168fa))
- *(policy)* Governed TEXT-rule plane — the quipu rule catalogue reaches the pre-edit guard (#60)([e84f151](https://github.com/scbrown/hank/commit/e84f151ba7560ccfc8478ec6a8de41bcc9a63f09))
- *(metrics)* The usage spool — guard decisions, governed rules, and deliberate use, one JSONL line each (#61)([14733cb](https://github.com/scbrown/hank/commit/14733cb1a5b3b2acd0b2d234e26bdd6b3ee9145d))
- *(metrics)* The guard and governed spool lines carry the MODE — soak hygiene for the enforce gate (#62)([52f3539](https://github.com/scbrown/hank/commit/52f3539fd0c519e488563e9eb321d0ac3839884e))

### CI/CD

- Test the mcp+quipu combo — the shipping config for hank_promote (#15)([db8d810](https://github.com/scbrown/hank/commit/db8d81005a73823ac625be5002061c9833a31c5d))

### Documentation

- *(policy-guard)* Advise mode is visible to the operator, not the agent (#40)([70edae3](https://github.com/scbrown/hank/commit/70edae31aceb83104e84e16615253408f87e7ca0))
- *(fr27)* Mark the parallel HTTP API phased instead of pretending it exists (#49)([11dd6e4](https://github.com/scbrown/hank/commit/11dd6e453dc0c06aa39cbd5fc65fd262a166dcd8))
- Reconcile four doc-vs-code drifts and pin the tool count with a test (#50)([e93fa2a](https://github.com/scbrown/hank/commit/e93fa2a4a716b3bc445ad903e4b7159e279c350c))
- *(promotion)* Promote is live + how to query dependencies back (#52)([fa99fac](https://github.com/scbrown/hank/commit/fa99fac10ddfec932397786cc63bbf7fe2b3314e))
- *(design)* Add the hank side of the policy edit-hooks path([bf9c0ff](https://github.com/scbrown/hank/commit/bf9c0ffd90442fdc61992c0e1fd87c548c2230f3))
- Add governed-relations & workflow-gated-edits design docs([e562027](https://github.com/scbrown/hank/commit/e56202787c738e2fbc75a2f8dd9b2df4a01133a5))
- Structural rules + projection (config + design)([5e02c14](https://github.com/scbrown/hank/commit/5e02c142dcb7870bc6e2896e49662cdbebbc4767))

### Fixed

- *(guard)* Measure every compiled language, and REPORT what it cannot measure (#38)([3aab970](https://github.com/scbrown/hank/commit/3aab970c3e6eefd8fa6dd1e7f12ac90bd5df9e75))
- *(baseline)* A ref that does not resolve builds NO baseline, and says so (#42)([01931e2](https://github.com/scbrown/hank/commit/01931e219c8be68380fe75d65fcab9c2d2784fa9))
- *(cli)* Honour --config and --verbose instead of silently ignoring them (#43)([2fee30b](https://github.com/scbrown/hank/commit/2fee30bfcbe5937c7ae20ac9806008d6bbde7317))
- *(mcp,cli)* Every served fact carries its tier; stop asserting freshness is served when it is not (#46)([45969c1](https://github.com/scbrown/hank/commit/45969c1d0f00282b82f27858f314133b41749fce))
- *(status)* Advertise only tiers with an implementation; drop the empty lsp/cpg features (#47)([caa2d16](https://github.com/scbrown/hank/commit/caa2d1649737f88085b0512d0b91c99bbb067a8f))
- *(config)* Wire the two live-security keys, mark the rest phased, guard against drift (#48)([3d8240e](https://github.com/scbrown/hank/commit/3d8240e9d4da571381ee18dc6fde070cc35da271))
- *(policy-guard)* Key the fail-open notice on the KIND of gap, not just the session (#51)([d4ac723](https://github.com/scbrown/hank/commit/d4ac7236476873cbbb590d715b73489e2faac2d4))
- *(promote)* Repo identity from --repo/origin remote, never the directory name (#15)([43af566](https://github.com/scbrown/hank/commit/43af56679348d0f5391bf01db3c740f7eb3a30ed))
- *(promote)* Surface quipu's server-side SHACL refusal as a refusal; isolate the endpoint test from the operator's user config([9e26609](https://github.com/scbrown/hank/commit/9e26609ab0427b28fbc19efebaa3854357f3ccd7))
- *(shapes)* Sync node shapes from quipu's registry — refuse symbol-IRI collisions before the network([6d399c5](https://github.com/scbrown/hank/commit/6d399c5ef5c8bfa530239a5a6014105cfd868087))
- *(ci)* Main is red — stale conforming fixture, unformatted pushes, three clippy lints([d922ff9](https://github.com/scbrown/hank/commit/d922ff9990cb16908d6d6892ee733784383a7808))
- *(export)* Every language this build parses exports — a Python repo promotes its real structure (the 81t2 class, found in export) (#63)([32e8fb8](https://github.com/scbrown/hank/commit/32e8fb8c28a3137fe6202ca00264eb2721c23e01))

### Miscellaneous

- *(fmt)* Reformat to satisfy stable rustfmt — CI Format has been red (#54)([f3f23a1](https://github.com/scbrown/hank/commit/f3f23a183f5da201f47a42c21242c582ff6e75b5))

### Testing

- *(cli)* Make refs_finds_definition actually able to fail (#44)([aa64ba9](https://github.com/scbrown/hank/commit/aa64ba9126a2b937bef1d60d7131b61f893c853d))
- Guard-level integration tests for structural rules([18049fc](https://github.com/scbrown/hank/commit/18049fca6228a31c75f16d0979a2ee279c2e2992))

### Design

- *(logo)* Give the feedback-loop lobes goggle eyes (#26)([b1e17dd](https://github.com/scbrown/hank/commit/b1e17dd28688490610c619af2d9ddd321e8320db))

## [0.2.0] - 2026-07-20

### Fixed

- **The pre-edit policy guard could block every edit in the fleet.** Absence of
  `hank` failed open (exit `127`), but a `hank` too old to know `hook pre-edit`
  answered it with the argument parser's error and exit `2` — Claude Code's
  fail-*closed* channel. Since parsing precedes any Hank code, the guard's own
  fail-open logic never ran. An unparseable `hank hook …` now degrades to a
  silent allow; other commands keep exit `2` so typos stay loud. The integration
  contract also pins a skew-proof invocation, since older binaries cannot be
  fixed retroactively (#35).
- **A workspace config silently disarmed the guard.** `HankConfig::load`
  documented itself as overlaying user then project config but assigned each
  file wholesale, so a project `.bobbin/config.toml` setting one unrelated key
  reset `[hank.policy]` to `mode = "off"` — enforcement stopped with no warning,
  indistinguishable from finding no violations. Config now merges per-key;
  arrays replace rather than accumulate, so a workspace cannot widen a scope the
  user config narrowed (#36).

### Added

- Vision document and full build specification (`docs/vision.md`,
  `docs/hank-spec.md`).
- Phase-1 scaffold: `hank` CLI (`analyze`, `refs`, `status`, `completions`,
  plus phase-gated `callers`/`impact`/`verify`/`promote`), tree-sitter Rust
  extraction, the tiered fact model, and the shared `[hank]` config table.
- Phase-1 MCP server (`mcp` feature): `hank serve` over stdio and
  streamable-HTTP, exposing `hank_status`, `hank_symbols`, `hank_references`,
  and `hank_analyze` via `rmcp` (Bobbin's registration pattern). The `mcp` arm
  is in the CI matrix.
- Phase-2 call graph and blast radius: tree-sitter call-site extraction, an
  in-memory `CodeGraph` (petgraph) with the `reachable()` primitive (FR-12),
  the `hank callers` and `hank impact` CLI commands, and the `hank_callers`,
  `hank_callees`, and `hank_impact` MCP tools.
- Phase-2 intra-procedural dataflow (Rust-native, resolving §14.1): per-function
  data-dependence extraction, `DependsOn`/`FlowsInto` flow queries, the
  `hank dataflow` CLI command, and the `hank_dataflow` MCP tool (eight tools
  total).
- Phase-2 exit (FR-11): reconcile the structural blast radius against a
  caller-supplied co-change set (`src/reconcile.rs`), partitioning files into
  corroborated / structural-only / co-change-only. Exposed via
  `hank impact --cochange` (CLI) and the `cochange` parameter on `hank_impact`
  (MCP). Co-change mining stays in Bobbin. **Phase 2 complete.**
- Edit-reactive harness integration (spec §5.9 / FR-30, prototype): `hank hook
  post-edit` reads a Claude Code `PostToolUse` payload on stdin and returns a
  synchronous cross-file blast-radius advisory as injected context — the agent's
  edit tool call *is* the change event. Spec §5.9 reworked into an interface
  model (hook / MCP / HTTP / CLI / optional LSP) with the resident-daemon latency
  requirement (FR-31).
- Unified code+docs referential structure (spec §5.10): the concept crystallized
  — code and docs are one referential graph (real-time for code, asynchronous for
  docs), distinct from Bobbin's chunking. Adds FR-33 (doc→code reference
  extraction) and FR-34 (`hank export`).
- Referential-structure export (FR-34, code side): `hank export --format turtle`
  emits `CodeModule`/`CodeSymbol` + `definedIn`/`calls` edges as RDF Turtle in the
  `bobbin:` code ontology (validating against `shapes/code-entities.ttl`) — the
  governed projection that is the substrate under Phase-4 promotion.
- Docs published to the `gh-pages` branch via `peaceiris/actions-gh-pages`.
- Project tooling matched to Bobbin and Quipu: `just` recipes, pre-commit,
  clippy lint policy, markdownlint/Vale/Prettier, mdBook, CI, and release-plz.
