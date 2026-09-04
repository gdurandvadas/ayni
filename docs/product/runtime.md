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

## Managed environment ownership

The [managed environment guide](environments.md) is the normative owner of the
lock, build, validation, target-support, and workspace lifecycle. In summary,
managed `check`, `verify`, and `impact run` consume an explicit lock and image;
they do not create or update either implicitly. Unsupported package managers or
build shapes fail explicitly, while `--host` retains adapter-owned runner
resolution as a compatibility escape hatch.

The [security and trust model](security.md) owns execution-boundary guidance.
In particular, networking is disabled only under the default managed profile;
an explicitly locked bridge request and per-invocation `--allow-network`
authorization enable it. Managed quality commands use a read-only host checkout
as input, whereas `env shell` and `env run` intentionally mount the checkout
read-write. Environment plan, lock, and image schema versions are listed in the
managed environment guide and are independent of the signal-artifact schema.

## Host prerequisite preflight

Explicit host execution validates the selected physical command topology before
any signal collector starts. Adapters declare executable entry points used by
each signal, including resolved package managers, wrappers, and executable
subcommand dispatch. Bare commands
use `PATH`, absolute commands are checked directly, and relative commands are
resolved from `exec_cwd`; Windows executable-extension lookup follows
`PATHEXT`. Unqualified repository-wide `[environment.tools]` keys are also
checked, while qualified Mise provider coordinates are not guessed as binary
names.

The check is topology-aware: when one coverage execution supplies both test and
coverage evidence, the unused ordinary test command is not required. Preflight
does not interpret arbitrary command arguments or verify package imports and
plugins that execute behind a valid entry point. A missing executable aborts
before scheduling and identifies its language root and signal. Repository
`check` persists that failure as current incomplete resolution evidence in
`.ayni/last/signals.json`; no signal row is fabricated.

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
resolution, cwd, command, exit code, stdout, and stderr for each tool run. It
also emits local `[profile]` phase, collector, and command `elapsed_ms` timing;
profiling is diagnostic output and never enters canonical signals.

Every adapter command uses the configured wall-clock timeout. The runner reads
stdout and stderr concurrently through a bounded queue and retains at most 16
MiB per stream. Complete progress lines are individually bounded, and terminal
delivery coalesces each tool to its latest pending line; lifecycle events remain
lossless. Exceeding either collector capture limit terminates and reaps the
command (its process group on Unix) and emits a typed `output_limit` failure
with explicit minimum truncated-byte counts. Persisted runner-failure messages
retain a 32 KiB head-and-tail excerpt per stream rather than copying the full
capture into signal artifacts.

Timeout, Ctrl-C, and orchestration cancellation terminate and reap active
commands; cancellation emits a typed `cancelled` failure. SIGINT is bridged to
cooperative cancellation for interactive and plain/JSON/Markdown checks,
focused verification, and impact planning/execution. OCI image builds use a
different capture policy: their logs continue streaming and the returned
diagnostics retain a rolling 16 MiB tail per stream, with omitted-byte counts,
rather than failing a healthy build only because its log was verbose.

On Unix, cleanup addresses the process group created for each direct tool. A
host command that deliberately starts a new process group or session can escape
that cleanup and outlive Ayni. Host execution is a trusted compatibility mode,
not containment for hostile repository code; use an appropriately isolated
managed or external runner when that distinction matters.

Managed Rust checks keep `CARGO_TARGET_DIR` below the existing lock- and
preparation-scoped `.ayni/environment/` cache so compatible runs can reuse
compilation artifacts. This is mutable local cache state, not completion
evidence. Stale generations are reported and safely selected for cleanup by
[`env storage` and `env prune`](environments.md#storage-lifecycle).

## Completion and focused verification

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

All adapters permit unscoped verification and accept an optional matching
`--language`. Selector support is adapter-specific and documented in each
adapter's focused-verification matrix. Unsupported or ambiguous selectors are
rejected before a tool runs. Copy the exact verification command from an
artifact finding instead of synthesizing or broadening one.
