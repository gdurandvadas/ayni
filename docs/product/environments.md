# Managed environments

Ayni's managed environment is a lock-driven OCI execution environment for code
quality commands. It makes the repository's configured runtimes, package
manager, analysis tools, native dependency inputs, and base image explicit
before a check runs.

Managed execution is the default for `ayni check`, `ayni verify`, and
`ayni impact run`. Use `--host` only when a repository cannot yet use a
supported managed environment.

## Supported targets

| Language | Managed target |
| --- | --- |
| Rust | Cargo projects and workspaces |
| Node | npm projects and workspaces |
| Go | Go modules and workspaces |
| Python | uv projects |
| Kotlin | Supported locked Gradle builds |

Yarn, Bun, non-uv Python package managers, and unsupported Gradle build shapes
fail explicitly rather than silently running with a different setup. Node
managed environments support npm and pnpm workspaces with committed native
lockfiles.
Language adapters own ecosystem discovery and resolution; the environment
backend consumes their validated plans without interpreting language manifests.
See the ecosystem-specific guides for [Rust](/adapters/rust),
[Node](/adapters/node), [Go](/adapters/go), [Python](/adapters/python), and
[Kotlin](/adapters/kotlin).

## Lifecycle

The lifecycle is explicit. Commands that consume a lock do not create or update
one implicitly.

| Command | Purpose | State behavior |
| --- | --- | --- |
| `ayni env show` | Discover and explain the environment plan | Read-only |
| `ayni env lock` | Resolve exact requirements and write `.ayni.lock` | Updates the committed lock |
| `ayni env doctor` | Validate the lock, engine, image, and prepared state | Read-only |
| `ayni env build` | Build the image and prepare locked dependencies | Updates engine-managed image state |
| `ayni env shell` | Open an interactive shell for one locked target | May materialize `.ayni/environment/`; checkout is read-write |
| `ayni env run -- <command>` | Run an arbitrary command for one locked target | May materialize `.ayni/environment/`; checkout is read-write |

A typical first-time setup is:

```sh
ayni env show
ayni env lock
ayni env build
ayni env doctor
ayni check
```

After changing `.ayni.toml`, a runtime declaration, package-manager metadata,
or a native dependency lock, inspect and refresh the environment explicitly:

```sh
ayni env show
ayni env lock
ayni env build
```

Commit `.ayni.lock` so humans, agents, and CI use the same resolved environment.
Docker or Podman stores the generated OCI image; Ayni stores materialized
caches, dependencies, and execution state below `.ayni/environment/`. Generated
state must not be committed.

## Plan and lock

`env show` reads `.ayni.toml`, asks each enabled language adapter to discover
its configured roots, and produces an explainable plan. The plan includes target
platforms, runtimes, package-manager ownership, signal tools, dependency locks,
warnings, and blocking conflicts. Use JSON when another tool needs the complete
deterministic projection:

```sh
ayni env show --output json
```

`env lock` resolves the plan to exact versions and writes the versioned,
fingerprinted `.ayni.lock` atomically. The lock records:

- the Ayni and lock-schema versions;
- the quality-contract path and digest;
- an immutable OCI provisioning base and SHA-256 digest;
- supported target platforms;
- exact runtimes, package manager, and signal-tool providers per target;
- repository-wide exact Mise tools and validated Debian package specifications;
- opt-in runtime capabilities such as bridge networking and Docker socket access; and
- digests of adapter-owned dependency and requirement inputs.

The lock intentionally omits credentials, host-specific paths, arbitrary system
commands, and checkout-mutating instructions. Equivalent inputs produce stable
lock output; a failed resolution preserves the previous lock.

By default, locking asks Docker Buildx for the immutable digest of Ayni's
published environment base. An exact alternative can be supplied explicitly:

```sh
ayni env lock --base <reference>@sha256:<digest>
```

If the published base is unavailable, `scripts/build-local-environment-image.sh`
builds a checkout-local base and prints the matching `env lock --base` command.

## Validation and staleness

Lock-consuming commands fail closed when `.ayni.lock` is missing, invalid, or
stale. A refresh is required when, among other inputs:

- the lock was produced by a different Ayni version;
- `.ayni.toml` changed;
- a locked manifest, runtime declaration, wrapper, or dependency lock changed;
- current adapter discovery no longer matches the locked targets; or
- the current host architecture is not represented in the lock.

Run `ayni env doctor` for a concise diagnosis. Repair the declared source input,
then rerun `env lock` and `env build`; do not hand-edit generated fingerprints or
digests.

## Build and dependency preparation

`env build` stages only adapter-approved manifests, native locks, wrapper files,
and generated scaffolding into the build context. Application source and
credentials are not copied into that context. The build installs the locked
runtime and analysis tooling, warms provider caches, and retains only declared
dependency outputs.

Project tools for npm, pnpm, uv, and Gradle must already be represented in
their native project inputs. Cargo and Go analysis tools may be provisioned from
exact adapter-owned provider coordinates. Repository-wide tools and Debian
packages may be declared under `[environment]`; these supplement rather than
replace adapter requirements. Managed runs materialize or reuse prepared state
below `.ayni/environment/` and execute with networking disabled unless bridge
networking is explicitly enabled.

Docker socket access is also opt-in. When enabled, Ayni mounts the host Unix
socket, configures Testcontainers to reach sibling containers, and reports the
host-control security implication in `env show`. This is socket sharing, not a
privileged Docker-in-Docker daemon.

## Running commands

When a lock contains one target, `env shell` and `env run` can select it
implicitly:

```sh
ayni env shell
ayni env run -- cargo test
```

When selection is ambiguous, pass `--language`; add `--root` with the language
when that language still has multiple locked roots. `--root` is never accepted
without `--language`.

```sh
ayni env run --language node --root apps/web -- npm test
ayni env shell --language rust
```

Interactive shells and arbitrary `env run` commands mount the checkout
read-write because they are development tools. Managed quality commands instead
mount repository source read-only, expose only generated `.ayni/` state as
writable, and preserve the quality command's exit code.

For command flags, see the [CLI reference](/cli). For runner resolution,
timeouts, diagnostics, and failure categories, see [Runtime and setup
rules](/product/runtime).
