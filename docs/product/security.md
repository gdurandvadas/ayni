# Security and trust model

This page is the normative security model for Ayni's managed environments and
host execution. The [managed environment guide](environments.md) owns lifecycle
and operational behavior; this page owns the trust assumptions and security
consequences of choosing an execution mode or capability.

## Security posture

Ayni's managed environment is designed for reproducibility and damage
containment; it is not a security boundary for hostile repository code. Under
the default profile, quality commands run with networking disabled, a private
read-only source snapshot, a read-only container root filesystem, all Linux
capabilities dropped, and privilege escalation disabled. `env shell`, `env
run`, and `--host` intentionally relax these boundaries for development
workflows. Requesting and authorizing Docker-socket access grants effective
control of the selected container daemon, while requesting and authorizing
bridge networking permits network access.

Use these relaxed modes only with trusted repositories. Evaluate untrusted
contributions on ephemeral, least-privileged runners or virtual machines
without host credentials or daemon access.

## Trust boundaries by mode

| Mode or capability | Host checkout | Network and host authority | Intended trust level |
| --- | --- | --- | --- |
| Managed `check`, `verify`, and `impact run` with default capabilities | The live checkout is not mounted; a bounded private snapshot feeds an ephemeral writable copy | Container network disabled; no daemon socket | Trusted repository code where reproducibility and containment are required |
| `env shell` and `env run` | Mounted read-write at `/workspace` | Locked capabilities still apply and require operator authorization; arbitrary commands can edit the checkout | Trusted interactive development only |
| `--host` | Commands operate directly on host files | Host identity, network, environment, and installed tools apply; there is no container boundary | Trusted code on a host prepared for direct execution |
| Bridge networking | Depends on the selected execution mode | Locked policy request plus `--allow-network` permits container network access and possible data exfiltration | Trusted code that has a documented network requirement |
| Docker socket access | Depends on the selected execution mode | Locked policy request plus `--allow-docker-socket` grants effective control of the selected container daemon | Highly trusted code on a dedicated or otherwise isolated daemon |
| `env build` | Application source is excluded from the staged build context | The build can access the network and execute package-manager, dependency, and image-build logic | Reviewed manifests, locks, base image, and dependency supply chain |

Bridge networking and Docker socket access are independent opt-in
capabilities. Selecting either changes the trust boundary for every launched
workload that consumes the resulting lock.

## Default managed quality execution

For managed quality commands, Ayni creates a bounded private snapshot of the
canonical host checkout, mounts that snapshot read-only, copies it into an
ephemeral writable `/workspace`, and discards workspace source changes when the
container exits. It keeps the checkout's `.ayni/` writable
so evidence and prepared environment state can persist.

The runtime uses `--read-only`, drops all Linux capabilities, applies
`no-new-privileges`, and creates temporary writable filesystems only for
declared paths such as `/tmp`, the generated home, and the ephemeral workspace.
It does not request privileged-container mode or disable the container engine's
default seccomp or Linux security-module policy. Docker runs with the invoking
numeric user and group; compatible Podman uses its keep-id user namespace.
Do not invoke Ayni as root when a non-root identity is available.

These controls reduce accidental checkout mutation and common privilege-
escalation paths. They do not make code safe merely because it runs in a
container.

## Resource limits

Every managed runtime launch also applies the resource ceilings recorded in the
lock. The defaults are 4 CPUs, 8192 MiB memory with no additional swap, 2048
processes, and 8192 open files. Repositories can override them under
`[environment.resources]`; the resolved values are included in the plan, lock,
and stale-lock check.

These limits reduce common denial-of-service impact, but they are not an
independent boundary against a repository that can raise its own policy limits.
They also do not bound `env build`, disk consumption, image storage, or the
container engine. Enforce outer CPU, memory, process, execution-time, and disk
ceilings on CI or untrusted-code runners, and review resource-policy changes
before refreshing the lock.

## Development and host execution

`env shell` and `env run` are intentional advanced-access tools. Their
read-write checkout mount lets an arbitrary command modify or delete repository
files, including untracked files. They do not produce normalized quality
evidence. Use them only when that direct development access is intended, and
review changes after the command exits.

`--host` bypasses managed isolation entirely. Adapter commands run with the
caller's filesystem permissions, host networking, installed runtimes and tools,
and relevant host environment. It is a compatibility escape hatch, not a
hardened execution mode.

## Network and container-daemon access

The default managed launch uses no container network. Authorized bridge
networking permits outbound connections, dependency downloads, and potential
exfiltration by repository or toolchain code. Prefer a runner-level egress
allowlist when network access is necessary; a general bridge is broader than a
package-registry exception.

Capabilities use a two-key model. The repository must request the capability in
the policy and lock, and the operator must authorize that invocation with
`--allow-network` and/or `--allow-docker-socket`. If both are requested, both
flags are required. The flags are not persisted and cannot enable a capability
that is absent from the lock. This check applies to managed `check`, `verify`,
`impact run`, `env shell`, and `env run`; it does not turn `--host` into an
isolated mode.

Docker socket access resolves the selected engine endpoint, rejects non-Unix
endpoints and non-socket filesystem objects, and mounts the canonical Unix
socket path. This is socket sharing rather than a privileged Docker-in-Docker
container, but the security
consequence is still substantial: code that can call the daemon can generally
create containers, mount paths visible to that daemon, and exercise the
daemon's authority. Capability dropping and `no-new-privileges` inside the Ayni
container do not constrain API requests accepted by the daemon.

For workloads that genuinely need Testcontainers or another daemon client,
prefer a dedicated rootless engine or an ephemeral virtual machine, keep bridge
networking disabled unless separately required, and do not expose a shared
developer or production daemon.

## Build-time execution and supply chain

