# Project 05: adapter and tool-catalog platform

## Goal

Make language adapters the single owners of ecosystem-specific discovery,
environment requirements, signal execution, and impact mapping.

The CLI orchestrates adapters through typed capabilities. Core defines shared
product contracts. Neither layer hard-codes language markers, lockfiles,
package managers, tools, commands, or parsers.

## Adapter capability groups

### Repository capabilities

- detect candidate roots;
- normalize configured roots;
- resolve workspace and package ownership;
- identify native manifests and dependency locks;
- interpret runtime version sources and precedence;
- resolve package-manager ownership.

### Environment capabilities

- produce runtime and component requirements;
- identify required system capabilities or packages;
- describe native dependency preparation;
- provide signal-tool requirements;
- explain whether a requirement can be provisioned immutably;
- translate a target environment into a command-launch context.

### Quality capabilities

- declare supported signals;
- declare selector support per signal;
- plan external commands;
- collect or parse tool output;
- normalize typed results, offenders, and command failures;
- construct typed verification targets.

### Impact capabilities

- map files to roots and packages;
- build internal dependency topology;
- map tests to packages, modules, or source targets where possible;
- decide signal-specific affectedness;
- report confidence and broaden-scope requirements.

## Tool catalogs

Catalog entries describe tooling required by adapter operations. Each entry
contains:

- stable tool identity;
- compatible or exact version requirement;
- signals or capabilities that require it;
- provider or installation mechanism;
- installation scope: runtime, isolated global tool, or project dependency;
- status-detection behavior;
- supported OS and architecture combinations;
- whether provisioning modifies the checkout;
- whether locked, offline provisioning is supported;
- provider integrity metadata when available.

Selection is enabled when **any** required enabled capability needs an entry,
unless the catalog explicitly models a composite requirement. Generic catalog
code must not accidentally require every associated signal to be enabled.

## Provisioning boundary

Catalog metadata is owned by the adapter. Shared infrastructure may execute a
validated installation plan, but it does not choose ecosystem tools or infer
package-manager behavior.

Project-integrated tools are repository dependencies. `env build` may verify
and materialize them from native locks, but it must not add them silently.
Explicit repository bootstrap belongs to `init` output or a future dedicated
workflow, not quality execution.

## Shared infrastructure

`adapters/common` may provide:

- timeout-aware, streaming process execution;
- process-tree cancellation;
- path normalization and containment;
- safe filesystem discovery primitives;
- report parsing helpers for genuinely shared formats;
- deterministic failure scaffolding;
- cache primitives;
- generic catalog status and installation execution.

Shared code must not contain Node lockfile precedence, Python manager
resolution, Gradle plugin policy, Cargo component selection, or similar
language semantics.

## Built-in adapters

The initial product includes Rust, Node, Python, Go, and Kotlin. Each adapter
must work for single roots and monorepos, including multiple versions of the
same runtime across roots where the ecosystem permits it.

Implementation should prove the adapter contract with these five languages
before designing an external binary or plugin protocol. Prematurely freezing a
public SDK would preserve internal mistakes as compatibility obligations.

## Conformance suite

Every adapter is tested against shared requirements:

- deterministic root discovery and containment;
- environment-source explanation and conflict handling;
- catalog selection for every enabled-signal combination;
- complete target/signal row production;
- missing evidence fails closed;
- inclusive maximum and correct minimum threshold boundaries;
- zero discovered tests fail as a quality finding;
- selector validation happens before tool execution;
- verification commands reproduce contract and root scope;
- impact uncertainty broadens rather than narrows;
- normalized paths and finding identities remain stable.

Fixtures should cover both valid tools and deliberately malformed output.

## Core interface shape

Exact Rust traits will be designed during implementation, but capabilities
should be separable. A language should not need to implement impact analysis in
order to support full quality checks, and the execution engine should be able to
report an unsupported optional capability explicitly.

Core capability types contain semantic data. They do not expose CLI argument
types, Dockerfile fragments, or `mise` command strings.

### Initial environment-capability boundary

The first environment adapter interface uses these decisions:

- Environment discovery is an optional, separable capability on a language
  adapter. Quality support does not imply environment support.
- Each call receives a canonical, containment-checked repository root, one
  normalized target identity, the enabled signal set, and deterministically
  ordered requested platforms. Existing target roots, including symlinked
  roots, must resolve within the canonical repository; missing targets are
  validated through their nearest existing ancestor.
- Each result is one validated target contribution plus typed warnings and
  conflicts. Repository-level aggregation remains outside the adapter.
- Enabled-signal matching uses **any** semantics: a tool associated with several
  signals is required when at least one associated signal is enabled. An empty
  signal association does not make a tool universally required; composite or
  non-signal requirements must be modeled explicitly.
- Unsupported capabilities fail explicitly. Request language, capability
  language, and returned target identity are checked before results enter a
  plan.
- The shared conformance harness runs discovery repeatedly, compares canonical
  serialized contributions, and snapshots the bounded fixture to detect
  mutation. It does not interpret ecosystem files or execute providers.
- The Go adapter emits a complete `go:` provider coordinate for isolated tools;
  the Python adapter maps enabled signals to exact uv-locked project
  dependencies; and the Kotlin adapter maps analysis capabilities to exact
  repository Gradle plugins. Generic provisioning understands provider and
  output modes, not language-specific manifests or task semantics.

## Non-goals

- Public third-party adapter ABI in the first release.
- Adapter-defined signal kinds.
- Untyped extension payloads as a substitute for core contracts.
- Language-specific branches in CLI orchestration.
- A universal parser that erases ecosystem semantics.

## Dependencies

- Core contract, environment-plan, execution-plan, result, and impact types.
- Shared process and filesystem infrastructure.

## Deliverables

- Capability-based adapter interfaces.
- Typed tool-catalog contract.
- Safe shared adapter infrastructure.
- Built-in Rust, Node, Python, Go, and Kotlin adapters.
- Golden fixtures and reusable conformance harness.
- Adapter-author documentation after the internal contract stabilizes.

## Definition of done

- Every built-in adapter passes the same applicable conformance suite.
- Adding a language does not require language-specific changes in core or CLI.
- Catalog requirements can be planned and provisioned without surprise
  checkout mutation.
- Adapter output always conforms to typed core results.
- Language-specific discovery and command decisions remain inside the adapter.
