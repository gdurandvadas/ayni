# Configuration reference (`.ayni.toml`)

Policy lives at the repository root in **`.ayni.toml`**. It is the contract between your repo and Ayni: which signals run, which languages are active, and per-language thresholds.

For the signal vocabulary and JSON artifact fields, see [`signals.md`](signals.md).

---

## Layout

| Section | Role |
| -------- | ----- |
| `[checks]` | Turn individual signal kinds on or off (`test`, `coverage`, `size`, `complexity`, `deps`, `mutation`). |
| `[languages]` | Explicit language list, for example `enabled = ["rust", "node"]`. |
| `[concurrency]` | Scheduler settings for running independent analyze roots in parallel. |
| `[report]` | Console report rendering settings such as offender list limits. |
| `[rust.*]`, `[go.*]`, `[node.*]` | Per-language settings (roots, thresholds, and optional tooling command overrides). |

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

| Pattern | Meaning |
| -------- | ------ |
| `target/**` | Everything under `target/` (Rust build output). |
| `**/target/**` | `target` anywhere in the path (unusual layouts). |
| `node_modules/**` | npm dependencies. |
| `dist/**`, `build/**` | Typical build output folders. |

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

Use the same shape for Node when that adapter is enabled:

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
```

Notes:

- `command` is required inside each override table.
- `args` is optional; when omitted, Ayni uses signal-specific defaults for that language.
- Overrides are command execution overrides only; result parsing still expects the signal collector’s native output shape.

## Language roots

Each language can define one or more roots under its top-level table.

```toml
[languages]
enabled = ["rust", "node"]

[rust]
roots = [".", "crates/api"]

[node]
roots = ["apps/web"]
```

Rules:

- Roots are repository-relative paths.
- Default is `["."]` when omitted.
- `auto` is not supported in `languages.enabled` in v0.
- Paths are canonicalized to POSIX style: backslashes become `/`, trailing `/` is removed.
- `.` means workspace root and maps to `scope.path = null` in artifacts.

---

## Report

Use `[report]` to tune console-only rendering behavior.

```toml
[report]
offenders_limit = 4
```

`offenders_limit` caps how many offender lines `ayni analyze` prints per
signal row. If omitted, Ayni prints all offenders (no cap).

## Concurrency

Use `[concurrency]` to control how `ayni analyze` schedules independent roots.
This is scheduler-level parallelism across analyze targets such as `rust/single`,
`rust/mono`, `node/frontend`, or `go/backend`. It does not change how an
individual language tool parallelizes internally.

```toml
[concurrency]
per_language = false
amount = 2
```

Fields:

| Field | Meaning |
| ----- | ------- |
| `per_language` | `false` means `amount` is a single global worker limit; `true` means each language gets its own `amount`-sized pool. |
| `amount` | Maximum concurrent analyze targets. Must be at least `1`. |

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

---

## Complexity, coverage, deps

These sections do **not** share the same `exclude` mechanism as size today; behavior is defined per collector (for example which paths external tools scan). Size exclusions are the supported, first-class way to drop build artifacts and vendored trees from **line-count** analysis.

---

## Dependency rules

Forbidden edges use the same map style as size: keys and values are glob patterns describing crate or package paths.

```toml
[rust.deps.forbidden]
"core" = ["adapters/*", "cli"]
```

---

## CLI scope flags

Narrowing a run does not replace `.ayni.toml`; it limits **what** is analyzed in that invocation. See the CLI reference: [`../cli.md`](../cli.md) (`--file`, `--package`, `--language`).
