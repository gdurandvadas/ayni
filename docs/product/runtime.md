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

## Install Validation

`ayni install --apply` must prove the foundation is usable before succeeding.
For every enabled detected language/root it validates:

- execution resolution exists
- required catalog tools are invocable through the resolved setup context
- generated artifact paths under `.ayni/work/<language>/<root>/` are writable

Set `[<language>.foundation].validate_install = false` only when a repository
intentionally wants scaffolding or installation without validation.

## Read-only Install Readiness

`ayni install --check` evaluates an existing repository setup without changing
it. Unlike normal `install` and `install --apply`, check mode requires an
existing valid `.ayni.toml` and never scaffolds policy, edits `.gitignore`,
prepares a package manager, installs a tool, creates validation directories, or
writes an artifact. It uses the same adapter-owned configured-target detection,
execution resolution, catalog selection, and requirement status checks used by
the install runtime. Repeated `--language` selectors filter the enabled,
configured policy targets; without selectors every enabled target is checked.

Human-readable output is the default. `--output json` is valid only with
`--check` and emits exactly one pretty-printed JSON document on stdout, ending
in a newline. Diagnostics for policy loading, validation, or internal command
failure are written to stderr, with no JSON document on stdout. Check mode
conflicts with `--apply`.

The JSON readiness contract is versioned independently from signal artifacts:

- `readiness_version`: currently `0.1.0`.
- `state`: `ready` or `not_ready`.
- `targets`: selected configured targets in validated policy order. Each entry
  contains `language`, `configured_root`, `detection` (`detected` and optional
  `reason`), nullable `execution`, and `requirements`.
- `execution`: when resolved, the adapter-owned `runner`, `resolved_from`,
  `kind`, `source`, `confidence`, `ambiguous`, `install_cwd`, and `exec_cwd`.
- `requirements`: enabled catalog entries in adapter catalog order, with
  `name`, ordered `signals`, and `status` (`current`, `missing`, or `outdated`).
- `issues`: an ordered array following target and catalog traversal. Each issue
  has `language`, `configured_root`, `stage` (`detection`, `resolution`, or
  `requirement`), `message`, and an optional `requirement` name.

The state is `not_ready`, and the process exits non-zero, when any configured
target is undetected or unresolved or any enabled requirement is missing or
outdated. `ready` exits zero. Paths in detection and resolution details reflect
the supplied repository root, so callers that need byte-identical output across
invocations should supply the same root spelling.

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

## Config Materialization

Generated config should stay small. Ayni materializes foundation settings only
when behavior would otherwise be surprising, such as workspace-ancestor runner
resolution or explicit validation opt-out.

## Partial Success

Ayni should report the full repository state whenever possible. A failed tool
row should not suppress valid rows from other roots or languages.

`ayni analyze` is repository-only: it plans and evaluates every configured
language root and is the sole writer of `.ayni/last/signals.json`. Use
`ayni verify <signal>` for focused evidence for one of the six canonical
signals. Its adapter-owned selectors are validated before tool invocation, and
its requested-scope evidence is written separately to
`.ayni/verify/last/signals.json`.
