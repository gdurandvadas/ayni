# Managed environments

Ayni's managed environment is a lock-driven OCI execution environment for code
quality commands. It makes the repository's configured runtimes, package
manager, analysis tools, native dependency inputs, and base image explicit
before a check runs.

The quality contract and environment answer different questions:

```text
.ayni.toml                    .ayni.lock + OCI image
What must be measured?   +    Which exact tools run it?
                     ↓
          check / verify / impact run
```

Signal results depend on runtime, package-manager, dependency, and analysis-tool
versions. Managed execution gives humans, coding agents, and CI the same locked
inputs instead of relying on whichever tools happen to be installed on a host.
Enabled signals contribute their required tools to the environment plan; the
configured language roots and native project metadata contribute runtime,
package-manager, and dependency requirements. `env lock` resolves those inputs,
and `env build` prepares them before quality execution.

Managed execution is built into `ayni check`, `ayni verify`, and
`ayni impact run`. Run these commands directly—do not wrap them in `ayni env
run`. Use `--host` only when a repository cannot yet use a supported managed
environment.

For the security consequences of managed execution, advanced access, host
execution, bridge networking, and Docker socket access, see the normative
[security and trust model](security.md).

## Prerequisites

- Install the Ayni CLI separately by following [Installation](/getting-started/installation).
- Use Docker with Buildx for default release-base resolution during `env lock`.
- Install Mise; every `env lock` records its version, and adapters may also use it to resolve runtime or tool selectors.
- Commit the native tool declarations and dependency locks required by each adapter.

Commands that consume an existing lock use Docker first and compatible Podman
second. To create a lock without Docker Buildx base resolution, pass an explicit
`--base <reference>@sha256:<digest>`; other resolution prerequisites still
apply.

## Supported targets

| Language | Managed target |
| --- | --- |
| Rust | Cargo projects and workspaces |
| Node | npm and pnpm projects and workspaces |
| Go | Go modules and workspaces |
| Python | uv projects and workspaces |
| Kotlin | Supported locked Gradle projects and workspaces |

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
| `ayni env storage` | Report Ayni images and repository-local environment state | Read-only |
| `ayni env prune` | Preview stale environment state and engine-wide Ayni image candidates | Dry-run by default; `--apply` removes repository state, while image removal also requires `--images` |
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
runtime capabilities, resource ceilings, warnings, and blocking conflicts. Use
JSON when another tool needs the complete deterministic projection:

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
- opt-in runtime capabilities such as bridge networking and Docker socket access;
- CPU, memory, combined memory-and-swap, process, and open-file ceilings for runtime containers; and
- digests of adapter-owned dependency and requirement inputs.

The lock intentionally omits credentials, host-specific paths, arbitrary system
commands, and checkout-mutating instructions. Equivalent inputs produce stable
lock output; a failed resolution preserves the previous lock.

The current environment plan schema is `0.3.0`, the committed environment lock
schema is `0.5.0`, and the internal OCI image-label schema is `0.5.0`. These are
separate from the current signal-artifact schema `0.4.0` (schema v4); consumers
must not infer one version from another.

By default, locking asks Docker Buildx for the immutable digest of Ayni's
published environment base. An exact alternative can be supplied explicitly:

```sh
ayni env lock --base <reference>@sha256:<digest>
```

If the published base is unavailable, `scripts/build-local-environment-image.sh`
builds a checkout-local base. It prints an `env lock --base` command when the
engine exposes a repository digest; otherwise it explains that the image must
first be pushed to a local registry so the lock can record a pullable
`RepoDigest`. The checkout CI action automates that local-registry step.

## Validation and staleness

Lock-consuming commands fail closed when `.ayni.lock` is missing, invalid, or
stale. A refresh is required when, among other inputs:

- the lock was produced by a different Ayni version;
- `.ayni.toml` changed;
- a locked runtime capability or resource ceiling changed;
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
networking is locked and the operator authorizes that invocation.

Docker socket access is also opt-in. When it is locked and authorized for a
runtime launch, Ayni mounts the host Unix socket and configures Testcontainers'
host override. Reaching sibling containers also requires bridge networking to
be locked and independently authorized; socket access alone retains
`--network none`. `env show` reports the host-control security implication.
This is socket sharing, not a privileged Docker-in-Docker daemon.

`env build` uses networked image and dependency installation regardless of the
runtime bridge setting. The runtime authorization flags described below do not
constrain build-time network access; review staged manifests and locks before
building less-trusted changes.

## Storage lifecycle

`ayni env storage` reports the image tag expected by the current lock, whether a
matching current image is actually present, other Ayni-labeled images visible to
the selected Docker or Podman engine, and classified environment state under
`.ayni/environment/`. It requires a current lock so the expected image and state
generation can be identified. Human and JSON output distinguish total state-root
bytes, classified state bytes, and the unclassified subset. Image sizes are
cumulative, not reclaimable disk space, because OCI layers can be shared.
The JSON fields are `expected_image_tag`, `current_image_present`,
`state_root_logical_size_bytes`, `classified_state_logical_size_bytes`, and
`unclassified_state_logical_size_bytes`.

