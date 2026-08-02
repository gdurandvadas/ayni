---
project_id: "0-8-0-fixes"
title: "0.8.0 fixes"
status: approved
base_branch: "main"
branch: "project/0-8-0-fixes"
created_at: "2026-08-01T19:07:40.850Z"
approved_at: "2026-08-01T19:34:05.829Z"
---

# Plan: 0.8.0 fixes

## Objective

Repair the verified false-green, path-containment, execution, release-integrity, and layer-ownership
defects in Ayni 0.8.0 so an emitted successful repository artifact means that every configured
target and signal produced the required evidence, every configured threshold was enforced with its
documented boundary semantics, and release/CI builds use the checked-in dependency graph. Restore
the documented `core <- adapters/common <- adapters/<language> <- cli` ownership boundary by moving
Node and Python package-manager detection, workspace resolution, and command construction out of
core/common and into their owning adapters. Preserve the v3 artifact wire version and user-visible
Node/Python install/runtime behavior while making generated verification commands copy/paste
reproducible and exercising the non-Rust adapters with their real tools in CI.

## Acceptance criteria

- [ ] Every maximum threshold uses the documented inclusive boundary: a value equal to `warn`
  produces a warning and a value equal to `fail` produces a failing row. Tests cover core size and
  the effective complexity metric for Rust, Go, Node, Python, and Kotlin.
- [ ] Coverage evaluates `line_percent` and `branch_percent` independently. A configured metric must
  be present and parseable, values below (not equal to) its minimum fail threshold fail, and values
  below its warn threshold warn. Rust, Go, Node, Python, and Kotlin each have boundary and missing-
  evidence regression tests.
- [ ] A successful coverage tool invocation with missing/unparseable configured evidence produces a
  failed coverage row with an actionable typed failure; it cannot become an adapter abort or an
  offender-free pass. Go explicitly fails closed when branch coverage is configured because the
  standard Go profile reports statement coverage, not branch coverage.
- [ ] Catalog tooling is eligible whenever **any** signal in `for_signals` is enabled. Unit/readiness
  tests prove test-only, coverage-only, and default-policy behavior, including shared test/coverage
  requirements such as Vitest and pytest.
- [ ] Policy roots reject every lexical parent component (including `./..`), absolute/drive-prefixed
  paths, and existing symlinks that resolve outside the canonical repository root. Analysis,
  verification, install, and read-only install checks all use the same containment check.
- [ ] Completion is calculated from expected `(language, configured root, signal kind)` row keys.
  Missing, duplicate, or unexpected rows make completion incomplete and aggregate status fail;
  failed rows still count as emitted rows. Serialization/deserialization also reject a non-empty
  complete target count paired with zero rows and structurally inconsistent target row sets.
- [ ] Every generated finding command includes the originating config path and an exact configured
  root selector as well as language and adapter-supported file/package/name selectors. Multi-root
  tests execute the emitted command and prove that the intended root/package is selected without
  ambiguity.
- [ ] The shared command runner invokes callbacks while the child is still running, preserves
  captured stdout/stderr, and kills timed-out commands. Collector, catalog status/install, and
  adapter install-preparation subprocesses use that runner and the configured timeout. Every
  collector spawn/wait/timeout error becomes an emitted failed signal row (never an adapter abort),
  with spawn/wait classified as `command_error` and timeout as `timeout`; deterministic tests cover
  live progress and non-zero configured-timeout row classification in all five adapters.
- [ ] `ayni-core` and `ayni-adapters-common` expose only a language-neutral catalog runtime contract:
  neither crate defines, exports, imports, or branches on Node/Python package-manager enums, marker
  files, workspace formats, module-name rules, or manager-specific run/add/install commands.
  `core/src/catalog/python_resolution.rs` and the old manager-bearing `InstallContext` are removed.
- [ ] The Node adapter owns npm/pnpm/Yarn/Bun marker precedence, `packageManager` parsing, workspace-
  ancestor resolution, fallback/ambiguity behavior, collector tool commands, dependency preparation,
  local requirement status, and add-package commands. Table-driven tests cover every manager and
  direct-root, workspace-ancestor, and fallback resolution without changing `ExecutionResolution`
  output or install/exec cwd selection.
- [ ] The Python adapter owns uv/Poetry/PDM/Pipenv/Hatch/pip marker precedence, uv-workspace
  resolution, fallback/ambiguity behavior, module normalization, collector/status commands, local
  package add commands, Python-runtime probes, and uv-tool operations. Table-driven tests cover every
  manager and direct-root, workspace-ancestor, ambiguous, and fallback resolution.
- [ ] Install listing, `--apply`, post-apply foundation validation, and read-only `--check` all call
  the same neutral adapter catalog runtime. Node/Python direct and workspace fixtures prove listing
  and check never prepare/install or write, apply prepares the resolved install cwd and installs only
  missing/outdated enabled entries, and timeout/spawn failures remain actionable rather than being
  silently reported as ordinary absence. CLI orchestration contains no Node/Python manager branch.
- [ ] The eight local workspace package entries in `Cargo.lock` are `0.8.0` without unrelated
  dependency churn, `cargo check --locked --workspace --all-features` succeeds, and dependency-
  resolving Cargo invocations in CI, documentation builds, example images, and release builds use
  `--locked`.
- [ ] Pull-request CI runs the checkout binary against Go, Node, Python, and Kotlin example
  workspaces with real language tools. The gate requires complete expected row sets and no command
  failures; intentional policy findings in the fixtures may make Ayni exit non-zero but may not be
  mistaken for tool/setup success.
- [ ] User documentation describes both coverage metrics and fail-closed evidence, root containment,
  exact verification commands, timeout/live-progress behavior, and the actual bare-install default.
  The README example contains no deleted paths, VitePress navigation labels v3 as current and v2 as
  historical, and generated `docs/cli.md` matches the checkout CLI.
- [ ] Repository policy is not loosened to accommodate the corrected equality semantics. Any newly
  exposed fail-at-15 dogfooding offender is reduced in code before promotion.
- [ ] Each milestone passes its listed focused checks and non-Ayni promotion commands. After a
  milestone is complete, the orchestrator—not an implementation task—runs the one canonical Ayni
  full gate for that milestone.

## Non-goals

- Changing the closed six-signal vocabulary, bumping schema v3, or redesigning offender/finding wire
  shapes. Row-aware generation and structural artifact validation will close this defect without a
  new serialized envelope field.
- Implementing branch coverage that the standard Go coverage profile does not provide. Configured
  Go branch thresholds will be explicit unsupported evidence and therefore fail closed.
- Silently changing `.ayni.toml` thresholds, enabling mutation, or making warnings blocking. The
  current `coverage.fail = 35`, size `fail = 1600` (maximum passing value becomes 1599), complexity
  `fail = 15`, and disabled mutation are product-policy choices requiring a separate approval.
- Making bare `ayni install` auto-detect every language. Existing behavior uses the Rust template
  when no `--language` is supplied; this project documents that behavior and keeps explicit repeated
  `--language` selection for non-Rust/polyglot bootstrap.
- Making intentionally incomplete example repositories pass their quality policies. Real-tool CI
  distinguishes expected policy findings from incomplete collection or command/setup failure.
- A wholesale refactor of all large files or of package-manager ownership for Rust, Go, or Kotlin.
  This Project performs the requested Node/Python ownership migration and the neutral catalog seam
  it requires, but does not redesign unrelated collectors or introduce a general plugin ABI.
- Changing Node/Python package-manager support, marker precedence, workspace/fallback selection,
  catalog package/version choices, `ExecutionResolution` fields, readiness schema `0.1.0`, or
  successful manager command semantics. Characterization tests lock existing behavior; unrelated
  manager feature additions or command-policy changes require separate approval.
- Adding runtime network dependencies. Network access is limited to CI setup/installing the real
  language tools already declared by adapter catalogs.

## Constraints and guardrails

- Preserve `core <- adapters/common <- adapters/<language> <- cli`; core owns threshold and artifact
  semantics, common owns process/path infrastructure, adapters own language parsing/tool behavior,
  and CLI owns root selection, command syntax, orchestration, persistence, and presentation.
- Keep all language-specific detection, runner/package-manager resolution, and tool parsing in the
  owning adapter. Do not add new language conditionals to CLI while implementing `--root`.
- The replacement catalog boundary may carry only neutral execution data (`ExecutionResolution`,
  cwd, timeout, catalog entry, status/error, and a progress callback). Use an opaque adapter-managed
  catalog key/summary where a static entry needs private metadata; do not replace the removed enums
  with manager strings, Node/Python option fields, language-tagged maps, or command templates in
  core/common/CLI.
- Node/Python manager enums, marker parsing, workspace traversal, requirement metadata, status
  interpretation, and command argv construction are private to their adapter crates. Common may
  execute an adapter-built invocation but must not decide which manager, executable, prefix, module,
  package flag, lockfile, or workspace rule applies.
- Keep list and `install --check` read-only, and keep preparation apply-only. A migration shim may
  exist between Tasks 5.3 and 5.6 solely to keep focused checks compilable, but no manager-bearing
  core/common API or dual catalog path may survive Milestone 5 promotion.
- Keep schema version `0.3.0`. Do not add a required wire field as a shortcut for completion
  validation; expected row keys are an in-process planning contract, with wire validation enforcing
  all invariants that can be established from serialized rows and completion counters.
