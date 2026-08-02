---
project_id: "0-8-0-fixes"
status: completed
---

# Result: 0.8.0 fixes

## Acceptance criteria

- [x] Every maximum threshold uses the documented inclusive boundary: a value equal to `warn` produces a warning and a value equal to `fail` produces a failing row. Tests cover core size and the effective complexity metric for Rust, Go, Node, Python, and Kotlin.
- [x] Coverage evaluates `line_percent` and `branch_percent` independently. A configured metric must be present and parseable, values below (not equal to) its minimum fail threshold fail, and values below its warn threshold warn. Rust, Go, Node, Python, and Kotlin each have boundary and missing-evidence regression tests.
- [x] A successful coverage tool invocation with missing/unparseable configured evidence produces a failed coverage row with an actionable typed failure; it cannot become an adapter abort or an offender-free pass. Go explicitly fails closed when branch coverage is configured because the standard Go profile reports statement coverage, not branch coverage.
- [x] Catalog tooling is eligible whenever **any** signal in `for_signals` is enabled. Unit/readiness tests prove test-only, coverage-only, and default-policy behavior, including shared test/coverage requirements such as Vitest and pytest.
- [x] Policy roots reject every lexical parent component (including `./..`), absolute/drive-prefixed paths, and existing symlinks that resolve outside the canonical repository root. Analysis, verification, install, and read-only install checks all use the same containment check.
- [x] Completion is calculated from expected `(language, configured root, signal kind)` row keys. Missing, duplicate, or unexpected rows make completion incomplete and aggregate status fail; failed rows still count as emitted rows. Serialization/deserialization also reject a non-empty complete target count paired with zero rows and structurally inconsistent target row sets.
- [x] Every generated finding command includes the originating config path and an exact configured root selector as well as language and adapter-supported file/package/name selectors. Multi-root tests execute the emitted command and prove that the intended root/package is selected without ambiguity.
- [x] The shared command runner invokes callbacks while the child is still running, preserves captured stdout/stderr, and kills timed-out commands. Collector, catalog status/install, and adapter install-preparation subprocesses use that runner and the configured timeout; deterministic tests cover live progress and timeout failure classification.
- [x] `ayni-core` and `ayni-adapters-common` expose only a language-neutral catalog runtime contract: neither crate defines, exports, imports, or branches on Node/Python package-manager enums, marker files, workspace formats, module-name rules, or manager-specific run/add/install commands.
- [x] The Node adapter owns npm/pnpm/Yarn/Bun marker precedence, `packageManager` parsing, workspace-ancestor resolution, fallback/ambiguity behavior, collector tool commands, dependency preparation, local requirement status, and add-package commands. Table-driven tests cover every manager and direct-root, workspace-ancestor, and fallback resolution without changing `ExecutionResolution` output or install/exec cwd selection.
- [x] The Python adapter owns uv/Poetry/PDM/Pipenv/Hatch/pip marker precedence, uv-workspace resolution, fallback/ambiguity behavior, module normalization, collector/status commands, local package add commands, Python-runtime probes, and uv-tool operations. Table-driven tests cover every manager and direct-root, workspace-ancestor, ambiguous, and fallback resolution.
- [x] Install listing, `--apply`, post-apply foundation validation, and read-only `--check` all call the same neutral adapter catalog runtime. Node/Python direct and workspace fixtures prove listing and check never prepare/install or write, apply prepares the resolved install cwd and installs only missing/outdated enabled entries, and timeout/spawn failures remain actionable rather than being silently reported as ordinary absence. CLI orchestration contains no Node/Python manager branch.
- [x] The eight local workspace package entries in `Cargo.lock` are `0.8.0` without unrelated dependency churn, `cargo check --locked --workspace --all-features` succeeds, and dependency-resolving Cargo invocations in CI, documentation builds, example images, and release builds use `--locked`.
- [x] Pull-request CI runs the checkout binary against Go, Node, Python, and Kotlin example workspaces with real language tools. The gate requires complete expected row sets and no command failures; intentional policy findings in the fixtures may make Ayni exit non-zero but may not be mistaken for tool/setup success.
- [x] User documentation describes both coverage metrics and fail-closed evidence, root containment, exact verification commands, timeout/live-progress behavior, and the actual bare-install default. The README example contains no deleted paths, VitePress navigation labels v3 as current and v2 as historical, and generated `docs/cli.md` matches the checkout CLI.
- [x] Repository policy is not loosened to accommodate the corrected equality semantics. Any newly exposed fail-at-15 dogfooding offender is reduced in code before promotion.
- [x] Each milestone passes its listed focused checks and non-Ayni promotion commands. After a milestone is complete, the orchestrator—not an implementation task—runs the one canonical Ayni full gate for that milestone.