`ayni env prune` is a read-only dry run by default. It classifies only
non-current state paths with the exact generated fingerprint/preparation shape,
plus the persistent runtime `home` below a non-current lock fingerprint, as
repository-local candidates. `ayni env prune --apply` removes that stale state,
including old persistent compiler caches. The current state paths and
unclassified files remain untouched.

Pruning is not a synchronization boundary. Run it only while managed commands
are stopped and no host process is mutating `.ayni/environment/`. Ayni validates
candidate shape, canonical containment, and symlink status before removal, but
those checks cannot make concurrent untrusted host mutation safe.

Image ownership is Ayni-wide within the selected engine, not repository-specific,
and an exact image can be shared by multiple repositories. Ayni therefore never
deletes images with `--apply` alone. `ayni env prune --apply --images` is the
separate acknowledgement required to delete non-current images carrying Ayni's
explicit ownership label. The command removes them one by one without force;
the current image, legacy images without the ownership label, and images that
the engine refuses to remove remain untouched. Review the engine-wide candidate
list before opting in because Ayni cannot prove exclusive repository ownership.

Ayni never invokes `docker system prune`, `podman system prune`, or a global
build-cache prune. The shared default builder does not provide safe Ayni-only
cache attribution, so build cache and shared-layer reclaim estimates are
intentionally excluded from both commands.

## Runtime resources

Every managed runtime container receives the resource ceilings recorded in the
lock. Defaults are 4 CPUs, 8192 MiB memory, 8192 MiB combined memory and swap,
2048 processes, and 8192 open files. Configure overrides under
`[environment.resources]`; all values must be positive, and
`memory_swap_mib` must be at least `memory_mib`. Equal memory and swap values
disable additional swap under Docker and Podman semantics.

The same ceilings apply to managed quality commands, `env shell`, and `env run`.
They do not constrain `env build`, image storage, or the container engine. See
the [configuration reference](config.md#managed-repository-environment) for
fields and validation, and retain runner-level maximums and disk quotas when
evaluating untrusted code.

`env doctor` reports the locked configuration and detected engine posture; it
does not prove that every host kernel or rootless engine can enforce every
ceiling. A managed launch fails if the engine rejects its flags, while CI and
untrusted-code runners must verify their outer cgroup and swap configuration
independently.

## Operator authorization

A capability in `.ayni.toml` and `.ayni.lock` is a repository request, not
operator consent. A managed launch fails closed until its invocation authorizes
every requested capability:

| Locked request | Required invocation flag |
| --- | --- |
| `network = "bridge"` | `--allow-network` |
| `access = "socket"` | `--allow-docker-socket` |

The checks are independent: a lock requesting both capabilities requires both
flags. Authorization applies only to that invocation, is not written into the
lock, and cannot add a capability the lock does not request. It applies to
managed `check`, `verify`, `impact run`, `env shell`, and `env run`:

```sh
ayni check --allow-network
ayni verify test --allow-docker-socket
ayni impact run --base origin/main --allow-network --allow-docker-socket
ayni env shell --allow-network
ayni env run --allow-docker-socket -- cargo test
```

`--host` does not use managed capability authorization because it bypasses the
container and lock boundary; the host's own network and permissions apply.
`env show`, `env lock`, `env doctor`, and `env build` do not launch a managed
runtime workload and do not accept these authorization flags.

## Quality commands

Quality commands own their managed launch and evidence semantics:

```sh
ayni check
ayni verify test
ayni impact run --base origin/main
```

They select locked targets, run adapter-owned signal commands, normalize the
result, and apply `.ayni.toml` policy. The host checkout is mounted as read-only
input and copied into an ephemeral writable workspace; source changes inside the
quality run are discarded. If the lock requests bridge networking or Docker
socket access, pass the corresponding per-invocation authorization described
above.

## Advanced development access

`env shell` and `env run` expose one target for intentional, arbitrary
development work. They are not part of setup or the quality loop, do not
normalize evidence, and do not apply quality thresholds. When a lock
contains one target, they can select it implicitly:

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

Add `--allow-network` and/or `--allow-docker-socket` before `--` when the locked
target requests those capabilities and the trust review permits them.

Interactive shells and arbitrary `env run` commands mount the host checkout
read-write because they are development tools. Managed quality commands instead
copy a read-only host input into an ephemeral writable workspace. Workspace
source changes are discarded after the run, while generated `.ayni/` state can
persist; the inner quality command's exit code is preserved.

For command flags, see the [CLI reference](/cli). For runner resolution,
timeouts, diagnostics, and failure categories, see [Runtime and setup
rules](/product/runtime).
