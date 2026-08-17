# Runtime and Setup Rules

This document defines product behavior for runner resolution, setup validation,
tool diagnostics, and partial success. It applies to every language adapter.

## Execution Resolution

Ayni discovers language roots separately from execution context.

- `target_root` is the repository root or leaf package being analyzed.
- `resolved_from` is the file or ancestor directory that determined the runner.
- `install_cwd` is the package-manager or workspace root used for environment preparation.
- `exec_cwd` is where analysis commands run.

Resolution must be ancestry-aware. A leaf package may execute through a manager
or workspace defined above it.

Configured roots remain contained in the selected repository: their policy
spelling is lexically relative, existing paths are canonically checked against
the canonical repository, and symlinks that escape it are rejected. A safe
missing root is not made into an error during policy validation; it remains a
configured target so detection and completion can produce an explicit issue.

Every analyzed root records:

- `runner`
- `resolved_from`
- `kind` (`direct_root`, `workspace_ancestor`, or `fallback`)
- `source`
- `confidence`
- `ambiguous`
- `install_cwd`
- `exec_cwd`

## Language Rules

- Rust resolves `cargo` from `Cargo.toml`; member crates use a Cargo workspace
  ancestor when one is present.
- Go resolves `go` from `go.mod`; `go.work` is recorded as a workspace ancestor
  while module commands still run from the module root.
- Node resolves `npm`, `pnpm`, `yarn`, or `bun` from direct root markers first,
  then from workspace ancestor package-manager markers.
- Python resolves `uv`, `poetry`, `pdm`, `pipenv`, `hatch`, or `python` from
  direct root markers first, then supported workspace ancestors.
- Kotlin resolves Gradle from the configured root, preferring `./gradlew`, then
  `gradlew.bat`, then `gradle`.

## Environment provisioning status

See [Managed environments](environments.md) for the user-facing lifecycle,
lock, build, and execution guide. The `env` command vocabulary is active. `env show`
builds a read-only, deterministic environment plan for configured Rust, Node,
Go, uv Python, and Gradle Kotlin targets. `env lock` resolves exact requirements
and atomically writes schema `0.4.0`, a versioned, fingerprinted `.ayni.lock`; older schema locks are replaced atomically during an explicit `env lock`;
unchanged inputs produce byte-stable output across equivalent checkout directory
names, and failed resolution preserves the previous lock. Adapter-owned
resolvers interpret each ecosystem's selectors. Project tools must already be
present in the native npm/uv/Gradle inputs, while isolated Cargo and Go tools
are provisioned from exact provider coordinates. A lock contains a validated immutable OCI base
reference and SHA-256 digest. The default release-base digest is resolved with
Docker Buildx; `env lock --base <reference>@sha256:<digest>` accepts an explicit
base. `env doctor`, `env build`, `env shell`, and `env run` use Docker first and
compatible Podman second. They derive generic mise input from the validated
lock and image identity from both the lock fingerprint and canonical dependency
preparation digest; they never implicitly create a lock or image. Adapters additionally provide structured Cargo, npm, Go module, uv, and Gradle
preparation commands over lock-digested inputs; the temporary build context
contains only those allowlisted manifests, locks, wrapper files, and generated
scaffolds—never application source or credentials. Image labels are checked
before reuse and launch.

Launch mounts the canonical checkout at `/workspace`, selects one locked target,
uses the invoking identity, disables mise auto-install and networking, mounts a
writable generated home, and applies read-only-root and privilege restrictions.
Interactive `env shell` and arbitrary `env run` intentionally mount the checkout
read-write so their declared development commands can edit it. Managed `check`,
`verify`, and `impact run` instead mount repository source read-only and expose
only generated `.ayni/` state as writable. Select an ambiguous shell/run target
with `--language`; add `--root` when that language still has multiple locked
roots. A root selector always requires a language selector.

`env build` runs adapter-owned preparation plans only in the isolated staged
context. Managed launch copies seeded npm dependencies, creates fresh
non-relocatable uv environments, and reuses prepared Cargo, Go, uv, and Gradle
caches from fingerprinted state below `.ayni/environment/`. Outputs are mounted
over their repository locations with the checkout read-only. Per-target runtime
and offline variables—including Go cache/toolchain controls, uv frozen state,
and Gradle/JDK activation—are injected only into that target's collector
process. Normal managed launch, check, and focused verification remain
network-disabled.

Managed `check` and focused `verify` are available for locked Rust, npm Node,
Go modules, uv Python projects, and locked Gradle Kotlin builds, preserving
inner quality exit codes. Yarn, Bun, non-uv Python managers, and unsupported
Gradle build shapes fail explicitly. `--host` retains adapter-owned runner
resolution as an escape hatch; it does not install tools or mutate repository
dependencies. No operation reuses removed `install` behavior.

## Failure Categories

Tool failures should become failed signal rows when a valid row can be emitted.
Adapter aborts are reserved for invalid policy/contracts or Ayni internal faults.

Failure categories:

- `repo_code_issue`: tests, coverage, or mutation fail because repository code
  or imports are broken.
- `repo_setup_issue`: tools, runners, generated paths, or repository setup are
  not usable.
- `ayni_internal_issue`: Ayni cannot satisfy its own contract.

Default output shows a short failure cause and category. `--debug` prints runner
resolution, cwd, command, exit code, stdout, and stderr for each tool run.

Every adapter command uses the configured wall-clock timeout. The runner
captures stdout and stderr concurrently, forwards complete output lines while
the command is still running for live progress, and on timeout kills and reaps
the child before returning typed timeout diagnostics with captured output.

## Partial Success

Ayni should report the full repository state whenever possible. A failed tool
row should not suppress valid rows from other roots or languages.

`ayni check` is repository-only: it plans and evaluates every configured
language root and is the sole writer of `.ayni/last/signals.json`; `--host`
changes execution location, not completion semantics. Use
`ayni verify <signal>` for managed focused evidence, or add `--host` as the
explicit escape hatch, for one of the six canonical signals. Its adapter-owned selectors are validated before tool invocation, and
its requested-scope evidence is written separately to
`.ayni/verify/last/signals.json`. Terminal and Markdown run reports never append
rerun commands; commands remain available in structured artifacts. Use
`ayni verify list` for the last repository artifact or pass
`--artifact .ayni/verify/last/signals.json` for focused evidence.
