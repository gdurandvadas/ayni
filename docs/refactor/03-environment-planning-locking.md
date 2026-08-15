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

### Initial Rust and Node discovery decisions

The first built-in environment capabilities establish these rules:

- Discovery reads only containment-checked repository inputs. Existing manifests, selectors, toolchain files, and native locks that are unreadable, malformed, or symlink outside the repository fail closed.
- Rust uses the nearest ancestor containing `rust-toolchain.toml` or `rust-toolchain`. In one directory, TOML takes precedence and disagreement with the legacy file is a blocking conflict. Cargo `rust-version` remains a minimum when no toolchain selector applies; workspace-inherited values retain workspace provenance.
- Rust coverage adds `llvm-tools-preview` to runtime components rather than inventing an independently versioned tool. Adapter catalog tools remain separate signal-tool requirements.
- Node member declarations and direct native locks take precedence over workspace defaults, matching the adapter's execution resolver. An ancestor owns a target only when a validated workspace pattern includes it.
- `.node-version` and `.nvmrc` are peer selectors at the same ownership level; disagreement is blocking. Direct selectors override workspace selectors, direct `engines.node` overrides workspace compatibility evidence, and every conflicting value is retained in source detail.
- Node `packageManager` declarations are checked against native lockfile family. Missing locks, ambiguous lock families, and declaration/lock disagreement are blocking conflicts. The npm execution fallback remains an unresolved assumed requirement rather than disappearing from the plan.
- Project-integrated Node signal tools never claim immutable offline provisioning in this discovery slice. Missing declarations explicitly report checkout mutation; declared tools remain online-only until native lock materialization proves the stronger guarantee.

### Initial Go, Python, and Kotlin decisions

- Go workspace ownership requires validated `go.work use` membership. Toolchain
  selectors govern exact selection while `go` directives remain minima; a
  selected toolchain below the effective module/workspace minimum is blocking.
  Go's own toolchain download is disabled, and complexity uses the exact
  adapter-owned `go:` provider coordinate for `gocyclo`.
- Managed Python is initially uv-only. Ownership follows validated uv workspace
  members, `[tool.uv].required-version` is mandatory, and each enabled project
  tool must be both declared and resolved once in `uv.lock`. Other manager
  families remain host-capable but block portable managed locking.
- Managed Kotlin is initially Gradle/JVM-only. The exact official wrapper
  distribution, repository JDK evidence, POSIX wrapper files, build/settings
  metadata, and committed dependency locks are mandatory. Kover/JaCoCo,
  Detekt, and PIT remain exact project plugins; the clean-slate path never
  inserts them.
- These adapters use the same optional capability interfaces and generic lock
  projection as Rust and Node. Core and CLI do not interpret Go directives, uv
  workspaces, Python requirements, Gradle wrappers, JVM toolchains, or plugins.

## Environment plan

The typed plan should contain:

- repository and contract identity;
- normalized target identities;
- requirement source and confidence;
- exact or unresolved runtime constraints;
- runtime components and targets;
- package-manager family, version, and ownership scope;
- signal-tool requirements and installation scope;
- native dependency-lock and preparation-manifest paths and digests;
- platform requirements;
- warnings and blocking conflicts.

The plan can be rendered for humans or machines. It is not itself necessarily a
committed artifact.

### Core contract decisions

The initial core planning contract establishes these boundaries:

- Target identity is only the language plus normalized repository-relative
  root. Workspace and package ownership are target context, not identity.
- Version evidence preserves whether a value is exact, a selector, a
  compatibility range, a minimum, or unresolved. Core does not interpret the
  ecosystem-specific expression.
- Explainable plans may contain unresolved requirements, warnings, and blocking
  conflicts. A resolved plan requires exact runtime, package-manager, and signal
  tool versions, at least one target and platform, no conflicts, and validated
  provisioning support.
- Portable paths are lexical repository-relative paths. Core rejects absolute,
  drive-prefixed, and parent-component paths without consulting the host
  filesystem.
- Adapter-owned system capabilities, system packages, platform support,
  checkout-mutation behavior, and offline provisioning support remain typed
  plan data. Provisioning backends must not rediscover them.
- Project-integrated signal tools that require checkout mutation cannot become
  resolved provisioning input.

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

### Initial Rust and Node locking decisions

- `.ayni.lock` schema `0.3.0` is canonical JSON with exact requirements,
  deterministic ordering, and a SHA-256 fingerprint calculated without the
  fingerprint field itself. Repository identity uses the contract digest and
  excludes the checkout directory name so equivalent clones remain byte-stable.
  Requirement-source files carry content digests so byte changes remain visible
  even when they resolve to the same exact versions.
- Exact resolution remains adapter-owned. The CLI invokes capabilities and
  persists validated core contracts; it does not interpret Rust or Node
  selectors, manifests, native locks, or tool catalogs.
- Rust runtime selectors and Cargo-installed catalog tools resolve through
  bounded `mise latest` invocations owned by the Rust adapter. Node runtime and
  package-manager selectors use `mise ls-remote` candidates and npm-compatible
  semver evaluation owned by the Node adapter, so bounded ranges select the
  highest matching exact version rather than merely the provider's latest
  version. Exact repository declarations bypass provider resolution. Provider
  calls use the shared timeout runner with repository configuration, environment
  activation, and hooks disabled.
- Node project-integrated signal tools are read from `package-lock.json`; a
  missing locked tool fails rather than changing repository dependencies.
  Other Node lock formats remain unsupported for exact tool extraction in this
  first locking slice.
- The lock records Ayni and resolver `mise` versions plus the selected base
  variant, its pinned image `mise` version, and an immutable OCI manifest
  digest. `env lock` resolves the release base through Docker Buildx, or accepts
  an explicit `<reference>@sha256:<digest>` for compatible runtimes. Base
  identity participates in the canonical lock fingerprint.
- Lock replacement is atomic. Malformed existing locks and failed discovery or
  resolution leave the previous bytes untouched.

### Native dependency preparation decisions

- Preparation is an optional adapter capability that returns typed, deterministic
  structured argv, repository-relative cwd, explicit environment, and
  digest-tracked inputs. Core and adapters never describe shell fragments, OCI,
  Docker, or provider execution.
- Preparation commands are for an isolated staged copy of their recorded inputs,
  never the checkout. The future backend must create that stage before invoking
  them and must reject untracked or changed inputs.
- Rust preparation is `cargo fetch --locked` at the Cargo workspace owner and
  fails explicitly without `Cargo.lock`. Its tracked inputs include the owner
  manifest, the target manifest when different, and the lockfile.
- Node preparation currently supports npm with `package-lock.json` only and
  plans `npm ci --ignore-scripts --no-audit --no-fund`. Other Node managers and
  a missing npm lockfile fail explicitly. The owning `package.json` and
  `package-lock.json` are digest-tracked.
- Go preparation runs `go mod download all` with `GOTOOLCHAIN=local` and
  repository-external caches. Runtime module access is read-only and offline.
- uv preparation warms its package cache without installing repository source,
  then creates a fresh, root-specific virtual environment after the checkout is
  mounted. Fresh outputs are distinct from relocatable seeded outputs.
- Gradle preparation stages wrapper/build/lock metadata plus an Ayni-generated
  init script that resolves every resolvable configuration into
  `GRADLE_USER_HOME`. Managed commands use the locked JDK and Gradle wrapper
  offline without executing the historical plugin insertion path.

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