- Preserve failure categories: a tool's non-zero exit remains a repository-code/setup failure as
  appropriate; missing configured metric evidence is `repo_setup_issue` with a stable
  `missing_coverage_metric`/`unparseable_coverage_metric` classification; an Ayni row-accounting
  mismatch is `ayni_internal_issue` and makes completion incomplete.
- Missing coverage evidence is not represented as a fabricated `0%`. Preserve `None` in the typed
  result, attach a typed failure, and fail the row. Genuine measured `0%` remains numeric evidence.
- Do not loosen policy to make the final dogfood gate green. Prefer small extractions and behavior-
  preserving function decomposition for exact-boundary complexity offenders.
- Keep `.ayni/` generated artifacts untracked. Example CI may create them only in ephemeral runners.
- Regenerate `docs/cli.md` after adding `--root`; validate generated content rather than hand-editing
  it. Update architecture, runtime, adapter-contribution, and Node/Python ownership documentation for
  the catalog migration. Keep release/licensing metadata untouched except for the lockfile and locked
  build commands.
- Avoid new crates and package dependencies. The process runner uses the standard library; the CI
  artifact assertion uses Python's standard library.

## Carried changes

The scaffold reported `carried_changes: []`. The inventory is empty, so there are no paths to mark
`adopted` or `unrelated`, and no pre-existing path is eligible to be staged through this Project.

| Path | Initial status | Decision |
| --- | --- | --- |
| _None reported_ | — | — (empty inventory) |

## Research findings

### Repository authority and verified defects

- `ARCHITECTURE.md` assigns product semantics to core, shared command execution/path normalization
  to `adapters/common`, local output parsing to language adapters, and orchestration/CLI syntax to
  CLI. `docs/contributing/adapters.md` additionally requires adapters to emit typed rows, declare
  honest selectors, and use catalog/command infrastructure.
- `README.md:292-296` and `docs/product/config.md:304-331` are unambiguous: maximum metrics warn/fail
  **at or above** their thresholds, while minimum coverage warns/fails only **below** thresholds.
  `core/src/size.rs:119-120` and all five complexity collectors use strict `>` comparisons. Rust's
  `threshold_level` has the reported defect; Go, Node, Python, and Kotlin repeat it and therefore
  belong in the same bounded semantic repair.
- Every coverage policy has both `line_percent` and `branch_percent`
  (`core/src/policy.rs:610-615`), and contract display already projects both. However, every adapter
  serializes only line thresholds into its budget and enforcement is line/headline-only. Rust, Go,
  and Node use `Option::is_none_or`, so missing measured evidence passes a configured threshold;
  Python determines pass only from per-file offenders and can pass missing totals/files; Kotlin can
  finish with no counters and no offenders. Rust, Node, and Kotlin already parse branch values;
  Python currently discards branch values; Go emits `branch_percent: None`.
- The official Go `cmd/cover` documentation describes profiles generated by `go test
  -coverprofile` and approximate basic-block/statement instrumentation, while official Go reporting
  reports “percent statements covered”; it does not expose a branch percentage compatible with
  Ayni's branch metric. Primary source: <https://pkg.go.dev/cmd/cover> and
  <https://go.dev/doc/build-cover>. Therefore a configured Go branch threshold must fail closed,
  not be silently mapped to statement coverage.
- pytest-cov's primary documentation defines `--cov-branch` as the switch that enables branch
  coverage: <https://pytest-cov.readthedocs.io/en/latest/config.html>. Coverage.py's documented JSON
  totals include `percent_statements_covered` and, when branch measurement is enabled,
  `percent_branches_covered`: <https://coverage.readthedocs.io/en/latest/faq.html>. Python can
  therefore provide both configured metrics without a new dependency.
- `cli/src/install.rs:312-320` uses `.all()` over `CatalogEntry::for_signals`. Shared requirements
  such as Node `vitest` and Python `pytest` are consequently omitted when only one of test/coverage
  is enabled. The same predicate feeds listing, apply, foundation validation, and read-only
  readiness, so one predicate fix plus tests covers every install mode.
- Task 3.1 readiness-fixture research confirms that the failing
  `shared_requirements_are_ready_when_one_associated_signal_is_enabled` assertion is a fixture
  expectation error, not a second eligibility defect. `install --check` is globally `ready` only
  when every selected target is detected/resolved and every selected catalog requirement is
  current (`docs/product/runtime.md:54-90`, `cli/src/install_check.rs:87-190`). The fixture creates
  only `package.json`, so the configured Python target is undetected; its empty Node manifest also
  makes `vitest` missing, and Python package status would otherwise depend on whichever interpreter
  and modules happen to be on the host. A non-zero check can and does still emit the complete
  readiness JSON on stdout, so the eligibility regression must inspect target requirements rather
  than require global process success.
- The deterministic correction is a table-driven JSON E2E with both adapter-owned detection
  manifests (`package.json` and `pyproject.toml`) and a fixture-local empty directory as the child
  `PATH`. This keeps all status probes deterministically `missing` without fake executables,
  installed-package manifests, package-manager preparation, or network access. Each case expects a
  non-zero process, empty stderr, `state = "not_ready"`, detected/resolved Node and Python targets,
  and exact catalog-order requirement names. Test-only expects Node `[node, vitest]` and Python
  `[python, pytest, pytest-json-report]`; coverage-only expects Node
  `[node, vitest, @vitest/coverage-v8]` and Python `[python, pytest, pytest-cov, coverage]`; omitted
  `[checks]` exercises `PolicyChecks::default` and expects Node
  `[node, vitest, @vitest/coverage-v8, eslint, @stylistic/eslint-plugin]` and Python
  `[python, pytest, pytest-json-report, pytest-cov, coverage, complexipy]`. Mutation-only entries
  remain absent under defaults. In every case, assert the shared `vitest` and `pytest` records are
  present with catalog associations `[test, coverage]` and status `missing`; do not infer their
  eligibility from foundation requirements or from the aggregate readiness state. A separate
  all-current fixture would test a different status/readiness contract and is unnecessary here.
- Root normalization rejects `..` and `../x`, but `./..` survives because it is neither equal to
  `..` nor matched by the string fragments. Operational paths are then formed with unchecked
  `repo_root.join(root)` in discovery/install. Existing file-selector code demonstrates the desired
  canonical, symlink-aware containment pattern, and `core/src/size.rs` already contains a focused
  canonical containment check for selected files.
- Schema v3 explicitly promises that a complete target emitted every requested row
  (`docs/product/signals/v3.md:39-67`). `RunCompletion::validate` checks only counters/issues, while
  `RunArtifact::aggregate` treats zero rows as “all rows pass”; `run_collect_with_ui` marks every
  runnable target completed based on target count rather than emitted row keys. A complete,
  expected-target artifact with zero rows can therefore aggregate pass despite the written contract.
- Generated finding commands are rendered in `cli/src/main.rs:1339-1358` from only signal,
  language, and adapter target. The originating `metadata.config_path` is available but omitted.
  `verify::select_non_file_targets` rejects package/name selectors whenever a language has multiple
  configured roots, and the CLI has no root selector, so an analyze-produced package command can be
  inherently non-reproducible.
- `adapters/common/src/exec.rs` claims callbacks fire “as [lines] arrive,” but waits for process exit
  before draining either reader channel. Direct subprocess bypasses include Go coverage post-
  processing, Go deps, Rust complexity/deps metadata and analysis (using the fallback timeout), Node
  install preparation, and catalog check/install/status probes. Catalog install commands inherit
  output, but have no timeout. This violates `docs/product/config.md:283-297` and the module's own
  “every adapter command” claim.
- Task 5.2 scope-review research after its partial implementation found that the direct collector
  bypass inventory is now closed: no production file below an adapter `src/collectors/` directory
  creates a process directly. The active partial changes route Rust metadata/complexity/deps, Go
  coverage post-processing/deps, and Kotlin Gradle task probing through the context-aware common
  runner, and `adapters/common/src/exec.rs` now exposes structured spawn/wait/timeout errors. Those
  paths are Project work, not scaffold-carried changes, and must be preserved while Task 5.2 is
  completed.
- Closing direct bypasses is insufficient for the approved failure contract. The compatibility
  `run_command_for_context` and `run_command_for_context_streaming` wrappers call the structured
  runner but immediately stringify its error. Rust still uses those wrappers in test, coverage,
  complexity, deps, and mutation; Go uses one through `collectors/util.rs` in test, primary coverage,
  complexity, and mutation; Node uses them in test, coverage, complexity, and mutation; Python uses
  them in test, coverage, complexity, and both mutation commands; Kotlin uses them in every primary
  test, coverage, complexity, deps, and mutation command. Node and Python deps are source-native and
  have no subprocess to migrate. Go deps, Go secondary coverage, and Kotlin coverage task probing
  already demonstrate the structured mapping, but do so with duplicated signal-local row builders.
- All five `collectors/mod.rs` implementations currently turn collector `String` errors into
  `AdapterError`. Analyze and verify then recover at their CLI orchestration boundaries by creating a
  failed row classified only as `adapter_error`. Thus a compatibility-wrapper timeout does not abort
  the whole analyze run, but it does cross the adapter boundary as an abort and loses the stable
  `timeout` classification, exact command, and captured runner context. This contradicts
  `docs/product/runtime.md:92-117` (tool failures should be rows; adapter aborts are for invalid
  contracts/internal faults) and `docs/product/config.md:285-298` (timeout is a failed row). The
  scope-review option to declare row synthesis solely a CLI responsibility is therefore rejected.
