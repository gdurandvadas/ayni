# Node Adapter

The Node adapter implements the same `LanguageAdapter` and `SignalCollector` contracts as the Rust and Go adapters.

It detects Node workspaces, resolves per-root package manager behavior, and emits canonical `SignalRow` values for each enabled `SignalKind`.
Runtime behavior follows the product-level [runtime and setup rules](../product/runtime.md).

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

Per root, package manager resolution is ancestry-aware. Precedence is:

1. `pnpm-lock.yaml`
2. `yarn.lock`
3. `package-lock.json`
4. `bun.lock` or `bun.lockb`
5. `packageManager` field in `package.json` (fallback when lockfiles are missing)
6. workspace ancestor `package.json` with `workspaces` and package-manager markers

When no manager can be confidently inferred, runtime behavior falls back to npm-compatible assumptions.

## Tool catalog

Node tools are declared in `catalog.rs` with typed installers. The catalog is consumed directly by `ayni install` and is root-aware for mixed package manager repositories.

Each entry declares:

| Field | Meaning |
| ----- | ------- |
| install/check behavior | typed installer and probe rules |
| `for_signals` | required signal kinds |
| `opt_in` | expensive or optional checks such as mutation |

## Setup contract

**Runtime and package-manager assumption:** Node.js is available on `PATH` and
each root has a supported package-manager context. Ayni resolves `pnpm`, Yarn,
npm, or Bun from lockfiles and `packageManager` metadata in the documented
precedence; without a confident match it expects npm-compatible behavior. Ayni
does not install Node or a package manager. It installs catalog packages as
root-local development dependencies only with `install --apply`.

| Tool/package | Signals | Required or optional | Ownership |
| --- | --- | --- | --- |
| `node` | all Node signals | required | Ayni detects/expects it; runtime installation is user-owned |
| `vitest` | test, coverage | required when either check is enabled | Ayni adds the local dev dependency with `--apply`, otherwise detects/reports |
| `@vitest/coverage-v8` | coverage | required when coverage is enabled | Ayni adds the local dev dependency with `--apply`, otherwise detects/reports |
| `eslint`, `@stylistic/eslint-plugin` | complexity | required when complexity is enabled | Ayni adds local dev dependencies with `--apply`, otherwise detects/reports |
| `@stryker-mutator/core` | mutation | optional (`mutation` is opt-in) | Ayni adds the local dev dependency with `--apply` when enabled, otherwise detects/reports |

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

`--language node` scopes installation to Node catalog entries only. Repeat the
flag for a polyglot repository, such as `--language node --language python`;
repeated values are deduplicated.

```bash
ayni install --language node --repo-root <path>
```

The flow is deterministic and idempotent.

It does not modify `AGENTS.md`; run `ayni agents sync --repo-root <path>` for
the explicit, idempotent managed-guidance update.

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
