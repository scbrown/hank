# Changelog

All notable changes to this project will be documented in this file.

## [Unreleased]

## [0.3.1] - 2026-07-23

### Added

- Scaffold Hank — Phase 1 CLI, tooling, docs([ea0f78b](https://github.com/scbrown/hank/commit/ea0f78bd3ae44d5a2a0396dcc77d163438eb83e0))
- Phase-1 MCP server over rmcp (stdio + streamable-HTTP)([90a0e73](https://github.com/scbrown/hank/commit/90a0e73e55ff45e728e1257fdf4a8ec3df8a7d7d))
- Phase-2 call graph and blast radius([9951f47](https://github.com/scbrown/hank/commit/9951f479bf8cc16aac9ecb7f9453926cfdd8161e))
- Phase-2 intra-procedural dataflow (Rust-native)([69bd5f9](https://github.com/scbrown/hank/commit/69bd5f958671a7ca05094ed48dc1b8c3664463e9))
- Phase-2 exit — co-change reconciliation (FR-11)([90df6b5](https://github.com/scbrown/hank/commit/90df6b54377ffe5ddee2a5e29c42ce7759049aaa))
- Edit-reactive harness hook + interface-model spec (FR-30/31)([6bd66a2](https://github.com/scbrown/hank/commit/6bd66a253d9893959a2295a1c5cc2df744942bb3))
- Referential-structure export + code/docs synergy (§5.10, FR-33/34)([d5668ec](https://github.com/scbrown/hank/commit/d5668eccced7d151d077384f98a98cda382a4189))
- Extract module import edges (bobbin:imports) in export (#23)([f0de144](https://github.com/scbrown/hank/commit/f0de144c60968f4114756176f7778b0e37606558))
- Git baseline — resolve base_ref to a commit + commit diff (OQ2) (#24)([9be9c49](https://github.com/scbrown/hank/commit/9be9c49241abfb0f13c6d7dad46692031a0de7bc))
- Live Louvain community detection over the in-memory graph (FR-9) (#27)([fec15ca](https://github.com/scbrown/hank/commit/fec15cabacc1320e4b140f9ad2665fa15e97eae0))
- Build the base graph from git-tree content at a ref (#12 slice 2, FR-13) (#28)([153b02c](https://github.com/scbrown/hank/commit/153b02ce19be56ab5346666e22011712d923dd90))
- Hank #5 (file-watch) + #16 (doc→code refs) + #9 grammars (langs-extra) (#29)([5fc6749](https://github.com/scbrown/hank/commit/5fc6749ba0f59da7c57aac3024b216bc07087030))
- *(policy)* Pre-edit blocking guard + per-tenant capability scoping (#20, #21) (#32)([53a2c41](https://github.com/scbrown/hank/commit/53a2c4131a848a133405d83093d0411306fc7b6f))
- *(verify)* Hank_verify monitor-guided edit verification (#19) (#33)([8a5d5f5](https://github.com/scbrown/hank/commit/8a5d5f544e13dbd02090fe47aa4e44d3c05afbfb))
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
- *(promote)* Write provenance + the rudof<->quipu verdict-agreement test([3dda463](https://github.com/scbrown/hank/commit/3dda4638357bc096a1618993db73174fe1551c61))
- *(census)* Hank census — count same-file symbol-name collisions at the only layer that can see them([900062a](https://github.com/scbrown/hank/commit/900062a2043e969c47a8a477e5eb5a848c1758f4))
- *(promote)* Chunk oversized promotions so real-world repos fit through /knot (#59)([83f414b](https://github.com/scbrown/hank/commit/83f414be6979122db0e8eab125e291b907db8d6b))
- *(extract)* Scope-qualified symbol IRIs — same-named symbols in one file stop merging (#64)([5bd0d39](https://github.com/scbrown/hank/commit/5bd0d394d4e3fc72262f0a76de4ea0ccc83553fb))
- *(promote)* Optional bearer auth on the Quipu write path (QUIPU_AUTH_TOKEN)([b9806fa](https://github.com/scbrown/hank/commit/b9806fa8dcfa5cd8613ce8b1ddff2a112c8115b6))
- *(promote)* Token-file fallback for the Quipu bearer — reaches pre-flip processes([a2d3cd9](https://github.com/scbrown/hank/commit/a2d3cd9a79e1a5ca6f9ddc10c342e4490285a80f))
- *(daemon)* Complete the FR-27 query surface — /references, /symbols, /dataflow (hank #1, stage 4) (#65)([a58467b](https://github.com/scbrown/hank/commit/a58467b871beaeac7ed1c0dcd9b73bf83168f234))
- *(daemon)* Post-edit thin client + graceful shutdown + wire-level SLO test — closes hank #1 (stage 5) (#66)([15756f1](https://github.com/scbrown/hank/commit/15756f15b886d935162930cedb797ec9a0b2f912))
- *(graph)* Shared read-only base + CoW per-tenant overlays (hank #2, slices 1+2+4) (#67)([67dcbba](https://github.com/scbrown/hank/commit/67dcbba33c9b0e124f43b3f179032243ae7bb29c))
- *(daemon)* Wire the tenant layer live — /edit feed, tenant-scoped queries, status overlays (hank #2 close) (#68)([fcd6958](https://github.com/scbrown/hank/commit/fcd6958121f056f0fe1fae5014a13f0f408001ef))
- *(graph)* Frontier-bounded overlay update + overlay-new-name resolution (hank #3, FR-16) (#69)([a298521](https://github.com/scbrown/hank/commit/a2985215ca7035c8f7e5ff0b17b82dfb5280098c))
- *(graph)* Content-hash structural sharing — base-hit no-op + sharing stats (hank #4, FR-15) (#70)([2a66389](https://github.com/scbrown/hank/commit/2a66389931255b5132547c25e85b0a65b99c12a9))
- *(watch)* File-watch drives per-tenant overlays via the frontier recompute (hank #5, FR-17) (#71)([00898b7](https://github.com/scbrown/hank/commit/00898b7e84d2e74c755a65cd445e8092511ea930))
- *(graph)* Overlay lifecycle + eviction + high-fan-in guard — completes Phase 3 (hank #6, FR-18) (#72)([8cb823f](https://github.com/scbrown/hank/commit/8cb823fbc349d93074e3b2ef91d8595dba62d905))
- *(promote)* Read the committed tree, not the working tree — FR-22 + arbitrary --commit (hank #15 slice) (#73)([12a1529](https://github.com/scbrown/hank/commit/12a1529536671d4813cd2cfe26b4a58ca62e3943))

### Changed

- *(graph)* Extract the FR-12 BFS behind an Adjacency trait (Slice 0) (#30)([f336d06](https://github.com/scbrown/hank/commit/f336d06be36071ec5774e72024c3d34282b5d03f))

### Documentation

- Add Hank vision and build specification([41519e9](https://github.com/scbrown/hank/commit/41519e9ef71e0f0db0d98671dab7e7090374b510))
- *(readme)* Named-competitor comparison + selling points + hank×quipu use-cases; reference the hank CLI (not cargo run)([18e813d](https://github.com/scbrown/hank/commit/18e813df8e091e4a9bb5917b28aed13efb97335a))
- Reconcile four doc-vs-code drifts and pin the tool count with a test (#50)([e93fa2a](https://github.com/scbrown/hank/commit/e93fa2a4a716b3bc445ad903e4b7159e279c350c))
- *(promotion)* Promote is live + how to query dependencies back (#52)([fa99fac](https://github.com/scbrown/hank/commit/fa99fac10ddfec932397786cc63bbf7fe2b3314e))

### Fixed

- *(policy)* A stale hank must not block every edit in the fleet (#35)([e38b5b8](https://github.com/scbrown/hank/commit/e38b5b8288f3fb32da7429a22c47350a6ebbc3f3))
- *(config)* A workspace config must not silently disarm the guard (#36)([831f6fc](https://github.com/scbrown/hank/commit/831f6fc08afef1cd9c3e4a731099f937698b195d))
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

- *(release)* V0.2.0 (#37)([3fc8e76](https://github.com/scbrown/hank/commit/3fc8e765ace94e797f25970931ae449faf1fccad))
- *(fmt)* Reformat to satisfy stable rustfmt — CI Format has been red (#54)([f3f23a1](https://github.com/scbrown/hank/commit/f3f23a183f5da201f47a42c21242c582ff6e75b5))
- *(release)* V0.3.0 — 43 unreleased commits, plus the two gaps that let them pile up([b529984](https://github.com/scbrown/hank/commit/b52998423709d3c26a7f701c06e1c3358595bc44))

### Testing

- *(cli)* Make refs_finds_definition actually able to fail (#44)([aa64ba9](https://github.com/scbrown/hank/commit/aa64ba9126a2b937bef1d60d7131b61f893c853d))
- Guard-level integration tests for structural rules([18049fc](https://github.com/scbrown/hank/commit/18049fca6228a31c75f16d0979a2ee279c2e2992))

### Release

- V0.3.1 — quipu feature in the shipped binary + stub promote exits non-zero([f7949fd](https://github.com/scbrown/hank/commit/f7949fd00713a3e2a560d3a40ad0a3d29d4855c2))

### Style

- Rustfmt the provenance + shape-agreement commit — third unformatted direct-to-main push today([b44bf3c](https://github.com/scbrown/hank/commit/b44bf3c7a113b09fd718e4e0b1dd056aa9cf49f7))

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