- The bounded solution is an adapter-common internal collector error seam, not a core trait or CLI
  contract change. A `CollectorError` distinguishes `Execution(Box<ExecutionError>)` from ordinary
  adapter strings, and a common `finish_collection` helper maps only the execution variant to a
  typed failed row before `SignalCollector` returns. Ordinary policy, selection, filesystem, and
  parser errors continue to become `AdapterError` exactly as today. This seam applies equally to
  normal, streaming, and focused collection, so neither analyze nor verify can misclassify a runner
  timeout.
- Structured execution returns normal `Output` for a child that exits non-zero. Consequently the
  migration must not route non-zero status through `CollectorError`: each collector retains its
  existing signal/tool-specific logic and classifications (`import_error`, `collection_error`,
  `no_tests`, `command_error`, missing report/evidence, and existing signal category). Only runner-
  owned spawn/wait/timeout failures take the new common failed-row path. A source assertion plus one
  public collector timeout test per adapter closes the inventory without requiring a slow timeout
  process for every call site.
- `Cargo.toml` declares workspace version `0.8.0`; the eight local workspace entries in `Cargo.lock`
  remain `0.7.0`. The ninth `0.7.0` match is the unrelated registry package `ratatui-macros` and must
  not be changed. Current quality, docs, release, and example Docker build commands omit `--locked`.
  Cargo's primary documentation says `--locked` exits if the lockfile is missing or resolution would
  change and recommends it for deterministic CI: <https://doc.rust-lang.org/cargo/commands/cargo-check.html#option-cargo-check---locked>.
- Existing CI dogfoods only Rust. The repository already has real Go/Node/Python/Kotlin mono
  fixtures, per-language Dockerfiles, configured policies, and catalog declarations. GitHub-hosted
  runners can install the declared toolchains; all four are feasible. The fixtures intentionally
  contain policy scenarios, so CI must inspect typed completion/failure evidence rather than simply
  expect every `analyze` process to exit zero.

### Documentation and policy assessment

- Bare install behavior is code-defined: `default_policy_toml` returns the Rust template when the
  language set is empty. The generic README quick start does not disclose that; this plan corrects
  documentation rather than changing onboarding semantics during an integrity repair.
- The README example is stale: it cites deleted `adapters/rust/src/tools/signals.rs` and carries a
  point-in-time offender list. The final example should be refreshed from the corrected checkout
  artifact and clearly labeled illustrative.
- `docs/.vitepress/config.ts` links v2 and v1 beneath Signals but omits current v3 in both nav and
  sidebar, despite `docs/index.md` and `docs/product/signals.md` naming v3 current.
- Root `.ayni.toml` has line coverage `{ warn = 40, fail = 35 }`, size
  `{ warn = 1000, fail = 1600 }`, complexity fail 15/30, and mutation disabled. Local line counts
  show the pressure that policy currently permits (`cli/src/main.rs` 1534 lines,
  `core/src/policy.rs` 1192, `core/src/signal.rs` 1144). The README's dogfood snapshot reports a
  cyclomatic value of exactly 15 as a warning; corrected semantics would make such a current value a
  failure. These facts justify code decomposition, not an unapproved policy change.

### Maintainability and boundary assessment

- Correctness work should create narrow modules rather than further grow `cli/src/main.rs`,
  `core/src/policy.rs`, and `core/src/signal.rs`: a core threshold evaluator, common contained-path
  and execution helpers, and CLI completion/verification-command modules are cohesive seams directly
  required by this objective.
- `core/src/catalog.rs` currently owns and publicly re-exports `NodePackageManager`,
  `PythonPackageManager`, both marker detectors, both manager command builders, Python module-name
  normalization, and `InstallContext` fields for both enums; `core/src/catalog/python_resolution.rs`
  owns uv-workspace ancestry and ambiguity. This directly violates `ARCHITECTURE.md:18-23`,
  `AGENTS.md`, and `docs/contributing/adapters.md:7-18`, all of which assign local detection,
  runner/package-manager resolution, and tool invocation to the language adapter rather than core.
  The requested migration is therefore adopted scope: it is necessary to satisfy the stated
  architecture-restoration objective, not unrelated cleanup.
- The violation extends beyond definitions. `adapters/common/src/catalog.rs` imports both core
  manager types and constructs Node add-package, Python add/run/status, Python-runtime, and uv-tool
  commands; `cli/src/install.rs:299-305` reconstructs both manager enums from the generic
  `ExecutionResolution.runner`. This forces common and CLI to know language-specific execution
  behavior even though the adapter already resolved it. Core's Node/Python-specific
  `Installer::{NpmGlobal,NodePackage,PythonPackage,PythonRuntime,UvTool}` variants also leak
  adapter-private command/package metadata into the shared contract.
- Node ownership is already partially demonstrated in `adapters/node/src/adapter.rs`: it owns
  package-workspace ancestry, emits the neutral `ExecutionResolution`, and selects `install_cwd`
  versus `exec_cwd`. Python's adapter already consumes the same neutral resolution shape. The safe
  migration keeps that public runtime shape and moves each enum/detector/constructor beside its
  adapter, rather than encoding manager identity into a new core field.
- The install call graph is singular and bounded: list, apply, foundation validation, and
  `install --check` iterate `LanguageAdapter::catalog`; apply alone calls `prepare_install` before
  status/install. A neutral adapter catalog-runtime trait can preserve that orchestration while
  common supplies only generic timeout-aware process execution. Node's manifest/`node_modules`
  status logic and Python's import probe/uv-tool logic then remain private implementation details.
- Existing characterization points define compatibility: Node lockfile precedence is pnpm, Yarn,
  npm, then Bun, followed by `packageManager`, ancestor workspace, and npm fallback; Python direct
  precedence is uv, Poetry, PDM, Pipenv, Hatch, then pip-compatible manifests, with uv workspace
  ancestry and pip fallback. `docs/product/runtime.md:6-40`, `docs/adapters/node.md:3-15`, and
  `docs/adapters/python.md:3-15` make the resulting runner, resolution kind, and install/exec cwd
  behavior public. No external source is needed to choose the ownership boundary or compatibility
  behavior because repository architecture and tests are authoritative.
- Further mechanical splits of policy/signal/report files are also deferred unless a touched
  function cannot be tested safely in place. This release extracts behavior seams, not arbitrary
  line-count shards.

## Architecture and approach

### Threshold and coverage semantics

Add a small core threshold module that is the single authority for inclusive maximum classification
and exclusive minimum classification. It accepts a measured value plus typed warn/fail levels and
returns pass/warn/fail; coverage additionally evaluates a configured metric as present, below-warn,
or below-fail. Core does not parse language output.

Each coverage adapter will map native output into `line_percent` and `branch_percent`, then call the
same evaluator for each configured policy metric. Row pass is:

1. the external tool/report operation succeeded;
2. every configured metric is present and finite; and
3. no configured metric is below its fail threshold.

Exactly-equal minimum values pass. Missing required values retain `None`, set coverage status to
`error`, and attach a typed setup failure naming the missing metric. Numeric low values produce
normal warn/fail coverage findings. `CoverageResult.percent` remains line-first, branch-fallback for
wire compatibility; pass/fail never uses that headline as a substitute for a separately configured
metric. Budgets expose both configured line and branch thresholds. Python enables branch collection
for its default command when branch policy is configured and parses documented statement/branch
totals. Custom overrides remain responsible for producing the native report; absent metrics fail
closed. Go documents and tests its branch limitation.

### Root containment

Core lexical normalization will iterate path components rather than search strings, rejecting every
parent/root/prefix component before policy use. A common operational helper will canonicalize the
repository and every existing configured root and require `root.strip_prefix(repository)` to
succeed. Nonexistent but lexically safe roots remain representable so completion can report a
detection issue; existing symlinks cannot escape. CLI calls this once before adapter detection in
analyze, verify, install/apply, and install-check. `contract display` preserves its documented no-
filesystem behavior and performs only lexical policy validation.

### Row-aware completion without a schema bump

Extract CLI completion planning around an internal `ExpectedRowKey { language, configured_root,
kind }`. Analyze creates the Cartesian set of runnable configured targets and enabled signal kinds;
verify creates one key per selected target for the requested kind. Collected rows are keyed by
language, normalized `scope.path` (`None` means `.`), and kind. Completion is complete only when the
sets are equal and duplicate-free. A missing key produces an ordered collection-stage issue for its
target and does not increment that target's completed count; an emitted failing row does.

Core wire validation will add the schema-compatible structural checks available without loading a
repository policy: a complete artifact with `expected_targets > 0` must contain rows, row keys must
be unique, the number of represented completed targets must reconcile, and completed repository
targets must have a consistent non-empty signal-kind set. CLI's explicit expected set supplies the
stronger generation-time guarantee, including detecting a signal omitted from every target. No
serialized field or schema-version change is required.

### Reproducible verification interface

