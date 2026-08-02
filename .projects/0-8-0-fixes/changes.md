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

## Milestone: milestone-4

- Status: validated
- Changed responsibilities: Internal expected `(language, normalized root, signal)` row planning/reconciliation; missing, duplicate, or unexpected rows make completion incomplete; failed rows remain evidence; schema-v3 structural row validation and zero-row hardening; public optional `verify --root`; all generated commands include shell-safe originating config/language/exact root and supported selectors; multi-root commands execute reproducibly; CLI docs regenerated.
- Explicit changed paths: `cli/src/completion.rs`, `cli/src/verification_command.rs`, `cli/src/main.rs`, `cli/src/verify.rs`, `cli/src/tests.rs`, `cli/src/ui/report.rs`, `cli/tests/completion_e2e.rs`, `cli/tests/findings_e2e.rs`, `cli/tests/verify_e2e.rs`, `core/src/signal.rs`, `docs/cli.md`, `docs/product/config.md`, `docs/product/signals/v3.md`.
- Focused commands and outcomes: Focused validation passed as supplied by the orchestrator.
- Promotion commands and outcomes: `cargo fmt --all -- --check` passed; `cargo test -p ayni-core --all-features` passed; `cargo test -p ayni-cli --test completion_e2e --test findings_e2e --test verify_e2e --all-features` passed.
- Ayni status and evidence: passed. Canonical Ayni ran once and passed: complete 1/1, 5/5 rows, 289 tests, coverage 67.58260283209711%, zero failing offenders.
- Accepted deviations: Comparison required no direct edit because its loading/validation already traverses strengthened `RunArtifact` validation.

## Milestone: milestone-3

- Status: validated
- Changed responsibilities: Catalog eligibility now uses any associated enabled signal, with test-only/coverage-only Node/Python readiness matrices; lexical configured-root validation rejects all parent, absolute/rooted/UNC, and drive-prefixed forms; shared canonical containment rejects existing symlink escapes while preserving inside symlinks and missing safe roots; analyze, verify, install list/apply, and install check guard before detection/execution, while contract display remains filesystem-free.
- Explicit changed paths: `.projects/0-8-0-fixes/plan.md`, `core/src/policy.rs`, `adapters/common/src/paths.rs`, `cli/src/discovery.rs`, `cli/src/install.rs`, `cli/src/tests.rs`, `cli/src/verify.rs`, `cli/tests/completion_e2e.rs`, `cli/tests/install_check_e2e.rs`, `cli/tests/verify_e2e.rs`, `docs/product/config.md`.
- Focused commands and outcomes: Focused checks passed.
- Promotion commands and outcomes: `cargo fmt --all -- --check` passed; `cargo test -p ayni-core -p ayni-adapters-common -p ayni-cli --all-features` passed.
- Ayni status and evidence: passed. Canonical Ayni ran once and passed: complete 1/1, 5/5 rows, 282 tests, line coverage 67.33195687084194%, zero failing offenders.
- Accepted deviations: Product config containment text was updated alongside behavior rather than deferred to final docs; no behavior/scope deviation.
-->

## Milestone: milestone-1

- Status: validated
- Changed responsibilities: Core maximum-threshold semantics and size classification; Rust, Go, Node, Python, and Kotlin complexity collection; behavior-preserving decomposition of Go test collection, Rust dependency collection, and CLI verify.
- Explicit paths: `core/src/lib.rs`, `core/src/threshold.rs`, `core/src/size.rs`, `adapters/rust/src/collectors/complexity.rs`, `adapters/rust/src/collectors/deps.rs`, `adapters/go/src/collectors/complexity.rs`, `adapters/go/src/collectors/test.rs`, `adapters/node/src/collectors/complexity.rs`, `adapters/python/src/collectors/complexity.rs`, `adapters/kotlin/src/collectors/complexity.rs`, `cli/src/verify.rs`, `Cargo.lock`.
- Focused commands and outcomes: Focused formatting/tests passed. Exact focused checkout verification passed after decomposition: maximum cyclomatic Go 9, Rust deps 6, CLI verify 10; all `fail_count 0`. One canonical Ayni analysis initially exposed three corrected fail-at-15 functions; no second analysis is claimed.
- Promotion commands and outcomes: Supplied focused and promotion fmt/tests passed.
- Ayni status and evidence: passed after focused remediation; evidence is the exact focused checkout verification above.
- Accepted deviations: Cargo.lock contains approved eventual 0.8.0 local workspace metadata early due Cargo/rust-analyzer regeneration; Task 6.1 remains pending.
## Milestone: milestone-2

