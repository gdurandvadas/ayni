# Configuration reference (`.ayni.toml`)

`.ayni.toml` is the quality contract between the repository and the agent. It
defines what the agent should check, which code is in scope, and which limits
the code must stay within.

Policy lives at the repository root. It controls enabled signals, active
languages and roots, per-language thresholds, dependency rules, report settings,
and tool command overrides.

Use `ayni contract show` to print a concise, deterministic projection of the
validated configured policy. Pass `--config <path>` to select a policy other
than `./.ayni.toml`, or `--output json` for a machine-readable, deterministic
projection. JSON output has a `projection_version` field (currently `0.4.0`),
ordered `languages` and `signals` arrays, and structured `warnings`. The
command shows every signal's enabled state for each enabled language,
normalized roots, configured thresholds, size rules,
dependency restrictions, and explicit tool overrides. Both formats include
advisory effectiveness warnings with stable codes; warnings do not make a valid
policy fail. It does not discover
projects, inspect or invoke tools, run adapters, analyze code, or write
artifacts. Use `ayni check` for managed measured results and completion
evidence, or `ayni check --host` for the explicit host path. At the start of a
`check` or `verify` invocation, Ayni removes that command's prior artifact rather
than leaving older evidence available as current if validation or later work
fails; fix the error and rerun to produce fresh evidence.

For the signal vocabulary and schema selection, see [`signals.md`](signals.md);
for current JSON artifact fields, see [schema v4](signals/v4.md).
For runner resolution, setup validation, failure categories, and debug
telemetry, see [`runtime.md`](runtime.md).

---

## Layout

| Section                                        | Role                                                                                                             |
| ---------------------------------------------- | ---------------------------------------------------------------------------------------------------------------- |
| `[checks]`                                     | Turn individual signal kinds on or off (`test`, `coverage`, `size`, `complexity`, `deps`, `mutation`).           |
| `[languages]`                                  | Explicit language list, for example `enabled = ["rust", "node"]`.                                                |
| `[concurrency]`                                | Scheduler settings for running independent analyze roots in parallel.                                            |
| `[execution]`                                  | Tool execution settings such as the per-command timeout.                                                          |
| `[report]`                                     | Console report rendering settings such as offender list limits.                                                  |
| `[environment.*]`                              | Repository-wide tools, packages, runtime capabilities, and resource ceilings for managed execution.             |
| `[rust.*]`, `[go.*]`, `[node.*]`, `[python.*]`, `[kotlin.*]` | Per-language settings (roots, thresholds, dependency rules, and optional tooling command overrides). |

Everything under a language key uses normal TOML **single-bracket** tables and inline tables. There are no `[[array.of.tables]]` blocks in the policy model.

---

## Managed repository environment

Language adapters contribute the runtimes, package managers, and signal tools
needed for their configured quality checks. Repositories may supplement that
inferred plan with tools for any ecosystem supported by Mise and packages from
the Debian provisioning base:

```toml
[environment.tools]
protoc = "35.1"
ruby = "3.4.2"

[environment.debian]
packages = ["libssl-dev", "postgresql-client"]

[environment.docker]
access = "socket"
network = "bridge"

[environment.resources]
cpus = 4
memory_mib = 8192
memory_swap_mib = 8192
pids = 2048
nofile = 8192
```

`environment.tools` values must be exact versions. Tool identifiers and Debian
package specifications are validated as data and are never interpolated as
arbitrary shell commands. Debian entries may use either a package name or an
exact `name=version` specification. The lock records the requested package
specification; use `name=version` when the Debian repository configured by the
base must not select a newer version.

Docker access and network access are disabled by default. `access = "socket"`
mounts the host Docker Unix socket, installs the Debian `docker.io` client, and
configures Testcontainers' daemon-socket and host overrides. Reaching sibling
containers independently requires `network = "bridge"` and per-invocation
network authorization; socket access alone retains `--network none`. Docker
socket access grants the managed environment control over the host Docker
daemon and must only be enabled for a trusted repository. Podman socket access
and privileged Docker-in-Docker are not supported.