Add `--root <configured-root>` to every `verify` subcommand as a CLI orchestration selector. It is
validated against the selected language's normalized policy roots before adapter selector
validation, then narrows planning to exactly one target. Adapters remain unaware of CLI root syntax.
Finding command materialization receives artifact config metadata and row scope, and always renders
shell-quoted `--config`, `--language`, and `--root` before supported file/package/name selectors.
This resolves same-name packages/tests across configured roots without inventing language-specific
logic. Extract rendering and quoting into `cli/src/verification_command.rs`; regenerate CLI docs.

### Unified subprocess behavior

Refactor common execution into one polling event loop that drains tagged stdout/stderr chunks while
checking timeout, emits complete lines before process exit, retains partial final lines, captures
both streams, and kills/waits on timeout. A structured execution error distinguishes spawn, wait,
and timeout failures so adapters/CLI can emit stable classifications.

Use the context timeout for every collector subprocess, including secondary metadata/report
commands. Make neutral catalog status/install/preparation operations carry the same timeout and
return structured spawn/wait/timeout diagnostics instead of collapsing execution faults to
“missing.” Status probes can use a no-op progress callback, while preparation/install commands
forward live lines to the existing CLI output. No asynchronous runtime or new crate is introduced.

For collector execution, keep the public core `SignalCollector` contract unchanged. Add
`adapters/common/src/collector.rs` with an internal `CollectorError`, `CollectorResult`, and
`finish_collection(language, kind, context, result)` boundary. `From<Box<ExecutionError>>` preserves
the structured runner error; `From<String>` preserves ordinary adapter errors. `finish_collection`
returns `Ok(failed_row)` only for the structured execution variant, using
`command_failure_from_execution_error`; it maps the ordinary variant to the existing
`AdapterError`. The failed row uses the requested language/kind/scope, the existing schema-v3 result
variant and neutral empty budget/offenders used by orchestration failures, `pass = false`, exact
runner command/cwd/exit context, and classifications `command_error` for spawn/wait or `timeout` for
timeout. It does not parse an error string.

Migrate each process-bearing collector and collector utility to
`run_command_for_context_structured` or `run_command_for_context_streaming_structured` and propagate
`ExecutionError` through `CollectorError`. Each adapter's `collectors/mod.rs` must call
`finish_collection` for normal, streaming, and focused paths. Source-only size collectors and
Node/Python source-native deps collectors may retain string-returning internals and explicitly map
those strings into `CollectorError`; they do not gain subprocesses. Keep the string compatibility
runner APIs temporarily for catalog/install callers until Tasks 5.3–5.6 migrate those paths, but no
collector may call them after Task 5.2e.

### Adapter-owned package managers and neutral catalog runtime

Replace the manager-bearing `InstallContext` and direct CLI calls into common catalog functions with
one core-defined, object-safe, language-neutral `CatalogRuntime` contract returned by
`LanguageAdapter::catalog_runtime(&self) -> &dyn CatalogRuntime`. `catalog()` continues to return
ordered declarative entries; the runtime supplies their behavior. Its operations are:

1. `status(entry, execution, timeout) -> Result<ToolStatus, CatalogOperationError>`;
2. `prepare(execution, timeout, on_line) -> Result<(), CatalogOperationError>`, invoked only by
   apply and defaulting to no work; and
3. `install(entry, execution, timeout, on_line) -> Result<(), CatalogOperationError>`.

The context consists only of the existing neutral `ExecutionResolution`, timeout, entry, and
callback. `CatalogOperationError` preserves operation (`status`, `prepare`, or `install`), generic
spawn/wait/timeout classification, formatted command/cwd when a process was attempted, and message.
A command exiting non-zero during a presence probe maps to `ToolStatus::Missing`; inability to spawn,
wait, or enforce the timeout is an operation error. Apply and readiness surface that diagnostic with
the language/root/requirement; they do not silently reinterpret it as a successful probe. The
readiness JSON version and status vocabulary stay at `0.1.0`: a probe error produces the existing
non-current requirement plus a requirement issue carrying the actionable diagnostic.

Core keeps generic catalog identity, signal mapping, status, and installer contracts, but replaces
the Node/Python-specific `Installer::{NpmGlobal,NodePackage,PythonPackage,PythonRuntime,UvTool}` with a neutral
`Installer::AdapterManaged { key, summary }`. The opaque key is interpreted only by the owning
adapter and the summary supplies CLI list text without exposing private metadata. Common implements
a reusable default runtime for the remaining generic installer variants and executes fully built
commands through the unified runner; it has no Node/Python branches. Rust, Go, and Kotlin delegate to
that runtime unchanged. Unknown adapter-managed keys and attempts to pass them to the generic
runtime are `ayni_internal_issue`-equivalent catalog contract errors, never no-ops.

Create private `package_manager` modules in Node and Python. They own enums and all mappings between
manager identity and executable/argv. Preserve the following compatibility behavior in
characterization tables:

- Node detects `pnpm-lock.yaml`, `yarn.lock`, `package-lock.json`, then `bun.lock`/`bun.lockb`, then
  the `packageManager` field; direct resolution wins, conflicting direct/ancestor managers mark the
  direct result ambiguous, an ancestor manifest with `workspaces` supplies workspace resolution,
  and a manifest with no manager falls back to npm. Tool prefixes remain `npm exec --`, `pnpm exec`,
  `yarn exec`, and `bun x`; dependency adds retain npm `install --save-dev`, pnpm `add -D`, Yarn
  `add --dev`, and Bun `add -d`; apply preparation remains `<manager> install` at `install_cwd`.
- Python detects `uv.lock`, `poetry.lock`, `pdm.lock`, `Pipfile.lock`, `hatch.toml`, then
  `pyproject.toml`/`requirements.txt`; uv workspace ancestry and existing direct-versus-ancestor
  ambiguity rules remain unchanged, and unresolved manifest cases retain the Python/pip fallback.
  uv/Poetry/PDM/Pipenv/Hatch keep their existing `run` prefixes, pip-compatible execution remains
  `python -m <normalized-module>`, and add-package argv remains uv `add --dev`, Poetry
  `add --group dev`, PDM `add --dev`, Pipenv `install --dev`, and the existing Hatch/pip
  `-m pip install` fallback. Python runtime probes remain `python3` then `python`, and complexipy
  remains managed through `uv tool`.

Node's adapter runtime checks local catalog requirements against dependency sections plus installed
`node_modules`; Python's runtime builds manager-aware import probes and uv-tool probes. Both use
private typed requirement tables keyed by the opaque catalog key. CLI remains responsible only for
policy filtering, traversal, output, and deciding which neutral operation to request. List and check
call status only; apply calls prepare once per resolved target, status, conditional install, then the
existing foundation status validation. Workspace tests assert preparation/installation use
`install_cwd` while collector commands continue to use `exec_cwd`.

### Risks and mitigations

- **Public contract cutover:** Removing exported manager types and changing `LanguageAdapter` is a
  source-breaking Rust API change across every workspace crate. Mitigation: introduce the neutral
  runtime and generic delegator first, migrate Node/Python in parallel, cut CLI over once, then make
  absence of old exports/identifiers a milestone promotion assertion; no dual API survives the gate.
- **Behavior drift hidden by strings:** `ExecutionResolution.runner` can reproduce an executable but
  not the manager's full command grammar. Mitigation: private typed manager enums remain in each
  adapter, and exhaustive command tables compare complete `(program, argv, cwd)` for every manager
  rather than reconstructing behavior in CLI/common.
- **Workspace installs at the wrong level:** A leaf target can execute in one cwd while dependencies
  belong at an ancestor. Mitigation: direct/ancestor fixtures assert both `install_cwd` and
  `exec_cwd`, fake-manager invocation logs assert actual apply cwd, and list/check snapshots assert
  no preparation side effect.
- **Readiness regression:** Distinguishing timeout/spawn errors from ordinary missing tools could
  accidentally alter versioned JSON or make check mutate state. Mitigation: retain readiness
  `0.1.0` fields/statuses, add diagnostic issues through the existing shape, byte-snapshot JSON, and
  snapshot the fixture filesystem before/after list and check.
- **Overlap with subprocess work:** Migrating catalog APIs before finalizing timeout/streaming would
  duplicate execution paths. Mitigation: Tasks 5.3–5.5 now follow the complete sequential collector
  migration, Task 5.6 deletes legacy catalog bypasses, and Milestone 5 promotes only after workspace
  check and static boundary assertions.
- **Collector migration breadth:** A partial switch can appear correct because CLI fallback still
  emits an `adapter_error` row. Mitigation: the common collector boundary is exercised through each
  adapter's public `SignalCollector`, each adapter has an exact one-second timeout test asserting an
  `Ok` failed row with classification `timeout`, and a promotion assertion rejects both compatibility
  runner calls and direct `Command` creation anywhere under adapter collector directories.
- **Large combined milestone:** The architecture cutover touches core, common, five adapters, and
  CLI. Mitigation: retain stable task boundaries and focused package checks, permit only a temporary
  compile-compatible shim within the milestone, and reserve the one Ayni full gate for completed
  milestone promotion.

### Release and real-tool CI

Refresh only local workspace package metadata in `Cargo.lock`, inspecting that no registry package
resolution changes. Add `--locked` to Cargo check/test/clippy/build/run/doc commands that resolve the
workspace in workflows, release builds, and example Dockerfiles (formatting is not a dependency-
resolution operation).

