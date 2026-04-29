# Node Adapter

The Node adapter implements the same `LanguageAdapter` and `SignalCollector` contracts as the Rust and Go adapters.

It detects Node workspaces, resolves per-root package manager behavior, and emits canonical `SignalRow` values for each enabled `SignalKind`.

## Module layout

```text
adapters/node/src/
├── lib.rs
├── adapter.rs
├── catalog.rs
└── collectors/
    ├── mod.rs
    ├── test.rs
    ├── coverage.rs
    ├── size.rs
    ├── complexity.rs
    ├── deps.rs
    └── mutation.rs
```

## Signal coverage

| Signal kind  | Collector module             | Source tool / method                                |
| ------------ | ---------------------------- | --------------------------------------------------- |
| `test`       | `collectors/test.rs`         | `vitest` (or policy override)                       |
| `coverage`   | `collectors/coverage.rs`     | `vitest --coverage` (or policy override)            |
| `size`       | `collectors/size.rs`         | file walk + `[node.size]` glob budgets              |
| `complexity` | `collectors/complexity.rs`   | `eslint` JSON metrics (or equivalent override flow) |
| `deps`       | `collectors/deps.rs`         | package/workspace manifest graph + forbidden rules  |
| `mutation`   | `collectors/mutation.rs`     | `stryker` (or policy override)                      |

Every collector outputs:

- a canonical `SignalResult` variant
- a matching typed offender list
- policy-aware `pass` evaluation

## Root and package manager detection

Node roots are discovered from `[node].roots` and adapter detection.

Per root, package manager resolution precedence is:

1. `pnpm-lock.yaml`
2. `yarn.lock`
3. `package-lock.json`
4. `bun.lock` or `bun.lockb`
5. `packageManager` field in `package.json` (fallback when lockfiles are missing)

When no manager can be confidently inferred, runtime behavior falls back to npm-compatible assumptions.

## Tool catalog

Node tools are declared in `catalog.rs` with typed installers. The catalog is consumed directly by `ayni install` and is root-aware for mixed package manager repositories.

Each entry declares:

- install/check behavior
- required signal kinds (`for_signals`)
- opt-in status for expensive checks (for example mutation)

## Policy expectations

Node collectors read these `.ayni.toml` sections:

- `[checks]`
- `[node]` (`roots = [...]`)
- `[node.size]` (glob budgets per extension)
- `[node.complexity]` (`fn_cyclomatic`)
- `[node.coverage]` (`line_percent`)
- `[node.deps.forbidden]` (forbidden dependency edges)
- optional `[node.tooling.test]`, `[node.tooling.coverage]`, `[node.tooling.mutation]` command overrides

If required policy fields are missing, collectors return explicit errors.

## Full TOML example

```toml
[languages]
enabled = ["node"]

[node]
roots = ["apps/web", "packages/ui"]

[node.tooling.test]
command = "pnpm"
args = ["exec", "vitest", "run", "--reporter=json", "--passWithNoTests"]

[node.tooling.coverage]
command = "pnpm"
args = ["exec", "vitest", "run", "--coverage", "--coverage.reporter=json-summary", "--passWithNoTests"]

[node.tooling.mutation]
command = "pnpm"
args = ["exec", "stryker", "run", "--logLevel", "error"]

[node.size]
"**/*.ts" = { warn = 300, fail = 600, exclude = ["node_modules/**", "dist/**", "coverage/**", "build/**", ".git/**", ".ayni/**"] }
"**/*.tsx" = { warn = 300, fail = 600, exclude = ["node_modules/**", "dist/**", "coverage/**", "build/**", ".git/**", ".ayni/**"] }
"**/*.js" = { warn = 300, fail = 600, exclude = ["node_modules/**", "dist/**", "coverage/**", "build/**", ".git/**", ".ayni/**"] }
"**/*.jsx" = { warn = 300, fail = 600, exclude = ["node_modules/**", "dist/**", "coverage/**", "build/**", ".git/**", ".ayni/**"] }
"**/*.mjs" = { warn = 300, fail = 600, exclude = ["node_modules/**", "dist/**", "coverage/**", "build/**", ".git/**", ".ayni/**"] }
"**/*.cjs" = { warn = 300, fail = 600, exclude = ["node_modules/**", "dist/**", "coverage/**", "build/**", ".git/**", ".ayni/**"] }

[node.complexity]
fn_cyclomatic = { warn = 10, fail = 20 }

[node.coverage]
line_percent = { warn = 70, fail = 50 }

[node.deps.forbidden]
"apps/web/**" = ["apps/legacy/**"]
"packages/domain/**" = ["packages/ui/**"]
```

## Command overrides

Node command override tables use:

```toml
[node.tooling.test]
command = "pnpm"
args = ["exec", "vitest", "run", "--reporter=json", "--passWithNoTests"]
```

The same shape applies to `coverage` and `mutation`.

## `ayni install --language node`

`--language node` scopes installation to Node catalog entries only.

```bash
ayni install --language node --repo-root <path>
```

The flow is deterministic and idempotent.

## `ayni analyze --language node`

```bash
ayni analyze --config ./.ayni.toml --language node --output stdout
```

Use markdown output when needed:

```bash
ayni analyze --config ./.ayni.toml --language node --output md
```

## Output guarantees

The adapter emits only core-defined signal kinds and typed payloads. It must not emit ad-hoc row shapes, so reporting remains stable across languages and output formats.
