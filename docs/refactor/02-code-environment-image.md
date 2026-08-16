# Project 02: code-environment base and repository image

## Goal

Provide a repeatable development and analysis environment for coding agents and
humans.

This is a **code environment**, not an application release image. Its job is to
make a repository buildable, testable, and measurable with the correct
toolchains and Ayni signal tools.

## Image model

### Published base

```text
ghcr.io/gdurandvadas/ayni-env:<ayni-version>-debian
```

The base image contains:

- the matching Ayni binary;
- a pinned `mise` binary;
- certificates, Git, an SSH client, and essential build utilities;
- a non-root workspace user;
- well-defined runtime, package, and build-cache directories;
- metadata identifying the Ayni and base-image versions.

The base image does **not** contain default Rust, Go, Node, Python, or Java
versions. It is universally capable, not universally preloaded.

### Repository environment

`ayni env build` derives an image from the committed environment lock. A local
tag is clone-independent and platform-specific:

```text
ayni-env:lock-<fingerprint-prefix>-linux-<architecture>
```

The repository image contains:

- every exact runtime version needed across configured roots;
- runtime components, such as Rust LLVM tools;
- required package-manager versions;
- Ayni signal tools;
- warmed native dependency stores;
- the validated environment lock and minimal environment metadata.

Repository source is not baked into the image. The checkout is mounted
read-write at `/workspace` when the environment runs.

## Why source is mounted

Keeping source outside the image:

- allows one environment image to serve many source revisions;
- avoids rebuilding after ordinary code changes;
- supports interactive agents and humans editing the same checkout;
- prevents repository content and local secrets from becoming image layers;
- makes the cache boundary explicit: environment inputs rebuild the image,
  source inputs do not.

## Per-root toolchains

One global `PATH` is insufficient for a polyglot monorepo. Every planned target
must carry an explicit environment selection, for example:

```text
apps/legacy  -> Node 20 + pnpm 9
apps/web     -> Node 22 + pnpm 10
services/api -> Python 3.12 + uv
crates/core  -> Rust 1.89 + llvm-tools
android      -> Temurin 21 + Gradle wrapper
```

The image may contain multiple versions of the same runtime. The Ayni process,
not the shell's current directory, selects the environment for each adapter
command. The target command is launched through an explicit environment such as
`mise exec`, which must set the full environment including variables such as
`JAVA_HOME`.

The container entrypoint is Ayni. A repository-root `mise exec -- ayni` would
select only one environment and is therefore not the execution model.

## Dependency preparation

There are three distinct responsibilities:

1. `mise` provisions runtime and package-manager binaries.
2. Ayni catalogs define required signal tools and compatible versions.
3. Native package managers own repository dependencies and their lockfiles.

The image builder may warm native stores:

- Cargo registry, Git, and build caches;
- Go module and build caches;
- npm, pnpm, or Yarn package stores;
- Python package caches and root-specific environments;
- Gradle dependency and wrapper caches.

Because the checkout is mounted later, some ecosystems require a fast offline
materialization step after mount. That step must use only locked inputs and
preloaded stores. A missing artifact is an environment failure, not permission
to download during `check`.

## Network and mutation rules

Provisioning is the networked phase. Normal managed-environment execution
should be able to run offline.

- Disable `mise` automatic installation in the finished image.
- Do not mutate dependency manifests or locks during `env build`.
- Do not modify the checkout during `check` merely to prepare tooling.
- Use BuildKit secret or SSH mounts for private registries.
- Never store credentials in locks, build arguments, or image layers.

If an Ayni signal tool must be project-integrated and is absent from the native
dependency lock, the build fails and explains the repository change required.

## Platform promise

Initial support:

- Debian/glibc;
- Linux AMD64 and ARM64;
- Docker or another compatible OCI runtime.

Deferred:

- Alpine/musl;
- Windows containers;
- macOS-specific and iOS toolchains;
- Android SDK management beyond the JDK and repository-owned Gradle setup;
- backing services such as databases or message brokers.

Repositories can compose the Ayni environment with Docker Compose or Dev
Containers for services. Ayni does not become a general service orchestrator.

## Rebuild identity

