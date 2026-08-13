# Project 03: environment discovery, planning, and locking

## Goal

Convert repository configuration and native ecosystem files into a
deterministic, explainable environment plan.

The plan is the semantic boundary between language discovery and provisioning.
OCI and `mise` backends consume the plan; they do not reinterpret repository
manifests.

## Commands

### `ayni env show`

Read-only explanation of the environment Ayni would require. It works before a
lock exists and reports:

- configured targets and detected roots;
- every runtime constraint and its source;
- the selected package manager and runner;
- enabled-signal tool requirements;
- conflicts, ambiguity, unsupported platforms, and missing pins;
- which requirements are exact and which still need resolution.

### `ayni env doctor`

Compare repository inputs, the committed lock, local image state, and runtime
availability. It classifies the environment as ready, missing, stale,
conflicting, or unsupported and prints the next repair command.

### `ayni env lock`

Resolve exact requirements for one or more target platforms and write
`.ayni.lock`. Resolution is an explicit mutation and may use the network. It
does not build the image or change repository dependencies.

## Inputs

- `.ayni.toml` and configured language roots;
- native version selectors and compatibility declarations;
- package-manager declarations and lockfiles;
- enabled signals and adapter capabilities;
- adapter-owned signal-tool catalogs;
- target OS, architecture, and libc;
- the selected Ayni environment base.

## Language-owned interpretation

Adapters must preserve ecosystem semantics rather than treating every version
string as equivalent.

### Rust

- `rust-toolchain.toml` or `rust-toolchain` selects a toolchain.
- Cargo `rust-version` is a minimum supported Rust version, not necessarily a
  toolchain selection.
- Components and targets are part of the runtime requirement.

### Node

- `.node-version`, `.nvmrc`, and similar files may select a version.
- `engines.node` normally expresses a compatibility range.
- the `packageManager` field can select a package-manager family and version.
- workspace ownership determines which declaration controls a root.

### Python

- `.python-version` may select a runtime.
- `requires-python` expresses compatibility.
- uv, Poetry, PDM, Pipenv, Hatch, or plain Python ownership must be resolved by
  the adapter using documented precedence.

### Go

- the `go` and `toolchain` directives have different meanings.
- workspace and module ownership must be retained.

### Kotlin

- the Gradle wrapper version is not the JDK version.
- JVM toolchains and JDK vendor requirements are separate concerns.
- repository-owned Gradle configuration remains authoritative.

When sources cannot be reconciled, Ayni reports the conflict. It must not choose
an arbitrary exact version merely to finish locking.

## Environment plan

The typed plan should contain:

- repository and contract identity;
- normalized target identities;
- requirement source and confidence;
- exact or unresolved runtime constraints;
- runtime components and targets;
- package-manager family, version, and ownership scope;
- signal-tool requirements and installation scope;
- native dependency-lock paths and digests;
- platform requirements;
- warnings and blocking conflicts.

The plan can be rendered for humans or machines. It is not itself necessarily a
committed artifact.

## `.ayni.lock`

The committed lock is a deterministic projection of a fully resolved plan. It
contains at least:

- schema version;
- Ayni and `mise` versions;
- selected base-image reference and immutable digest;
- target platform, architecture, and libc;
- every normalized target identity;
- exact runtime selection per target;
- runtime components and package-manager versions;
- signal-tool versions, providers, and scopes;
- hashes of native dependency locks;
- provider-native lock data or a digest of that data;
- provenance describing which repository input produced each requirement.

The lock must not contain:

- credentials or registry tokens;
- local absolute paths;
- host-specific cache paths;
- unresolved `latest` values;
- executable hooks or arbitrary repository scripts.

The exact serialized format will be versioned, but there is no compatibility
requirement with the old Ayni artifact schemas.

## Determinism and staleness

- Equal normalized inputs for the same requested platforms produce byte-stable
  lock output.
- Ordering is defined by the schema, not filesystem traversal order.
- A lock records every source file whose contents affect resolution.
- `env doctor` reports staleness when a recorded input digest changes.
- Quality commands never refresh the lock automatically.
- Provider lock capabilities are reported honestly; `mise` checksums and
  provenance are not available equally for every backend.

## Security boundary

Repository configuration may contain executable hooks, tasks, templates, or
plugin code. Ayni should derive an isolated, validated provisioning
configuration from the environment plan rather than blindly activating an
arbitrary repository `mise.toml` during analysis.

Backends should be allowlisted. Environment inspection must remain read-only;
locking and provisioning must clearly disclose network access and executable
provider behavior.

## Non-goals

- Reimplementing native dependency resolution.
- Building an OCI image.
- Running quality signals.
- General developer-shell configuration.
- Claiming checksum or provenance guarantees a provider cannot supply.

## Dependencies

- [Project 01](01-command-model.md) for command behavior.
- [Project 05](05-adapter-catalog-platform.md) for language-owned requirement
  discovery and tool metadata.

## Deliverables

- Core environment requirement, plan, conflict, and lock types.
- Deterministic lock serialization and fingerprinting.
- Adapter version-source discovery for all supported languages.
- `env show`, `env doctor`, and `env lock` application operations.
- Provider translation for `mise` without leaking it into core semantics.
- Cross-platform and conflicting-version fixtures.

## Definition of done

- All built-in adapters explain their environment sources and precedence.
- Conflicting or ambiguous version sources fail with actionable diagnostics.
- Re-locking unchanged inputs is byte-stable.
- `env doctor` detects missing, stale, incompatible, and unsupported state.
- `env build` can consume the lock without reading language manifests to make
  new semantic decisions.
