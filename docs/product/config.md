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
projection. JSON output has a `projection_version` field (currently `0.1.0`),
ordered `languages` and `signals` arrays, and structured `warnings`. The
command shows every signal's enabled state for each enabled language,
normalized roots, configured thresholds, size rules,
dependency restrictions, and explicit tool overrides. Both formats include
advisory effectiveness warnings with stable codes; warnings do not make a valid
policy fail. It does not discover
projects, inspect or invoke tools, run adapters, analyze code, or write
artifacts. Use `ayni check` for managed measured results and completion
evidence, or `ayni check --host` for the explicit host path.

For the signal vocabulary and schema selection, see [`signals.md`](signals.md);
for current JSON artifact fields, see [schema v3](signals/v3.md).
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
| `[rust.*]`, `[go.*]`, `[node.*]`, `[python.*]`, `[kotlin.*]` | Per-language settings (roots, thresholds, dependency rules, and optional tooling command overrides). |

Everything under a language key uses normal TOML **single-bracket** tables and inline tables. There are no `[[array.of.tables]]` blocks in the policy model.

---

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

## Tool command overrides

For high-variance tooling, each language can override command and args for `test`, `coverage`, and `mutation`.

```toml
[rust.tooling.test]
command = "cargo"
args = ["test"]

[go.tooling.coverage]
command = "go"
args = ["test", "./..."]

[node.tooling.mutation]
command = "pnpm"
args = ["exec", "stryker", "run", "--logLevel", "error"]

[python.tooling.test]
command = "uv"
args = ["run", "pytest", "--json-report", "--json-report-file", ".ayni/pytest-report.json"]
```

Notes:

- `command` is required inside each override table.
- `args` is optional; when omitted, Ayni uses signal-specific defaults for that language.
- Overrides are command execution overrides only; result parsing still expects the signal collector’s native output shape.
- Overrides are host-execution features. Managed environment planning rejects an
  enabled signal with an override because the lock cannot prove or provision an
  arbitrary command. Use `--host` when an override is required.

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
- Paths are canonicalized to POSIX style: backslashes become `/`, trailing `/` is removed.
- Absolute, rooted, Windows drive-prefixed, and any parent-component (`..`) roots are rejected during policy validation.
- Before operational commands inspect adapters, root containment is checked three
  ways: policy spelling must be lexically repository-relative, existing paths
  must canonically remain below the canonical repository, and a symlink may not
  resolve outside it. Lexically safe missing roots remain valid; they are not
  dereferenced during validation, so adapter detection and schema-v3 completion
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

Allows up to two Rust roots and two Node roots to run at the same time. For a
repo with `rust/backend`, `rust/worker`, and `node/web`, that means Rust can
run two targets concurrently while Node gets its own separate capacity.

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
current artifact's `applied_thresholds` field; see [schema v3](signals/v3.md).

## Output and report safety

`ayni check --output markdown` renders typed findings under **Offenders** and adds a
**Failures** section only when a collector command failed. Failure entries can
include the command, working directory, exit code, and tool message. Markdown
and the schema-v3 JSON artifact can consequently expose repository paths and raw
tool output; do not publish them without reviewing that diagnostic data.

For machine consumers, `ayni check --output json` and
`ayni check --host --output json` select the same schema-v3 artifact shape.
The supported output values are `human`, `json`, and `markdown`; choose exactly
one `--output` value. See [schema v3](signals/v3.md) for the current schema and
migration posture.

---

## Dependency rules

Forbidden edges use the same map style as size: keys and values are glob patterns describing crate or package paths.

```toml
[rust.deps.forbidden]
"core" = ["adapters/*", "cli"]
```

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
schema-v3 evidence is persisted at `.ayni/verify/last/signals.json`. It has
`completion.scope = "requested"`, cannot establish repository completion, and
does not overwrite the full completion artifact under `.ayni/last/`.