## Final behavior

Centralized inclusive maximum semantics are used by core size and the five complexity adapters. Coverage now records shared core minimum/evidence evaluation and common typed failure mapping, independently enforces line and branch metrics across Rust, Go, Node, Python, and Kotlin, collects Python branch coverage, and documents Go's branch-coverage limitation with fail-closed behavior. Catalog eligibility is shared across enabled signals, and core validation now uses component-based validation with common canonical/symlink containment before detection in analyze, verify, install, and check. Completion is keyed by in-process expected language/root/signal rows, with duplicate, missing, and unexpected rows failing completion while failed rows remain emitted; schema-v3 structural validation rejects inconsistent row sets, while structural validation remains limited to the represented artifact shape. Generated verification supports `verify --root`, materializes config/root commands shell-safely, and is reproducible across multiple roots. Contract display remains filesystem-free, and missing safe roots remain representable. Three newly exposed exact-boundary functions were decomposed without policy changes.

## Architectural impact

Threshold direction is centralized at the core semantic boundary and consumed by size and language adapters; shared coverage evaluation and typed failures keep adapter behavior consistent, while adapter-specific collection remains local. Expected row keys are computed in-process and structural validation protects schema-v3 target accounting without claiming semantic validation beyond the serialized structure. Root-aware, shell-safe command materialization makes generated verification deterministic for multi-root workspaces. Shared eligibility and component-based validation centralize policy decisions, and common canonical/symlink containment protects all pre-detection entry points. Locked dependency resolution is deterministic, and dependency-free typed fixture artifact validation permits only typed policy findings, not incomplete or command-failed collection. Python supplies branch evidence; Go rejects configured branch coverage when its standard profile cannot provide it. Complexity decomposition preserves existing policy and behavior while reducing exact-boundary offenders.

## Known limitations

- Standard Go tooling has no compatible branch-percentage metric, so configured Go branch coverage fails closed.
- Schema v3 serialized artifacts cannot reconstruct policy-specific expected kinds without in-process planning; serialized validation is structural.
- GitHub-hosted four-language real-tool matrix execution is deferred to draft-PR CI; its workflow, validator, and tests are validated locally.
- No local package-registry or language-toolchain installation was performed.

## Technical debt

- Broad mechanical splitting of remaining large modules and policy tightening/mutation enablement remain separate future work.
- No language-specific package-manager logic remains in core/common.

All approved milestones are complete. Final behavior includes the centralized threshold, coverage, containment, completion, command-runner, neutral catalog, adapter-ownership, locked-build, documentation, and CI changes described above. Architectural impact is confined to those responsibilities: policy semantics and structural accounting are centralized, language-specific package-manager behavior remains in adapters, and command execution preserves live progress, output, and timeout classification.

## Ayni

- Status: passed
- Evidence: Passed generated CLI diff, VitePress build, fmt, workspace Clippy with `-D warnings`, workspace all-feature tests, workspace check, locked workspace check, validator 15 tests, focused/E2E suites, static architecture/process assertions, and final canonical Ayni: complete 1/1, 5/5 rows, 310 tests, coverage `70.9894084215965%`, max cyclomatic 14, max cognitive 27, zero failing offenders. Skipped/deferred: GitHub-hosted four-language real-tool job execution awaits PR CI; no local package-registry/toolchain installation was performed.

## Publication

- Draft PR: pending
