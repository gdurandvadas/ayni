---
project_id: "0-8-0-fixes"
document: changes
---

# Changes: 0.8.0 fixes

Append one section per milestone. Before `project_commit`, use this exact marker:
`- Status: validated`.

<!--
## Milestone: milestone-1

- Status: validated
- Changed responsibilities:
- Explicit paths:
- Focused checks:
- Promotion validation:
- Ayni: passed | failed | skipped
- Accepted deviations: none
-->

## Milestone: milestone-1

- Status: validated
- Changed responsibilities: Core maximum-threshold semantics and size classification; Rust, Go, Node, Python, and Kotlin complexity collection; behavior-preserving decomposition of Go test collection, Rust dependency collection, and CLI verify.
- Explicit paths: `core/src/lib.rs`, `core/src/threshold.rs`, `core/src/size.rs`, `adapters/rust/src/collectors/complexity.rs`, `adapters/rust/src/collectors/deps.rs`, `adapters/go/src/collectors/complexity.rs`, `adapters/go/src/collectors/test.rs`, `adapters/node/src/collectors/complexity.rs`, `adapters/python/src/collectors/complexity.rs`, `adapters/kotlin/src/collectors/complexity.rs`, `cli/src/verify.rs`, `Cargo.lock`.
- Focused commands and outcomes: Focused formatting/tests passed. Exact focused checkout verification passed after decomposition: maximum cyclomatic Go 9, Rust deps 6, CLI verify 10; all `fail_count 0`. One canonical Ayni analysis initially exposed three corrected fail-at-15 functions; no second analysis is claimed.
- Promotion commands and outcomes: Supplied focused and promotion fmt/tests passed.
- Ayni status and evidence: passed after focused remediation; evidence is the exact focused checkout verification above.
- Accepted deviations: Cargo.lock contains approved eventual 0.8.0 local workspace metadata early due Cargo/rust-analyzer regeneration; Task 6.1 remains pending.