Add a PR example matrix for Go, Node, Python, and Kotlin. Each job sets up its official toolchain,
builds the checkout CLI with `--locked`, applies/checks catalog tooling in the mono fixture, runs the
fixture's configured analysis, and feeds `.ayni/last/signals.json` to a standard-library validation
script. The script requires schema v3, complete reconciled targets, the exact five enabled signal
kinds per expected target, and no `result.failure`. It permits aggregate failure only when caused by
typed policy findings, because examples intentionally contain such scenarios. Kotlin's Gradle/Maven,
Node's npm, Python's pip/uv, and Go's module downloads are explicit CI-only network use.

## Validation strategy

- Task checks use package/test filters and repository-native scripts only. Do not use `ayni analyze`
  as preflight, task validation, or an extra promotion command.
- Milestone promotion commands listed below are deliberately non-Ayni. When a milestone is complete,
  the orchestrator runs exactly one canonical checkout gate,
  `cargo run -p ayni-cli -- analyze --config ./.ayni.toml`; that implicit Ayni run is not repeated in
  a milestone's command list.
- The final milestone runs the repository's classic checks exactly as documented, plus the explicit
  locked check. A non-zero orchestrator Ayni gate is repaired without changing `.ayni.toml`.
- Runner timeout/progress tests use deterministic fixture processes: one emits a line, signals that
  the callback observed it, then remains alive; the common runner unit uses a sub-second timeout.
  Adapter collector tests use the valid non-zero configured value
  `execution.tool_timeout_seconds = 1` and a fixture child that blocks indefinitely, then assert the
  public collector returns `Ok` with `pass = false`, the requested signal result contains a failure
  classified `timeout`, and command/cwd are retained. Do not use invalid zero-second policy values,
  wall-clock sleep as the only synchronization, or assert only an error string.
- Package-manager tests are table-driven characterization tests. Every Node and Python manager must
  assert marker/executable recognition, collector run argv, requirement status invocation or local
  inspection, add-package argv, and fallback. Separate ancestry fixtures assert direct,
  workspace-ancestor, ambiguity, `install_cwd`, and `exec_cwd`; do not infer architecture coverage
  from one happy-path manager.
- CLI install E2E tests put deterministic fake manager executables on a fixture-local `PATH` and
  record invocations outside the configured repository roots but inside the test temp repository.
  Snapshots prove list/check do not call prepare/install or mutate fixtures; apply proves the
  resolved workspace cwd, enabled-entry filtering, conditional install, re-probe, progress, and
  timeout diagnostics. They do not access package registries.
- Milestone 5 promotion includes a source-boundary assertion over Rust sources in `core/` and
  `adapters/common/` for the removed Node/Python manager identifiers and marker/command vocabulary,
  plus a collector-source assertion rejecting direct process creation and the string compatibility
  context-runner APIs. Temporary compatibility exports are permitted only between that milestone's
  tasks and fail its promotion check.
- Coverage tests exercise equal-warn, equal-fail, below-fail, measured zero, absent metric, and
  malformed native report cases. Each adapter must have a wiring-level test, not only shared-helper
  tests.
- CI integration validates typed artifacts rather than grepping console text, so expected policy
  failures cannot hide missing rows or command failures.

## Milestones

### Milestone 1: Correct threshold boundaries and keep dogfooding honest

- Status: pending
- Depends on: none
- Acceptance:
  - [ ] Core size and all effective adapter complexity thresholds fail/warn at equality.
  - [ ] Any repository function newly failing at the configured exact boundary is decomposed without
    changing policy or behavior.
- Validation:
  - `cargo fmt --all -- --check`
  - `cargo test -p ayni-core -p ayni-adapters-rust -p ayni-adapters-go -p ayni-adapters-node -p ayni-adapters-python -p ayni-adapters-kotlin --all-features`

#### Task 1.1: Centralize threshold direction and fix size equality

- Tier: M
- Tier rationale: The documented comparator behavior is exact and local, but extracting one product-
  semantic helper and migrating size is ordinary multi-file work rather than a one-line isolated fix.
- Depends on: none
- Affected responsibilities: Core threshold semantics; core size classification and counters.
- Expected paths: `core/src/lib.rs`, `core/src/threshold.rs` (new), `core/src/size.rs`.
- Focused checks:
  - `cargo test -p ayni-core threshold::tests --all-features`
  - `cargo test -p ayni-core size::tests::maximum_threshold_equality --all-features`

#### Task 1.2: Apply inclusive maximum semantics to every complexity adapter

- Tier: M
- Tier rationale: This repeats one demonstrated core rule across five bounded collector modules and
  adds parser-level boundary fixtures; it does not alter tool contracts.
- Depends on: 1.1
- Affected responsibilities: Adapter normalization of native complexity metrics; warn/fail counters;
  repository code at newly exposed exact fail boundaries.
- Expected paths: `adapters/rust/src/collectors/complexity.rs`,
  `adapters/go/src/collectors/complexity.rs`, `adapters/node/src/collectors/complexity.rs`,
  `adapters/python/src/collectors/complexity.rs`,
  `adapters/kotlin/src/collectors/complexity.rs`, and, if still measured at 15,
  `adapters/rust/src/collectors/deps.rs`.
- Focused checks:
  - `cargo test -p ayni-adapters-rust complexity::tests::maximum_threshold_equality --all-features`
  - `cargo test -p ayni-adapters-go complexity::tests::maximum_threshold_equality --all-features`
  - `cargo test -p ayni-adapters-node complexity::tests::maximum_threshold_equality --all-features`
  - `cargo test -p ayni-adapters-python complexity::tests::maximum_threshold_equality --all-features`
  - `cargo test -p ayni-adapters-kotlin complexity::tests::maximum_threshold_equality --all-features`

### Milestone 2: Make configured coverage evidence fail closed in all adapters

- Status: pending
- Depends on: Milestone 1
- Acceptance:
  - [ ] Line and branch budgets are serialized and independently enforced with minimum semantics.
  - [ ] Missing/unparseable required evidence yields a failed typed row in all five adapters.
  - [ ] Python collects/parses branch evidence; Go rejects configured branch evidence as unavailable.
- Validation:
  - `cargo fmt --all -- --check`
  - `cargo test -p ayni-core -p ayni-adapters-common -p ayni-adapters-rust -p ayni-adapters-go -p ayni-adapters-node -p ayni-adapters-python -p ayni-adapters-kotlin --all-features`

#### Task 2.1: Define shared configured-metric evaluation and failure scaffolding

- Tier: M
- Tier rationale: The evaluator is a bounded product-semantic extension of Task 1.1 with one shared
  adapter failure mapping; no language parser or wire type changes.
- Depends on: 1.1
- Affected responsibilities: Core minimum-threshold classification; common typed setup-failure
  construction for absent/unparseable metric evidence.
- Expected paths: `core/src/threshold.rs`, `core/src/lib.rs`,
  `adapters/common/src/failure.rs`, `adapters/common/src/lib.rs`.
- Focused checks:
  - `cargo test -p ayni-core threshold::tests::minimum_boundaries --all-features`
  - `cargo test -p ayni-adapters-common failure::tests::coverage_metric_failure --all-features`

#### Task 2.2: Enforce Rust and Go line/branch policy

- Tier: M
- Tier rationale: Both collectors are bounded; Rust already parses both metrics, while Go needs only
  explicit unavailable-branch behavior and timeout-safe post-processing.
- Depends on: 2.1
- Affected responsibilities: cargo-llvm-cov parsing/evaluation; Go statement profile parsing;
  coverage budgets, result status, and typed failure behavior.
- Expected paths: `adapters/rust/src/collectors/coverage.rs`,
  `adapters/go/src/collectors/coverage.rs`.
- Focused checks:
  - `cargo test -p ayni-adapters-rust coverage::tests --all-features`
  - `cargo test -p ayni-adapters-go coverage::tests --all-features`

#### Task 2.3: Enforce Node and Python line/branch policy

- Tier: M
- Tier rationale: Node already exposes both native values; Python requires a documented command flag
  and bounded JSON summary fields, but remains within two adapter collectors.
- Depends on: 2.1
- Affected responsibilities: Vitest summary evaluation; pytest-cov command construction; coverage.py
  statement/branch JSON parsing; missing report/metric classification.
- Expected paths: `adapters/node/src/collectors/coverage.rs`,
  `adapters/python/src/collectors/coverage.rs`.
- Focused checks:
  - `cargo test -p ayni-adapters-node coverage::tests --all-features`
  - `cargo test -p ayni-adapters-python coverage::tests --all-features`

#### Task 2.4: Enforce Kotlin line/branch policy and counter presence

- Tier: M
- Tier rationale: JaCoCo/Kover counters are already parsed and aggregated; the task adds required-
  counter validation and shared threshold evaluation in one collector.
- Depends on: 2.1
- Affected responsibilities: Kotlin XML counter parsing, aggregate metrics, budgets, and failed-row
  construction.
- Expected paths: `adapters/kotlin/src/collectors/coverage.rs`.
- Focused checks:
  - `cargo test -p ayni-adapters-kotlin coverage::tests --all-features`

### Milestone 3: Repair installer eligibility and contain configured roots

- Status: pending
- Depends on: Milestone 2
- Acceptance:
  - [ ] Shared catalog tools are selected when any associated signal is enabled.
  - [ ] All operational entry points reject canonical root escape while retaining normal missing-root
    completion/readiness behavior.