The repository environment is stale when any material environment input
changes, including:

- `.ayni.lock`;
- the selected base-image digest;
- a native dependency lock tracked by `.ayni.lock`;
- platform or architecture;
- an Ayni or catalog version that participates in provisioning.

Ordinary source edits do not invalidate the image.

## Initial environment-image decisions

The first lock-driven OCI backend establishes these rules:

- The universal base is published as a Debian AMD64/ARM64 manifest at
  `ghcr.io/gdurandvadas/ayni-env:<ayni-version>-debian`. It contains the
  matching Ayni binary built against Debian Bookworm, checksum-verified `mise`
  2025.2.4, essential build utilities, and a non-root `ayni` user, but no
  language runtime.
- `.ayni.lock` schema `0.3.0` records the base reference, immutable manifest
  digest, variant, and image `mise` version. Locking resolves the default base
  through Docker Buildx or accepts an explicit digest-qualified base.
- Repository images use clone-independent, platform-specific tags derived from
  the full lock fingerprint. The fingerprint, base digest, Ayni/mise versions,
  platform, and environment-image schema are repeated as OCI labels and
  validated by `env doctor` and before launch.
- Generated build context contains a Dockerfile, deterministic `mise.toml`, and
  only adapter-declared inputs whose digests are present in the lock. Repository
  source and credentials are never copied. Project-scoped Node tools remain
  native dependencies rather than being translated into invented `mise`
  providers.
- `env shell` and `env run` require an explicit language/root when the lock has
  multiple matching targets. Launch selects exact locked tool versions, mounts
  the canonical checkout at `/workspace`, disables networking and mise
  auto-installation, drops capabilities, and uses generated state below
  `.ayni/environment/` for the container home.
- Adapter-owned preparation contracts provide deterministic staged argv for
  Rust Cargo, Node npm, Go modules, uv Python, and Gradle. The generic backend
  verifies every staged digest and warms only declared caches. npm uses a
  relocatable seeded output; uv uses a fresh output because virtual environments
  are path-sensitive; Go and Gradle reuse cache-only state. All materialization
  occurs below `.ayni/environment/` with no network and a read-only checkout.
  Unknown providers and unsupported managers fail explicitly.
- Managed `check`, `verify`, and `impact run` mount repository source read-only
  and add a narrow writable mount for generated `.ayni/` evidence and caches.
  Gradle quality execution redirects every project build directory and project
  cache into target- and signal-specific paths below `.ayni/quality/kotlin/`,
  so reports remain writable without weakening the source mount. Interactive
  `env shell` and `env run` retain a read-write checkout because editing is
  their declared purpose.
- Target activation now includes Go toolchain/cache controls, uv frozen/offline
  state, Gradle cache controls, and a backend-derived `JAVA_HOME` for the exact
  mise-managed JDK. Gradle collectors add `--offline --no-daemon` only when the
  managed activation marker is present.

## Non-goals

- Production deployment or application runtime images.
- Regulatory attestation as a product feature.
- Perfect hermeticity across kernel, hardware, clocks, and external services.
- General workstation or dotfile management.
- Silent online recovery during quality execution.

## Dependencies

- [Project 01](01-command-model.md) for `env` command semantics.
- [Project 03](03-environment-planning-locking.md) for the validated build input.
- [Project 05](05-adapter-catalog-platform.md) for language requirements and
  signal-tool catalogs.

## Deliverables

- Minimal multi-architecture Debian base image.
- OCI builder driven only by a validated environment lock.
- Repository image naming and cache-key rules.
- Workspace mount and non-root launcher.
- Explicit per-target environment command wrapper.
- Offline dependency-materialization support.
- Mixed-version polyglot fixtures and CI coverage.

## Definition of done

A mixed Rust and Node monorepo with conflicting Node versions can:

1. Build its environment from a clean machine and committed locks.
2. Mount the checkout without baking source into the image.
3. Materialize repository dependencies without network access.
4. Run `ayni check` without downloads or checkout mutation.
5. Use the correct runtime and package manager for every root.
6. Reuse the image until a material environment input changes.

The built image contains neither repository source nor credentials.