- Status: validated
- Changed responsibilities: shared exclusive minimum threshold and configured metric evidence evaluation; typed missing/unparseable coverage setup failures; independent line/branch budgets and fail-closed evidence in all five coverage collectors; Python default collection enables branch coverage; Go configured branch thresholds intentionally fail closed because standard Go coverage lacks compatible branch evidence.
- Changed paths: `core/src/threshold.rs`, `core/src/lib.rs`, `adapters/common/src/failure.rs`, `adapters/rust/src/collectors/coverage.rs`, `adapters/go/src/collectors/coverage.rs`, `adapters/node/src/collectors/coverage.rs`, `adapters/python/src/collectors/coverage.rs`, `adapters/kotlin/src/collectors/coverage.rs`
- Focused commands and outcomes: Focused tests for core/common and every adapter passed.
- Promotion commands and outcomes: `cargo fmt --all -- --check` passed; `cargo test -p ayni-core -p ayni-adapters-common -p ayni-adapters-rust -p ayni-adapters-go -p ayni-adapters-node -p ayni-adapters-python -p ayni-adapters-kotlin --all-features` passed.
- Ayni status and evidence: passed. Canonical checkout Ayni command ran once and passed: complete 1/1 target, 5/5 rows passing, 274 tests, line coverage 66.96238262572729%, no failing offenders.
- Accepted deviations: none
## Milestone: milestone-5
- Status: validated
- Changed responsibilities: true concurrent stdout/stderr streaming and capture; structured spawn/wait/timeout with kill and reap; all collector processes use the configured structured runner, with execution failures becoming failed rows before the adapter boundary; static inventory passes; neutral object-safe `CatalogRuntime` with opaque adapter-managed entries; Node npm/pnpm/Yarn/Bun and Python uv/Poetry/PDM/Pipenv/Hatch/pip detection and workspace resolution, with command/status/install ownership moved privately into adapters; CLI list/apply/check uses one neutral runtime, read-only modes are status-only, apply prepares once with conditional install and reprobe, and timeout/spawn diagnostics are actionable; legacy manager APIs/resolver removed; deterministic manager workspace and timeout E2E coverage; readiness schema remains 0.1.0.
- Changed paths: `.projects/0-8-0-fixes/plan.md`; `core/src/{adapter.rs,catalog.rs,lib.rs}`; deleted `core/src/catalog/python_resolution.rs`; `adapters/common/src/{catalog.rs,collector.rs,exec.rs,failure.rs,lib.rs}`; currently modified adapter files under Rust/Go/Node/Python/Kotlin, including adapter and collector modules plus private Node/Python package-manager/catalog/lib files; `cli/src/{install.rs,install_check.rs}`; `cli/tests/{install_check_e2e.rs,install_managers_e2e.rs}`. Unrelated externally modified `AGENTS.md` is explicitly excluded.
- Focused commands and outcomes: focused validation covered formatting, tests for common, all five adapters, and CLI, workspace check, collector no-compatibility/direct-process assertions, and core/common no-Node/Python-vocabulary assertions; all passed. Workspace Clippy also passed during focused validation.
- Promotion commands and outcomes: the full promotion suite (fmt, tests, workspace check, and associated assertions) passed after remediation.
- Ayni status and evidence: passed. One canonical analysis initially failed only `cli/src/install_check.rs::probe_install_readiness` at cyclomatic complexity 15. The behavior-preserving decomposition was verified with the exact focused checkout command, which passed with maximum cyclomatic complexity 7, cognitive complexity 11, and `fail_count 0`; the full promotion suite was rerun and passed. No second canonical analysis was run.
- Accepted deviations: technical scope refinement split collector migration into 5.2a–e without behavior or scope change; external `AGENTS.md` modification preserved and excluded.