`env build` is an execution boundary, not a passive lockfile operation. Ayni
limits the build context to adapter-approved manifests, native locks, wrapper
files, and generated scaffolding; it does not copy application source or
repository credentials into that context. The build nevertheless runs image,
package-manager, dependency, and tool-installation logic with network access.
A compromised base image, registry response, build plugin, wrapper, package
lifecycle hook, or transitive dependency can therefore execute during the
build.

`--allow-network` authorizes a locked runtime capability; it does not control
the network used by `env build`. Runtime resource ceilings likewise do not
constrain the image build. Isolate and bound the outer builder separately.

Review `.ayni.toml`, `.ayni.lock`, staged manifests, native dependency locks,
wrapper changes, and the immutable base-image digest before building changes
from a less-trusted source. Do not pass secrets through build arguments or
embed them in manifests. Use registry credentials with the narrowest scope and
prefer external short-lived credentials managed by the container engine.

## Release supply chain

The managed environment-image release workflow pins third-party GitHub Actions
to full commit SHAs and pins its Debian base and Rust builder images by digest.
Architecture images publish an SBOM and build provenance, then Trivy blocks the
multi-architecture release path on fixable `CRITICAL` operating-system or
library vulnerabilities. The immutable multi-architecture image digest is
signed keylessly with Cosign using GitHub's short-lived OIDC identity.

These controls strengthen publisher provenance; they do not eliminate registry,
dependency, workflow, or scanner risk. Ayni currently locks and validates an
immutable image digest but does not verify the Cosign signature at `env lock`,
`env build`, or runtime launch. Runtime signature-policy enforcement is a future
trust boundary. Until it exists, operators that require publisher verification
must verify the signature externally before accepting the digest. Constrain
both the GitHub workflow identity and OIDC issuer; for a released digest:

```sh
cosign verify \
  --certificate-identity 'https://github.com/gdurandvadas/ayni/.github/workflows/release.yml@refs/heads/main' \
  --certificate-oidc-issuer 'https://token.actions.githubusercontent.com' \
  'ghcr.io/gdurandvadas/ayni-env@sha256:<release-digest>'
```

The workflow pushes architecture images by digest without persistent
architecture tags, scans the same immutable digests used to assemble the
index, signs the index digest under a staging reference, promotes that exact
digest to the release tag, and verifies the promoted digest against the expected
main-branch workflow identity and GitHub OIDC issuer. Rust dependencies are audited with an
exact cargo-audit version in the pull-request quality workflow and in a weekly
default-branch workflow, so newly published RustSec advisories do not have to
wait for another code change. Dependabot maintains Cargo and GitHub Action
references; maintainers must rotate cargo-audit, the Debian and Rust-builder
images, and the Mise pins in
`.github/docker/ayni-env.versions` deliberately and verify their published
digests/checksums before merging.

## Secrets and diagnostics

Managed quality execution snapshots tracked files and unignored untracked files
from the checkout according to a host-produced Git manifest. Ignored build and
dependency trees are omitted to bound Docker workspace materialization. The
live checkout is not mounted into the quality container, but every selected
file is present in its private snapshot, so `.gitignore` should not be treated
as the only secrecy control: keep credentials outside the repository tree and
review any process granted network or daemon access.

Ayni does not wholesale mount the host home directory or forward the complete
host environment into managed containers. Explicitly mounted engine sockets,
runtime capabilities, repository contents, and values deliberately supplied by
the operator remain in scope. Host execution inherits the host process context.

Debug output, live tool output, Markdown reports, and JSON artifacts may contain
repository paths, command lines, test names, and raw tool diagnostics. Review
them before publishing or attaching them to an issue, and redact credentials or
private source details.

## Persistent state and evidence limitations

Files below `.ayni/`, including prepared caches and signal artifacts, are local
mutable state. They are not signed, tamper-evident attestations and do not prove
that an independent trusted runner executed a command. Any user or process with
write access can replace them, and a compromised dependency can affect a cache
that later managed runs reuse.

Treat artifacts as reproducible diagnostic evidence, validate their schema and
provenance before automated consumption, and generate release or compliance
evidence on a clean controlled runner. Discard `.ayni/environment/`, container
images, and runner state when moving between trust domains rather than sharing
them across unrelated or mutually untrusted repositories.

Run `env prune` only when managed commands are stopped and no host process is
mutating `.ayni/environment/`. Its exact-shape, containment, and symlink checks
reduce accidental deletion; they are not a synchronization or security boundary
against concurrent hostile filesystem changes.

## Untrusted repositories and pull requests

Do not build or execute an untrusted pull request on a normal developer machine
solely because Ayni uses containers. Use an ephemeral runner or virtual machine
with:

- no container-daemon socket and no privileged containers;
- no repository, cloud, signing, SSH, or package-registry credentials;
- networking disabled or constrained by an external egress policy;
- a non-root identity and a dedicated rootless engine where practical;
- reviewed Ayni resource ceilings plus stricter outer CPU, memory, process,
  execution-time, and disk limits enforced by the runner platform; and
- disposable image, cache, workspace, and `.ayni/` state.

Review policy, lock, manifest, wrapper, and workflow changes before `env build`.
If reviewing them safely is not possible, do not execute the contribution. Run
trusted release and signing work in a separate environment from untrusted
contribution evaluation.

## Residual risk

Containers share a kernel—or, with desktop container products, a managed
virtual-machine kernel—and execute repository and toolchain code. The default
controls reduce accidental mutation and common privilege-escalation paths, but
they do not protect against container-runtime vulnerabilities, resource-
exhaustion attacks, compromised dependencies, malicious diagnostics, or
capabilities explicitly granted by the operator.

To report an unexpected boundary violation, follow the repository's
[security policy](https://github.com/gdurandvadas/ayni/security/policy).