- Validation:
  - `cargo fmt --all -- --check`
  - `cargo test -p ayni-core -p ayni-adapters-common -p ayni-cli --all-features`

#### Task 3.1: Change catalog eligibility to any enabled associated signal

- Tier: S
- Tier rationale: The exact existing predicate and all four consumers are demonstrated; the behavior
  change is `.all()` to `.any()` while retaining vacuous eligibility for an empty association list,
  plus table-driven predicate and readiness regression tests using existing JSON fixture patterns.
- Depends on: 2.4
- Affected responsibilities: Install listing/apply/foundation/readiness requirement selection.
- Expected paths: `cli/src/install.rs`, `cli/src/tests.rs`, `cli/tests/install_check_e2e.rs`.
- Fixture contract:
  - Keep the unit matrix for test-only, coverage-only, one-enabled shared entries, all-disabled
    shared entries, and empty `for_signals`; the predicate is true when the list is empty or at least
    one associated signal is enabled, and false otherwise.
  - Replace the global-ready E2E expectation with one table-driven
    `shared_requirements_follow_any_enabled_signal` check covering test-only, coverage-only, and an
    omitted-`[checks]` default policy. Create both `package.json` and `pyproject.toml`, pass a
    fixture-local empty directory as the child `PATH`, parse JSON despite the expected non-zero exit,
    and assert the exact per-language requirement lists documented in Research findings.
  - Assert `vitest` and `pytest` themselves have `signals = ["test", "coverage"]` and
    `status = "missing"`; also assert `state = "not_ready"` and empty stderr. Do not install or fake
    tools, create `node_modules`/Python package state, access a registry, or treat foundation
    `node`/`python` readiness as evidence for shared-entry eligibility.
- Focused checks:
  - `cargo test -p ayni-cli catalog_entry_is_eligible_for_any_enabled_signal --all-features`
  - `cargo test -p ayni-cli --test install_check_e2e shared_requirements_follow_any_enabled_signal --all-features`

#### Task 3.2: Reject lexical and canonical configured-root escape

- Tier: L
- Tier rationale: Repository-root containment is a security boundary used by policy, CLI, install,
  discovery, and symlink-aware filesystem behavior; it is cross-layer even though the helper is
  small.
- Depends on: 2.4
- Affected responsibilities: Core lexical policy validation; common canonical path containment; CLI
  pre-adapter validation for analyze/verify/install/check; deterministic error behavior.
- Expected paths: `core/src/policy.rs`, `core/src/policy/roots.rs` (new),
  `adapters/common/src/paths.rs`, `cli/src/discovery.rs`, `cli/src/install.rs`,
  `cli/src/install_check.rs`, `cli/src/verify.rs`, `cli/src/tests.rs`,
  `cli/tests/install_check_e2e.rs`, `cli/tests/verify_e2e.rs`.
- Focused checks:
  - `cargo test -p ayni-core policy::tests::rejects_parent_components --all-features`
  - `cargo test -p ayni-adapters-common paths::tests::canonical_containment --all-features`
  - `cargo test -p ayni-cli configured_root_escape --all-features`

### Milestone 4: Make completion and generated verification commands exact

- Status: pending
- Depends on: Milestone 3
- Acceptance:
  - [ ] Expected row keys, not target counters alone, determine completion.
  - [ ] Complete/pass zero-row and missing-row artifacts are rejected.
  - [ ] Emitted commands preserve config and exact root, and execute against the intended multi-root
    target.
- Validation:
  - `cargo fmt --all -- --check`
  - `cargo test -p ayni-core --all-features`
  - `cargo test -p ayni-cli --test completion_e2e --test findings_e2e --test verify_e2e --all-features`

#### Task 4.1: Reconcile completion against expected row keys

- Tier: L
- Tier rationale: Completion and aggregate status are public artifact integrity contracts spanning
  concurrent collection, serialization/deserialization, and comparison compatibility.
- Depends on: 3.2
- Affected responsibilities: Expected-row planning; deterministic collection issues; completed-
  target accounting; aggregate and structural artifact validation; schema-v3 documentation tests.
- Expected paths: `cli/src/main.rs`, `cli/src/verify.rs`, `cli/src/completion.rs` (new),
  `core/src/signal.rs`, `core/src/comparison.rs`, `cli/tests/completion_e2e.rs`.
- Focused checks:
  - `cargo test -p ayni-core signal::tests::complete_artifact_requires_rows --all-features`
  - `cargo test -p ayni-core signal::tests::complete_artifact_rejects_inconsistent_row_sets --all-features`
  - `cargo test -p ayni-cli --test completion_e2e missing_expected_signal_row --all-features`

#### Task 4.2: Add exact root/config selectors to finding commands

- Tier: L
- Tier rationale: This adds public CLI syntax and changes every serialized finding command while
  preserving adapter selector contracts and handling multi-root ambiguity.
- Depends on: 3.2, 4.1
- Affected responsibilities: Verify argument parsing and target selection; shell-safe command
  rendering; finding materialization; multi-root copy/paste behavior.
- Expected paths: `cli/src/main.rs`, `cli/src/verify.rs`,
  `cli/src/verification_command.rs` (new), `cli/src/tests.rs`, `cli/tests/findings_e2e.rs`,
  `cli/tests/verify_e2e.rs`.
- Focused checks:
  - `cargo test -p ayni-cli verification_command::tests --all-features`
  - `cargo test -p ayni-cli --test findings_e2e preserves_non_default_config_and_root --all-features`
  - `cargo test -p ayni-cli --test verify_e2e emitted_multi_root_command_is_reproducible --all-features`

### Milestone 5: Unify execution and restore adapter-owned package-manager architecture

- Status: pending
- Depends on: Milestone 4
- Acceptance:
  - [ ] A callback observes a child line before child exit, and timeout kills/waits deterministically.
  - [ ] No production adapter/catalog subprocess bypasses common execution.
  - [ ] Configured timeout reaches collector secondary commands and install/status/preparation paths.
  - [ ] Every collector runner spawn/wait/timeout failure is an emitted failed row before the adapter
    boundary, with exact structured classification; no collector uses a string compatibility runner.
  - [ ] Core/common expose the neutral catalog runtime only; all Node/Python manager types,
    detection/resolution, requirement metadata, and command construction live in the owning adapter.
  - [ ] Node/Python manager and workspace characterization matrices pass, and list/apply/foundation/
    check E2E tests prove unchanged operation ordering, cwd selection, read-only behavior, and errors.
  - [ ] CLI has one language-neutral catalog path, and no migration shim or old manager-bearing core
    export remains at promotion.
- Validation:
  - `cargo fmt --all -- --check`
  - `cargo test -p ayni-adapters-common -p ayni-adapters-rust -p ayni-adapters-go -p ayni-adapters-node -p ayni-adapters-python -p ayni-adapters-kotlin -p ayni-cli --all-features`
  - `cargo check --workspace --all-features`
  - `python3 -c 'import re; from pathlib import Path; banned=(r"\brun_command_for_context\(",r"\brun_command_for_context_streaming\(",r"\bCommand::new\(",r"\bstd::process::Command\b"); hits=[f"{p}:{pattern}" for root in Path("adapters").glob("*/src/collectors") for p in root.rglob("*.rs") for pattern in banned if re.search(pattern,p.read_text())]; assert not hits,hits'`
  - `python3 -c 'from pathlib import Path; banned=("NodePackageManager","PythonPackageManager","detect_node_package_manager","detect_python_package_manager","resolve_python_package_manager","node_package_manager","python_package_manager","NpmGlobal","NodePackage","PythonPackage","PythonRuntime","UvTool","pnpm-lock.yaml","yarn.lock","package-lock.json","bun.lock","packageManager","uv.lock","poetry.lock","pdm.lock","Pipfile.lock","hatch.toml"); hits=[f"{p}:{term}" for root in (Path("core"),Path("adapters/common")) for p in root.rglob("*.rs") for term in banned if term in p.read_text()]; assert not hits, hits; assert not Path("core/src/catalog/python_resolution.rs").exists()'`

#### Task 5.1: Make the common runner concurrently drain, stream, capture, and time out

- Tier: L
- Tier rationale: Correctly coordinating child lifecycle, two pipes, partial lines, callback timing,
  and kill/wait behavior is bounded but concurrency-sensitive.
- Depends on: 4.2
- Affected responsibilities: Shared process lifecycle and structured execution errors; live progress;
  stdout/stderr capture ordering guarantees.
- Expected paths: `adapters/common/src/exec.rs`, `adapters/common/src/lib.rs`,
  `adapters/common/src/failure.rs`.
- Focused checks:
  - `cargo test -p ayni-adapters-common exec::tests::callback_runs_before_child_exit --all-features`
  - `cargo test -p ayni-adapters-common exec::tests::timeout_kills_and_classifies_child --all-features`
  - `cargo test -p ayni-adapters-common exec::tests::captures_partial_stdout_and_stderr --all-features`

#### Task 5.2: Define the structured collector-failure boundary

- Tier: M
- Tier rationale: The runner error and schema-v3 row shapes already exist; this adds one bounded
  adapter-common conversion seam without changing the public core collector trait or CLI contract.
- Depends on: 5.1
- Affected responsibilities: Internal distinction between runner-owned and ordinary adapter errors;
  all-signal failed-row synthesis; exact command/cwd/timeout classification before an adapter abort.
