# Tooling reconciliation — foundations and baselines

The intended lifecycle is: reconciliation establishes native declarations and
locks, `env lock` snapshots exact requirements, and `env build` installs them.
Milestones 1 and 2 implement the core boundary and adapter-owned baseline inventories.
They do not implement `tools reconcile`, manifest editing, package-manager
execution, or changes to `init`.

## Ownership and version authority

Omitted ownership means `project`, preserving existing policy behavior. Core
parses `[environment.signal_tools] ownership = "project" | "ayni"`. Ayni-owned
mode is not operational yet: environment planning and locking reject it rather
than silently consuming project-owned declarations. Host quality commands keep
their existing execution semantics and do not reconcile tooling.

`ToolVersionAuthority` identifies who chooses the version, independently of a
`RequirementSource` describing the supporting evidence:

| Authority | Meaning | Current uses |
| --- | --- | --- |
| `adapter_pinned` | An exact adapter baseline selects the version. | cargo-llvm-cov, rust-code-analysis-cli, and gocyclo. |
| `project_locked` | Native project declarations and locks select the version. | Node dependencies, uv dependencies, Gradle plugins. |
| `lock_resolved` | Explicit locking resolves a provider requirement. | Available for explicitly unpinned provider requirements; no current isolated signal-tool baseline uses it. |
| `toolchain` | The runtime/toolchain chooses a component's version. | Reserved for signal tools represented with runtime scope; existing Rust components remain in runtime requirements. |

Adapter-pinned requirements must be exact. Project-locked tools use project
scope, toolchain tools use runtime scope, and project tools cannot claim
lock-resolved authority. Resolution retains the authority; it does not infer it
from a changed source-kind string. Every serialized signal tool now requires
`version_authority`. Environment plan schema is `0.4.0`; lock schema is `0.6.0`.
Older documents require regeneration. Signal artifacts and OCI image labels
retain their existing versions.

## Read-only proposals

`ToolingRequest` carries an absolute repository root, normalized target,
ownership, enabled signals, and the subset using default tools. A caller must
validate filesystem containment before constructing it. Core performs lexical
validation only.

`ToolingPlan` contains tool requirements and current declaration/resolution
text, an owning root, digest-tracked staging inputs, declaration edits,
structured commands, exact output files, warnings, and conflicts. Existing
outputs require matching input digests. Missing outputs use an explicit
`Absent` preimage. An edit must match its output's preimage. Duplicate edits,
conflicting input/output expectations, directory/file overlaps, escaping paths,
and repository-control paths are rejected.

Commands use program/argv/cwd/environment fields and declare exactly which
allowlisted outputs they produce. Core reuses preparation's argument and
environment validation, with additional executable-name and path checks.
Command order is preserved. No shell is invoked, and validating a program name
does not authorize resolving it from ambient PATH.

Project-owned plans cannot propose mutations. Requirements can only cover the
request's default-tool signals, so custom command overrides cannot be rewritten.
A proposal may describe edits alongside conflicts for a future preview; an apply
engine must refuse execution while conflicts exist.

`LanguageAdapter::plan_tooling` checks request/capability language and validates
the returned target and entire plan. Public proposal fields permit adapter
assembly, but the plan has no unchecked deserializer. Do not call the raw
capability in an executor or treat JSON output as permission to execute.

## Required before apply support

A future executor must resolve exact adapter-approved runtimes and package
managers, verify every preimage, stage only approved inputs, and enforce canonical
containment and symlink protections at the filesystem boundary. It must validate
outputs and rerun reconciliation before publishing them. Metadata staging alone
is not a sandbox for native build configuration or package-manager execution.

Replacing individual files atomically is not an atomic multi-file transaction.
The apply design still needs a recovery journal, rollback policy, and concurrent
writer exclusion for publication failures or process crashes. Expected digests
are necessary but do not close races between a final check and rename.

The next milestone adds read-only adapter reconciliation planning and CLI preview/check.
Keep new ownership out of generated init policies until the adapter planners
and staged apply engine are available and verified.

## Adapter-owned baselines (milestone 2)

Each adapter's `tooling.rs` owns a `ManagedToolSpec` inventory exposed by
`LanguageAdapter::managed_tool_specs`. It references catalog names; the catalog
remains the single source for signal mappings. `validate_managed_tools` requires
exactly one inventory entry per catalog tool, including runtimes. External tools
require exact baselines; runtimes and components follow the selected toolchain.

`select_managed_tools` accepts only the caller's default-tool signals, excludes
runtime entries, and returns only matching signal associations. A reconciliation
caller must pass `ToolingRequest::default_tool_signals()`, never all enabled
signals: custom commands suppress default-tool reconciliation. The Kotlin
adapter must narrow the two coverage alternatives after inspecting native
metadata. `coverage_baseline` preserves Kover or JaCoCo when explicitly selected
and prefers Kover when no provider is declared; conflicting declarations must be
reported before selection. Disabled tools are not removed.

| Adapter | Exact external baselines | Integration |
| --- | --- | --- |
| Rust | cargo-llvm-cov 0.8.5; rust-code-analysis-cli 0.0.25 | Isolated Cargo tools; llvm-tools-preview follows Rust. |
| Go | gocyclo 0.6.0 | Isolated Go module provider; never added to application go.mod. |
| Node | vitest 3.2.7; @vitest/coverage-v8 3.2.7; eslint 9.39.5; @typescript-eslint/parser 8.67.0 | Governing project devDependencies in a future reconciliation planner. |
| Python | pytest 9.0.3; pytest-json-report 1.5.0; pytest-cov 6.0.0; coverage 7.6.12; complexipy 7.0.1; mutmut 2.5.1 (opt-in) | Governing uv development group in a future reconciliation planner. |
| Kotlin | Kover 0.9.8; JaCoCo 0.8.12; Detekt 1.23.8; PIT Gradle plugin 1.19.0 (opt-in) | Exact plugin declarations; JaCoCo uses the bundled plugin's `toolVersion`. |

These are compatibility baselines, not a latest-version policy. Native Node,
Python, and Kotlin versions remain project-authoritative today; inventories do
not overwrite their declarations, change init, or enable Ayni ownership. Existing
supported project plugin aliases remain accepted. The Detekt 1.23.8 baseline
specifically uses `io.gitlab.arturbosch.detekt`, not the newer `dev.detekt` plugin.

Rust environment planning now pins rust-code-analysis-cli to 0.0.25 with
`adapter_pinned` authority. Existing environment locks using lock-resolved
complexity tooling must be regenerated. Exact tool versions are also checked for
lock staleness, so future baseline upgrades invalidate older locks even when
authority and source files are unchanged. Schema versions remain plan 0.4.0 and
lock 0.6.0 because no serialized shape changes in milestone 2.

Baseline tests check catalog completeness, selection, opt-in mutation behavior,
and native example declarations. Rust 0.0.25 is exercised by checkout complexity
verification and the final repository contract. New native fixtures execute
mutmut, JaCoCo, and PIT, with their actual XML outputs retained in adapter parser
tests. See [baseline fixture validation](tooling-baseline-fixtures.md) for commands,
versions, and the limits of this evidence. Full native managed-environment runs
across all five adapters remain a later milestone.