The policy requests these capabilities, but it does not authorize a launch.
Managed `check`, `verify`, `impact run`, `env shell`, and `env run` fail closed
unless the operator also passes `--allow-network` for a locked bridge request
and `--allow-docker-socket` for a locked socket request. The flags are
independent, apply to one invocation, and cannot enable a capability absent from
the lock. See the
[security and trust model](security.md#network-and-container-daemon-access)
before authorizing either capability.

Every managed runtime launch also applies resource ceilings. The defaults are:

| Field | Default | Runtime effect | Validation |
| --- | ---: | --- | --- |
| `cpus` | `4` | CPU quota | Positive integer |
| `memory_mib` | `8192` | Memory limit in MiB | Positive integer |
| `memory_swap_mib` | `8192` | Combined memory-and-swap limit in MiB | At least `memory_mib` |
| `pids` | `2048` | Process limit | Positive integer |
| `nofile` | `8192` | Soft and hard open-file limit | Positive integer |

Override only the values the repository needs under `[environment.resources]`;
omitted values keep their defaults, except that an omitted `memory_swap_mib`
tracks the effective `memory_mib`. This preserves the default of no additional
swap when memory is raised or lowered. Set `memory_swap_mib` above
`memory_mib` only when the managed process should be allowed to use swap. These
values enter the environment plan and lock, so changing one makes the lock
stale and requires `env lock` followed by `env build` where the image identity
also changed.

Resource values are repository policy, not a defense against a repository that
can raise its own limits. CI and untrusted-code runners should enforce an outer
maximum and a disk quota independently of Ayni. The runtime ceilings do not
bound `env build` or the container engine itself.

Environment provisioning is open-ended; quality analysis remains available for
the languages represented by registered Ayni adapters. A generic Mise tool does
not imply that Ayni provides quality signals for that language.

For explicit host quality execution (`check --host`, `verify --host`, and
`impact run --host`), Ayni preflights every executable entry point in the selected
adapter execution topology before any collector starts. This includes directly
launched commands and executable dispatch such as Cargo subcommands,
adapter-selected runners and tools, selected command overrides, and every
unqualified `environment.tools` key (for example `protoc`). Bare commands are
resolved through the host `PATH`; absolute commands are checked directly; and
relative commands such as `./gradlew` or `tools/check` are resolved from the
planned execution working directory. Windows `PATHEXT` command resolution is
honored. Missing executables name the affected language root and signal and
recommend rerunning without `--host` to use the locked managed environment.
When coverage-backed test reuse is active, the intentionally unused ordinary
test command is not a prerequisite.

Qualified Mise coordinates such as `ubi:owner/tool` or `npm:package` identify
installation sources, not executable names, so Ayni does not guess a host
binary for them. Preflight validates executable entry points, not arbitrary
arguments or package/plugin imports behind those entry points; adapters remain
responsible for classifying setup failures discovered by the tool itself.

## Excluding paths (size signal)

The **size** signal walks source files under the repo root and compares line counts to budgets. To skip generated or dependency trees, use **`exclude`** on each size entry.

Paths are **repository-relative**, use **forward slashes**, and are matched with the Rust [`glob`](https://docs.rs/glob/) pattern syntax (not gitignore).

```toml
[rust.size]
"*.rs" = {
  warn = 400,
  fail = 700,
  exclude = [
    "target/**",
    "node_modules/**",
    ".git/**",
  ]
}
```

Common patterns:

| Pattern               | Meaning                                          |
| --------------------- | ------------------------------------------------ |
| `target/**`           | Everything under `target/` (Rust build output).  |
| `**/target/**`        | `target` anywhere in the path (unusual layouts). |
| `node_modules/**`     | npm dependencies.                                |
| `dist/**`, `build/**` | Typical build output folders.                    |

`exclude` applies **after** the main glob for that row matches: a file must match the row’s key glob **and** not match any `exclude` pattern.

Omit `exclude` when you want every path that matches the key glob to be considered (defaults to no exclusions).

---

## Size: multiple globs per language

`[rust.size]` is a **map**: each **key** is a glob; each **value** is `{ warn, fail, exclude? }`.

```toml
[rust.size]
"*.rs"           = { warn = 400, fail = 700, exclude = ["target/**"] }
"src/**/*.rs"    = { warn = 500, fail = 900 }
```

Matching uses the map’s key order (sorted lexicographically). If two keys could match the same file, the **first matching rule in that sorted order** wins. Prefer one broad glob plus `exclude`, or keys that do not overlap, to avoid surprises.

---

## Other languages

Use the same shape for Go, Node, Python, and Kotlin when those adapters are enabled:

```toml
[go.size]
"**/*.go" = { warn = 300, fail = 600, exclude = ["vendor/**", ".git/**", ".ayni/**"] }

[go.complexity]
fn_cyclomatic = { warn = 10, fail = 20 }

[go.coverage]
line_percent = { warn = 70, fail = 50 }
```

```toml
[node.size]
"**/*.ts" = { warn = 300, fail = 600, exclude = ["node_modules/**", "dist/**"] }
"**/*.tsx" = { warn = 300, fail = 600, exclude = ["node_modules/**", "dist/**"] }
```

```toml
[node.complexity]
fn_cyclomatic = { warn = 10, fail = 20 }

[node.coverage]
line_percent = { warn = 70, fail = 50 }

[node.deps.forbidden]
"apps/web" = ["apps/legacy-*"]
```

```toml
[python.size]
"**/*.py" = { warn = 400, fail = 800, exclude = [".venv/**", "venv/**", "__pycache__/**", ".git/**", ".ayni/**"] }

[python.complexity]
fn_cognitive = { warn = 10, fail = 15 }

[python.coverage]
line_percent = { warn = 80, fail = 60 }

[python.deps.forbidden]
"src/domain/**" = ["src/presentation/**"]
```

```toml
[kotlin.size]
"**/*.kt" = { warn = 400, fail = 800, exclude = ["build/**", ".gradle/**", ".git/**", ".ayni/**"] }
"**/*.kts" = { warn = 400, fail = 800, exclude = ["build/**", ".gradle/**", ".git/**", ".ayni/**"] }

[kotlin.complexity]
fn_cyclomatic = { warn = 10, fail = 20 }

[kotlin.coverage]
line_percent = { warn = 70, fail = 50 }

[kotlin.deps.forbidden]
"apps/api" = ["libs/ui"]
```

Note: Ayni uses Rust `glob` matching. Brace expansion like `*.{ts,tsx}` is not supported; use separate entries per extension.

Coverage threshold values must be finite percentages from `0` through `100`; complexity threshold values must be finite and non-negative. Ayni rejects invalid ranges and warn/fail ordering while loading the policy, before discovery or tool invocation.

## Tool command overrides

For high-variance tooling, adapters accept command and argument overrides only
for signals whose native output they can parse:

| Adapter | `test` | `coverage` | `mutation` |
| --- | --- | --- | --- |
| Rust | yes | yes | unavailable |
| Node | yes | yes | unavailable |
| Go | yes | yes | unavailable |
| Python | yes | yes | yes |
| Kotlin | yes | yes | yes |

```toml
[rust.tooling.test]
command = "cargo"
args = ["test"]

[go.tooling.coverage]
command = "go"
args = ["test", "./..."]

# Rust and Node only. Opt in when the coverage command runs the complete
# required suite and emits the adapter's normal parseable test evidence.
[node.tooling]
coverage_satisfies_test = true

[node.tooling.test]
command = "pnpm"
args = ["exec", "vitest", "run", "--reporter=json", "--passWithNoTests"]

[python.tooling.test]
command = "uv"
args = ["run", "pytest", "--json-report", "--json-report-file", ".ayni/pytest-report.json"]
```

Notes:

- `command` is required inside each override table and must be non-empty; an empty table is invalid policy rather than a command with an empty executable.
- `args` is optional; when omitted, Ayni uses signal-specific defaults for that language.
- Overrides are command execution overrides only; result parsing still expects the signal collector’s native output shape.
- For Rust and Node, set `coverage_satisfies_test = true` in the language's
  `tooling` table only when its coverage command executes the complete test
  suite. When both checks are enabled, repository `check` runs that command
  once and emits separate typed test and coverage rows instead of first running
  the ordinary test command. Rust still requires parseable Cargo `test result:`
  summaries plus the cargo-llvm-cov JSON report. Node still requires a parseable
  Vitest JSON test report in command output plus a newly generated
  `coverage/coverage-summary.json`. Missing either half fails both rows closed.
  The default is `false`; focused `verify` and impact scopes continue using
  their existing signal-specific execution.
- Mutation is unavailable for Rust, Node, and Go. Those adapters reject the
  signal before tool invocation even if a `tooling.mutation` table is present.
- Managed execution accepts overrides after the adapter has contributed and
  locked its normal runtime and signal-tool requirements. Add any extra override
  executable under `[environment.tools]` or `[environment.debian]`; Ayni does
  not infer arbitrary commands from the override text.

## Language roots

Each language can define one or more roots under its top-level table.

```toml
[languages]
enabled = ["rust", "node"]

[rust]
roots = [".", "crates/api"]

[node]
roots = ["apps/web"]

[python]
roots = ["services/api"]

[kotlin]
roots = ["apps/android"]
```

Rules:

- Roots are repository-relative paths.
- Default is `["."]` when omitted.
- `auto` is not supported in `languages.enabled` in v0.
- Each enabled language may appear only once; duplicates are rejected rather than silently changing target accounting.
- Paths are canonicalized to POSIX style: backslashes become `/`, trailing `/` is removed.
- Absolute, rooted, Windows drive-prefixed, and any parent-component (`..`) roots are rejected during policy validation.
- Before operational commands inspect adapters, root containment is checked three
  ways: policy spelling must be lexically repository-relative, existing paths
  must canonically remain below the canonical repository, and a symlink may not
  resolve outside it. Lexically safe missing roots remain valid; they are not
  dereferenced during validation, so adapter detection and schema-v4 completion
  can report the missing target instead of silently broadening its scope.
- `.` means workspace root and maps to `scope.path = null` in artifacts.

---

## Report

Use `[report]` to tune console-only rendering behavior.

```toml
[report]
offenders_limit = 4
```

`offenders_limit` caps how many offender lines `ayni check` prints per signal
row in either managed or host mode. If omitted, Ayni prints all offenders (no cap).

## Concurrency

Use `[concurrency]` to control how `ayni check` schedules independent roots.
This is scheduler-level parallelism across analyze targets such as `rust/single`,
`rust/mono`, `node/frontend`, or `go/backend`. It does not change how an
individual language tool parallelizes internally.

```toml
[concurrency]
per_language = false
amount = 2
```

Fields:

| Field          | Meaning                                                                                                              |
| -------------- | -------------------------------------------------------------------------------------------------------------------- |
| `per_language` | `false` means `amount` is a single global worker limit; `true` means each language gets its own `amount`-sized pool. |
| `amount`       | Maximum concurrent analyze targets. Must be at least `1`.                                                            |

Examples:

```toml
[concurrency]
per_language = false
amount = 3
```

Runs up to three roots total at once, regardless of language.

```toml
[concurrency]
per_language = true
amount = 2
```

Allows up to two targets per language when the owning adapter has no lower
limit. For a repo with two Node roots and two Go roots, all four may run at the
same time because each language gets separate capacity.

Languages whose tooling serializes on shared state may cap their own pool:
the Rust adapter, for example, never runs more than one target at a time
because Cargo serializes builds on the target-directory lock.

---

## Execution

Use `[execution]` to bound how long a single tool invocation may run. When a
command exceeds the timeout, Ayni kills and reaps it, preserves any captured
diagnostics, and reports a typed failed signal row rather than hanging the run. Command output is streamed as it arrives,
so interactive progress reflects live tool output rather than a post-process
summary.

```toml
[execution]
tool_timeout_seconds = 1800
```

| Field                  | Meaning                                                                  |
| ---------------------- | ------------------------------------------------------------------------ |
| `tool_timeout_seconds` | Wall-clock limit per tool invocation in seconds. Default `1800` (30 min). Must be at least `1`. |

---

## Complexity, coverage, deps

These sections do **not** share the same `exclude` mechanism as size today; behavior is defined per collector (for example which paths external tools scan). Size exclusions are the supported, first-class way to drop build artifacts and vendored trees from **line-count** analysis.

### Threshold semantics

Every threshold has `warn` and `fail` levels. For **maximum** metrics (size and
complexity), boundaries are inclusive: a value equal to `warn` warns and a
value equal to `fail` fails the signal. `fail` takes precedence, so `warn` must
not exceed `fail`:

```toml
[rust.size]
"src/**/*.rs" = { warn = 400, fail = 700 }
```

For that size rule, a 399-line file passes, a 400-line file is a visible warning
but does not fail the row, and a 700-line file is a fail-level offender that
makes the row fail. Complexity uses the same direction: with
`{ warn = 10, fail = 20 }`, a function at 10 warns and one at 20 fails.

For **minimum** metrics (coverage), boundaries are exclusive: a value equal to
either threshold meets that threshold, while a value below `warn` warns and one
below `fail` fails. `fail` takes precedence, so `warn` must be at least `fail`:

```toml
[rust.coverage]
line_percent = { warn = 80, fail = 70 }
```

For that coverage rule, 80% passes, 79% is a warning, and 69% is a failing
offender (70% is only a warning). Coverage reverses the comparison because more coverage is better.
Warnings are retained in reports and aggregate warning counts, while only
fail-level offenders make a row and the aggregate run status fail.

Line and branch coverage are independent minimum metrics. When either is
configured, that metric requires finite, parseable evidence: missing or
unparseable evidence fails the coverage row instead of being treated as zero or
ignored. A configured Go `branch_percent` therefore fails closed because the
standard `go test` profile and `go tool cover` expose statement coverage, not
branch coverage. Python's default coverage command requests branch collection
(`--cov-branch`); command overrides must still produce parseable evidence for
every configured metric.

The effective typed budgets applied to each analyzed row are preserved in the
current artifact's `applied_thresholds` field; see [schema v4](signals/v4.md).

## Output and report safety

`ayni check --output markdown` renders typed findings under **Offenders** and a
**Failures** section only when a collector command failed. Human and Markdown
reports omit finding verification commands; those remain structured in the
schema-v4 JSON artifact and are available explicitly through `ayni verify list`.
Failure entries can include the collector command, working directory, exit code,
and tool message. Reports and artifacts can consequently expose repository paths
and raw tool output; do not publish them without reviewing that diagnostic data.

For machine consumers, `ayni check --output json` and
`ayni check --host --output json` select the same schema-v4 artifact shape.
The supported output values are `human`, `json`, and `markdown`; choose exactly
one `--output` value. See [schema v4](signals/v4.md) for the current schema and
migration posture.

---

## Dependency rules

Forbidden edges use a map from source-endpoint globs to destination-endpoint
globs. Each adapter defines the endpoint paths represented by its dependency
graph.

```toml
[rust.deps.forbidden]
"core" = ["adapters/*", "cli"]
```

For Node, the graph is built from the governing workspace `package.json` and
the package manifests selected by its workspace patterns. Its endpoints are the
complete, repository-relative directories of those packages, and its edges are
workspace-package dependencies declared in package manifests. Match the package
directory itself, not files beneath it:

```toml
[node.deps.forbidden]
"apps/web" = ["packages/server"]
"packages/browser-*" = ["packages/server"]
```

A trailing descendant pattern such as `apps/web/**` does not match the
`apps/web` package endpoint. Node dependency rules also do not inspect
JavaScript, TypeScript, or Svelte imports; source-level import direction remains
the responsibility of a linter such as ESLint.

---

## Completion and focused verification

`ayni check` and `ayni check --host` always evaluate every configured language
root. They are repository completion operations and the sole writer of
`.ayni/last/signals.json`; it does not accept `--file`, `--package`, or
`--language` selectors.

For the fast TDD loop, `ayni verify <signal>` runs one of the six canonical
signals. `--root` selects exactly one normalized root configured for the
selected language and is validated before adapter-owned selectors. `--file`,
`--package`, and test-only `--name` are adapter- and
signal-owned capabilities, not generic filters; unsupported, conflicting, or
ambiguous selections are rejected before a collector invokes a tool. Requested
schema-v4 evidence is persisted at `.ayni/verify/last/signals.json`. It has
`completion.scope = "requested"`, cannot establish repository completion, and
does not overwrite the full completion artifact under `.ayni/last/`.