- Expected paths: `adapters/common/src/collector.rs` (new), `adapters/common/src/failure.rs`,
  `adapters/common/src/lib.rs`.
- Interface/failure contract:
  - `CollectorError::Execution(Box<ExecutionError>)` is the only variant converted to an `Ok`
    failed row; `CollectorError::Adapter(String)` remains an `AdapterError`.
  - `CollectorResult` is internal adapter plumbing. `finish_collection` accepts language, requested
    kind, context, and that result; it preserves scope and emits the matching typed result variant.
  - Spawn/wait map to `command_error`, timeout maps to `timeout`; category continues to use the
    existing signal-category mapping. A child non-zero exit is not a `CollectorError` and remains in
    the owning collector's existing classifier.
- Focused checks:
  - `cargo test -p ayni-adapters-common collector::tests::execution_errors_become_typed_failed_rows --all-features`
  - `cargo test -p ayni-adapters-common failure::tests::execution_error_classification --all-features`

#### Task 5.2a: Migrate every Rust collector command to structured failures

- Tier: M
- Tier rationale: Five known collector command paths and one dispatch module follow the exact Task
  5.2 seam; metadata nesting makes this multi-file work, but no new contract is introduced.
- Depends on: 5.2
- Affected responsibilities: Normal/focused/streaming Cargo test; llvm-cov; cargo metadata for
  complexity/deps; rust-code-analysis; mutation; Rust collector dispatch.
- Expected paths: `adapters/rust/src/collectors/mod.rs`,
  `adapters/rust/src/collectors/test.rs`, `adapters/rust/src/collectors/coverage.rs`,
  `adapters/rust/src/collectors/complexity.rs`, `adapters/rust/src/collectors/deps.rs`,
  `adapters/rust/src/collectors/mutation.rs`.
- Failure/compatibility contract: Propagate runner errors from both cargo-metadata levels and the
  selected-test streaming path through `CollectorError`; retain all current policy/selector/parser
  errors and non-zero tool handling, including the current Rust test and coverage failure categories.
- Focused checks:
  - `cargo test -p ayni-adapters-rust collectors::tests::configured_timeout_is_failed_row --all-features`
  - `cargo test -p ayni-adapters-rust collectors --all-features`

#### Task 5.2b: Migrate every Go collector command to structured failures

- Tier: M
- Tier rationale: The primary utility has four consumers and the already-structured deps/secondary-
  coverage paths are consolidated onto the demonstrated common boundary; scope is one adapter.
- Depends on: 5.2a
- Affected responsibilities: Go test and focused test, primary and post-processing coverage commands,
  gocyclo, go-list deps, mutation, and Go collector dispatch.
- Expected paths: `adapters/go/src/collectors/mod.rs`, `adapters/go/src/collectors/util.rs`,
  `adapters/go/src/collectors/test.rs`, `adapters/go/src/collectors/coverage.rs`,
  `adapters/go/src/collectors/complexity.rs`, `adapters/go/src/collectors/deps.rs`,
  `adapters/go/src/collectors/mutation.rs`.
- Failure/compatibility contract: Both coverage subprocesses and `go list` use the common execution-
  error row path; profile cleanup remains best-effort on every exit. Existing non-zero test,
  coverage, deps, complexity, and mutation behavior is not reclassified.
- Focused checks:
  - `cargo test -p ayni-adapters-go collectors::tests::configured_timeout_is_failed_row --all-features`
  - `cargo test -p ayni-adapters-go collectors --all-features`

#### Task 5.2c: Migrate every Node collector command to structured failures

- Tier: M
- Tier rationale: One manager-aware utility plus override/streaming call sites cover four bounded
  process-bearing collectors. The manager ownership migration remains deferred to Task 5.4.
- Depends on: 5.2b
- Affected responsibilities: Node normal/focused test, coverage, ESLint complexity, mutation, package-
  manager execution utility, and Node collector dispatch.
- Expected paths: `adapters/node/src/collectors/mod.rs`, `adapters/node/src/collectors/util.rs`,
  `adapters/node/src/collectors/test.rs`, `adapters/node/src/collectors/coverage.rs`,
  `adapters/node/src/collectors/complexity.rs`, `adapters/node/src/collectors/mutation.rs`.
- Failure/compatibility contract: Preserve manager-built argv and all non-zero `import_error`,
  `no_tests`, report, coverage, complexity, and mutation classifications. `deps.rs` remains unchanged
  because it is source-native and creates no child process.
- Focused checks:
  - `cargo test -p ayni-adapters-node collectors::tests::configured_timeout_is_failed_row --all-features`
  - `cargo test -p ayni-adapters-node collectors --all-features`

#### Task 5.2d: Migrate every Python collector command to structured failures

- Tier: M
- Tier rationale: One manager-aware utility feeds four process-bearing collectors, with a bounded
  second mutation invocation. The six-manager ownership cutover remains deferred to Task 5.5.
- Depends on: 5.2c
- Affected responsibilities: Python normal/focused test, coverage, complexipy, both mutmut commands,
  manager execution utility, and Python collector dispatch.
- Expected paths: `adapters/python/src/collectors/mod.rs`, `adapters/python/src/collectors/util.rs`,
  `adapters/python/src/collectors/test.rs`, `adapters/python/src/collectors/coverage.rs`,
  `adapters/python/src/collectors/complexity.rs`, `adapters/python/src/collectors/mutation.rs`.
- Failure/compatibility contract: Preserve manager-built argv, report preparation, and existing
  non-zero `import_error`, `collection_error`, `no_tests`, coverage, complexity, and mutation
  classifications. `deps.rs` remains unchanged because it is source-native.
- Focused checks:
  - `cargo test -p ayni-adapters-python collectors::tests::configured_timeout_is_failed_row --all-features`
  - `cargo test -p ayni-adapters-python collectors --all-features`

#### Task 5.2e: Migrate every Kotlin collector command and close the source inventory

- Tier: M
- Tier rationale: Five primary Gradle commands and the already-structured task probe follow the
  common seam; the final static inventory is exact and bounded to collector sources.
- Depends on: 5.2d
- Affected responsibilities: Kotlin normal/focused test, primary coverage, coverage task probing,
  detekt complexity, dependencies, mutation, Gradle utility, and Kotlin collector dispatch.
- Expected paths: `adapters/kotlin/src/collectors/mod.rs`,
  `adapters/kotlin/src/collectors/util.rs`, `adapters/kotlin/src/collectors/test.rs`,
  `adapters/kotlin/src/collectors/coverage.rs`, `adapters/kotlin/src/collectors/complexity.rs`,
  `adapters/kotlin/src/collectors/deps.rs`, `adapters/kotlin/src/collectors/mutation.rs`.
- Failure/compatibility contract: A task-probe spawn/wait/timeout and a primary Gradle spawn/wait/
  timeout use the same common row path. A successful task probe with no preferred task and every
  primary non-zero/report classification remain unchanged.
- Focused checks:
  - `cargo test -p ayni-adapters-kotlin collectors::tests::configured_timeout_is_failed_row --all-features`
  - `cargo test -p ayni-adapters-kotlin collectors --all-features`
  - `python3 -c 'import re; from pathlib import Path; banned=(r"\brun_command_for_context\(",r"\brun_command_for_context_streaming\(",r"\bCommand::new\(",r"\bstd::process::Command\b"); hits=[f"{p}:{pattern}" for root in Path("adapters").glob("*/src/collectors") for p in root.rglob("*.rs") for pattern in banned if re.search(pattern,p.read_text())]; assert not hits,hits'`

#### Task 5.3: Define the neutral catalog runtime and generic execution backend

- Tier: L
- Tier rationale: This replaces a public cross-layer core trait/context and catalog installer
  contract, adds structured operation failures, and changes how every adapter reaches shared
  process execution; it is the architectural foundation of the requested L migration.
- Depends on: 5.2e
- Affected responsibilities: Core catalog identity/status/runtime contracts; opaque adapter-managed
  entries; generic timeout-aware status/install/preparation errors; delegation for adapters without
  private package managers.
- Expected paths: `core/src/adapter.rs`, `core/src/catalog.rs`, `core/src/lib.rs`,
  `adapters/common/src/catalog.rs`, `adapters/common/src/exec.rs`,
  `adapters/common/src/failure.rs`, `adapters/common/src/lib.rs`,
  `adapters/{rust,go,kotlin}/src/adapter.rs`.
- Focused checks:
  - `cargo test -p ayni-core catalog::tests::adapter_managed_entries_are_opaque --all-features`
  - `cargo test -p ayni-adapters-common catalog::tests::status_probe_times_out --all-features`
  - `cargo test -p ayni-adapters-common catalog::tests::installer_streams_and_times_out --all-features`

#### Task 5.4: Move Node manager resolution and catalog commands into the Node adapter

- Tier: L
- Tier rationale: Although behavior is characterized, this moves runtime resolution and command
  construction across a public crate boundary and must preserve four package managers, workspace
  ancestry, collector execution, install state, and failure behavior as one coherent contract.
- Depends on: 5.3
- Affected responsibilities: Private Node manager enum; marker/manifest detection; direct/ancestor/
  fallback resolution; ambiguity and cwd mapping; tool/add/prepare command construction; local
  dependency status and adapter-managed catalog operations.
