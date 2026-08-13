# Runtime and Setup Rules

This document defines product behavior for runner resolution, setup validation,
tool diagnostics, and partial success. It applies to every language adapter.

## Execution Resolution

Ayni discovers language roots separately from execution context.

- `target_root` is the repository root or leaf package being analyzed.
- `resolved_from` is the file or ancestor directory that determined the runner.
- `install_cwd` is where setup commands add or validate tools.
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

The clean-slate `init` and `env` command vocabulary is active. `env show` now
builds a read-only, deterministic environment plan for configured Rust and Node
targets. Repository initialization, readiness diagnosis, locking, managed
environment builds, and environment support for Go, Python, and Kotlin are not
implemented yet. Those unavailable operations do not reuse the removed
`install` behavior.

Until the environment projects are complete, quality execution requires the
explicit `--host` escape hatch. Host mode retains adapter-owned runner
resolution and typed failure evidence; it does not install tools or mutate
repository dependencies.

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

## Config Materialization

Generated config should stay small. Ayni materializes foundation settings only
when behavior would otherwise be surprising, such as workspace-ancestor runner
resolution or explicit validation opt-out.

## Partial Success

Ayni should report the full repository state whenever possible. A failed tool
row should not suppress valid rows from other roots or languages.

`ayni check --host` is repository-only: it plans and evaluates every configured
language root and is the sole writer of `.ayni/last/signals.json`. Use
`ayni verify <signal>` for focused evidence for one of the six canonical
signals. Its adapter-owned selectors are validated before tool invocation, and
its requested-scope evidence is written separately to
`.ayni/verify/last/signals.json`.
