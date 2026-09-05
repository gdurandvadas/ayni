# Tooling reconciliation — milestone 1

The intended lifecycle is: reconciliation establishes native declarations and
locks, `env lock` snapshots exact requirements, and `env build` installs them.
This milestone implements the core boundary for that lifecycle. It does not
implement `tools reconcile`, adapter baselines, manifest editing, package-manager
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
| `adapter_pinned` | An exact adapter baseline selects the version. | cargo-llvm-cov and gocyclo. |
| `project_locked` | Native project declarations and locks select the version. | Node dependencies, uv dependencies, Gradle plugins. |
| `lock_resolved` | Explicit locking resolves a provider requirement. | The currently unpinned rust-code-analysis-cli. |
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

The next milestone should establish tested adapter-owned baselines and complete
catalog coverage. Keep new ownership out of generated init policies until the
adapter planners and staged apply engine are available and verified.