- Expected paths: `adapters/node/src/package_manager.rs` (new), `adapters/node/src/lib.rs`,
  `adapters/node/src/adapter.rs`, `adapters/node/src/catalog.rs`,
  `adapters/node/src/collectors/util.rs`, `adapters/node/src/collectors/test.rs`,
  `adapters/node/src/collectors/mutation.rs` and any other Node collector importing manager helpers.
- Focused checks:
  - `cargo test -p ayni-adapters-node package_manager::tests --all-features`
  - `cargo test -p ayni-adapters-node adapter::tests::package_manager_resolution_matrix --all-features`
  - `cargo test -p ayni-adapters-node catalog::tests::manager_install_and_status_matrix --all-features`

#### Task 5.5: Move Python manager/workspace resolution and catalog commands into the Python adapter

- Tier: L
- Tier rationale: This relocates a public resolver and six-manager command API across layers while
  preserving uv ancestry/ambiguity, module normalization, runtime/import/uv-tool probes, catalog
  installation, collector execution, and failure behavior.
- Depends on: 5.3
- Affected responsibilities: Private Python manager and resolution types; direct/ancestor/fallback
  detection; uv workspace parsing; ambiguity and cwd mapping; run/module/add command construction;
  Python runtime, local import, and uv-tool catalog operations.
- Expected paths: `adapters/python/src/package_manager.rs` (new), `adapters/python/src/lib.rs`,
  `adapters/python/src/adapter.rs`, `adapters/python/src/catalog.rs`,
  `adapters/python/src/collectors/util.rs` and collectors using its command helpers.
- Focused checks:
  - `cargo test -p ayni-adapters-python package_manager::tests --all-features`
  - `cargo test -p ayni-adapters-python adapter::tests::package_manager_resolution_matrix --all-features`
  - `cargo test -p ayni-adapters-python catalog::tests::manager_install_and_status_matrix --all-features`

#### Task 5.6: Switch every install mode to adapter catalog runtimes and remove the leaked API

- Tier: L
- Tier rationale: This is the compatibility cutover for public install/list/check behavior across
  CLI, core, common, and both adapters; it removes the old API only after cross-mode workspace E2E
  proof and must preserve a versioned readiness contract.
- Depends on: 5.2e, 5.4, 5.5
- Affected responsibilities: Language-neutral CLI catalog orchestration; apply-only preparation;
  list/check read-only status; conditional install and foundation revalidation; progress/timeout
  diagnostics; deletion of manager exports, language-specific installer variants, resolver module,
  and transitional shims.
- Expected paths: `cli/src/install.rs`, `cli/src/install_check.rs`, `cli/src/tests.rs`,
  `cli/tests/install_check_e2e.rs`, `cli/tests/install_managers_e2e.rs` (new),
  `core/src/catalog.rs`, `core/src/catalog/python_resolution.rs` (delete), `core/src/lib.rs`,
  `adapters/common/src/catalog.rs`, `adapters/node/src/{adapter,catalog}.rs`,
  `adapters/python/src/{adapter,catalog}.rs`.
- Focused checks:
  - `cargo test -p ayni-cli --test install_managers_e2e node_list_apply_check_workspace_matrix --all-features`
  - `cargo test -p ayni-cli --test install_managers_e2e python_list_apply_check_workspace_matrix --all-features`
  - `cargo test -p ayni-cli --test install_check_e2e timeout_is_not_ready --all-features`
  - `python3 -c 'from pathlib import Path; banned=("NodePackageManager","PythonPackageManager","detect_node_package_manager","detect_python_package_manager","resolve_python_package_manager","node_package_manager","python_package_manager","NpmGlobal","NodePackage","PythonPackage","PythonRuntime","UvTool"); hits=[f"{p}:{term}" for root in (Path("core"),Path("adapters/common")) for p in root.rglob("*.rs") for term in banned if term in p.read_text()]; assert not hits, hits; assert not Path("core/src/catalog/python_resolution.rs").exists()'`

### Milestone 6: Restore lock integrity and add non-Rust real-tool CI

- Status: pending
- Depends on: Milestone 5
- Acceptance:
  - [ ] Workspace lock entries match 0.8.0 and all dependency-resolving CI/release builds are locked.
  - [ ] Go, Node, Python, and Kotlin real-tool matrix jobs enforce complete/no-command-failure
    artifacts with no exclusions.
- Validation:
  - `cargo check --locked --workspace --all-features`
  - `python3 -m unittest discover -s scripts/ci -p 'test_*.py'`
  - `cargo test -p ayni-cli --all-features`

#### Task 6.1: Refresh only workspace lock metadata and require locked builds

- Tier: S
- Tier rationale: Cargo's existing lockfile behavior and the exact eight stale local entries are
  demonstrated; changes are version metadata and command flags, not dependency selection.
- Depends on: 5.6
- Affected responsibilities: Workspace package/release integrity; deterministic CI/docs/release and
  example-image builds.
- Expected paths: `Cargo.lock`, `.github/workflows/ayni.yml`, `.github/workflows/docs.yml`,
  `.github/workflows/release.yml`, `examples/{rust,go,node,python,kotlin}/Dockerfile`.
- Focused checks:
  - `cargo check --locked --workspace --all-features`
  - `python3 -c 'import pathlib,re; p=pathlib.Path("Cargo.lock").read_text(); names={"ayni-core","ayni-cli","ayni-adapters-common","ayni-adapters-rust","ayni-adapters-go","ayni-adapters-node","ayni-adapters-python","ayni-adapters-kotlin"}; blocks=p.split("[[package]]"); found={m.group(1):m.group(2) for b in blocks if (m:=re.search(r"name = \"([^\"]+)\"\nversion = \"([^\"]+)\"", b)) and m.group(1) in names}; assert found == {name:"0.8.0" for name in names}, found'`

#### Task 6.2: Add a typed artifact validator and four-language real-tool matrix

- Tier: L
- Tier rationale: The gate coordinates four package/build ecosystems, network-backed tool setup,
  intentional non-zero policy outcomes, and a shared typed artifact assertion.
- Depends on: 2.4, 3.1, 4.1, 5.6, 6.1
- Affected responsibilities: PR integration coverage for adapter catalogs/collectors; typed CI
  distinction between policy findings and incomplete/tool-failed collection.
- Expected paths: `.github/workflows/ayni.yml` or `.github/workflows/examples.yml` (new),
  `scripts/ci/validate_example_artifact.py` (new),
  `scripts/ci/test_validate_example_artifact.py` (new), and only if a real tool proves a fixture
  contract stale, the corresponding `examples/{go,node,python,kotlin}/mono/` manifest/policy.
- Focused checks:
  - `python3 -m unittest discover -s scripts/ci -p 'test_*.py'`
  - `python3 scripts/ci/validate_example_artifact.py --help`

### Milestone 7: Align user documentation and run the final repository contract

- Status: pending
- Depends on: Milestone 6
- Acceptance:
  - [ ] Generated CLI docs include exact root selection; product/adapter/runtime docs describe the
    implemented failure behavior, Go limitation, and adapter-owned Node/Python catalog runtime.
  - [ ] README install/output examples and VitePress navigation match current behavior and v3.
  - [ ] No root policy threshold is changed; architecture/runtime/contributor docs agree that core
    and common are manager-neutral and each language adapter owns detection, resolution, and command
    construction.
  - [ ] All classic repository checks and the locked check pass before the orchestrator's one final
    canonical Ayni gate.
- Validation:
  - `cargo doc-cli | diff -u docs/cli.md -`
  - `npm run docs:build`
  - `cargo fmt --all -- --check`
  - `cargo clippy --workspace --all-targets --all-features -- -D warnings`
  - `cargo test --workspace --all-features`
  - `cargo check --workspace --all-features`
  - `cargo check --locked --workspace --all-features`

#### Task 7.1: Document corrected product, adapter, and CLI contracts

- Tier: M
- Tier rationale: The content spans generated CLI, current schema, architecture, config/runtime, five
  adapters, and contribution guidance, but every statement is determined by completed behavior.
- Depends on: 2.4, 3.2, 4.2, 5.6
- Affected responsibilities: Public configuration/signal/runtime contracts; adapter capability and
  package-manager/catalog ownership and failure docs; generated CLI reference.
- Expected paths: `ARCHITECTURE.md`, `docs/cli.md`, `docs/product/config.md`, `docs/product/runtime.md`,
  `docs/product/signals/v3.md`, `docs/contributing/adapters.md`,
  `docs/adapters/{rust,go,node,python,kotlin}.md`.
- Focused checks:
  - `cargo doc-cli | diff -u docs/cli.md -`
  - `cargo test -p ayni-cli agents_managed_guidance --all-features`

#### Task 7.2: Correct onboarding, example output, and v3 navigation drift

- Tier: S
- Tier rationale: Verified stale text/links have exact replacements: disclose Rust bare-install
  default, refresh current paths/output, and add current-v3 navigation ahead of historical versions.
- Depends on: 1.2, 4.1, 7.1
- Affected responsibilities: README onboarding and illustrative report; VitePress information
  architecture; example-fixture guidance where needed.
- Expected paths: `README.md`, `docs/.vitepress/config.ts`, `docs/index.md`,
  `examples/README.md` if command examples require clarification.
- Focused checks:
  - `npm run docs:build`
  - `cargo test -p ayni-cli agents_managed_guidance_describes_discovery_policy_and_quality_workflow --all-features`
