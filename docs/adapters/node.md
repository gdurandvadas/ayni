# Node Adapter

## Installation

Configure roots in `.ayni.toml` and provide the documented runtime, package
manager, and signal tools when using `--host`. `ayni env show` discovers Node
requirements, and `ayni env lock` resolves runtime/package-manager ranges using
`mise` candidates while reading exact project-tool versions from
`package-lock.json`. Locking does not install dependencies or modify the
checkout; managed environment builds are not implemented yet.

## Signal Coverage

| Signal | Required tool or method | Version contract |
| --- | --- | --- |
| `test` | `vitest` | pinned to 3.2.4 when Ayni installs it |
| `coverage` | `vitest`; `@vitest/coverage-v8` | each package pinned to 3.2.4 when Ayni installs it |
| `size` | built-in Node source scan | no version enforced |
| `complexity` | `eslint`; `@stylistic/eslint-plugin` | no version enforced |
| `deps` | package and workspace manifest graph | no version enforced |
| `mutation` | `@stryker-mutator/core` (opt-in) | no version enforced |

## Focused verification

`verify` writes requested-scope evidence only to `.ayni/verify/last/signals.json`.
Every command accepts an optional `--language node`; unscoped verification is
always valid. The accepted selectors are:

| Signal | `--file` | `--package` | `--name` |
| --- | --- | --- | --- |
| `test` | yes | yes | yes |
| `coverage` | no | no | no |
| `size` | yes | no | no |
| `complexity` | yes | no | no |
| `deps` | yes | yes | no |
| `mutation` | no | no | no |

Package selection remains owned by the resolved npm, pnpm, Yarn, or Bun adapter
path. `--name` is test-only, and `--file` cannot be combined with `--package`.
Unsupported or ambiguous selectors are rejected before a tool runs.

Verification commands carry their originating contract and target, for example:
`ayni verify test --config './.ayni.toml' --language node --root 'apps/web'
--file 'src/example.test.ts' --name 'renders'`. Use only the selectors marked
above; copy the exact command in an artifact finding rather than synthesizing one.

## Contract

Enabled checks come from `[checks]`. Configure roots in `[node].roots`
(default `["."]`), size budgets in `[node.size]`, complexity in
`[node.complexity]`, coverage in `[node.coverage]`, and forbidden edges in
`[node.deps.forbidden]`. Command overrides are optional in
`[node.tooling.test]`, `[node.tooling.coverage]`, and
`[node.tooling.mutation]`; each override requires `command` and may set `args`.

Size requires a budget entry and complexity requires `fn_cyclomatic`; either
missing value produces a clear collector error. Coverage thresholds and
dependency rules are optional: without `line_percent`, coverage has no policy
threshold, and without `node.deps.forbidden`, no edges are forbidden.

Maximum size and complexity boundaries are inclusive (`warn` and `fail` trigger
at equality); coverage is an exclusive minimum boundary (equality passes that
threshold). Line and branch coverage are independently enforced: a configured
metric with missing or unparseable evidence fails the coverage row.

## Configuration Example

```toml
[languages]
enabled = ["node"]

[node]
roots = ["apps/web", "packages/ui"]

[node.tooling.test]
command = "pnpm"
args = ["exec", "vitest", "run", "--reporter=json", "--passWithNoTests"]

[node.size]
"**/*.ts" = { warn = 300, fail = 600, exclude = ["node_modules/**", "dist/**", "coverage/**", ".git/**", ".ayni/**"] }

[node.complexity]
fn_cyclomatic = { warn = 10, fail = 20 }

[node.coverage]
line_percent = { warn = 70, fail = 50 }

[node.deps.forbidden]
"apps/web/**" = ["apps/legacy/**"]
```
