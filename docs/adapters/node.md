# Node Adapter

## Installation

Configure roots in `.ayni.toml` and provide the documented runtime, package
manager, and signal tools when using `--host`. `ayni env show` discovers Node
requirements, and `ayni env lock` resolves runtime/package-manager ranges using
`mise` candidates while reading exact project-tool versions from either
`package-lock.json` or `pnpm-lock.yaml`. Locking does not install dependencies
or modify the checkout. `env build` stages only locked manifests and native
package-manager inputs, runs an ignore-scripts frozen installation (`npm ci` or
`pnpm install --frozen-lockfile`), and stores `node_modules` as an image seed.
Shell, run, managed check, and managed focused verification copy the seed below
`.ayni/environment/`, mount it over the target, and run the corresponding
offline rebuild in an ephemeral workspace without modifying the host checkout.
npm `file:` and `link:`
dependencies are rejected because their referenced content is not part of the
staged input contract. Yarn and Bun remain unsupported for managed execution;
use `--host` for those package managers.

## Signal Coverage

| Signal | Required tool or method | Version contract |
| --- | --- | --- |
| `test` | `vitest` | managed: exact project version from the native lockfile; host: no version enforced |
| `coverage` | `vitest`; `@vitest/coverage-v8` | managed: exact project versions from the native lockfile; host: no version enforced |
| `size` | built-in JavaScript/TypeScript source scan | no external tool |
| `complexity` | `eslint`; `@typescript-eslint/parser` | managed: exact project versions from the native lockfile; host: no version enforced |
| `deps` | built-in Node workspace package-manifest graph | no external tool |
| `mutation` | unavailable | Ayni rejects Node mutation before invoking a tool; no override or managed tool is accepted |

## Focused verification

Shared artifact, completion, validation, and exact-command reuse semantics are
defined in [Completion and focused verification](../product/runtime.md#completion-and-focused-verification).
The accepted Node selectors are:

| Signal | `--file` | `--package` | `--name` |
| --- | --- | --- | --- |
| `test` | yes | yes | yes |
| `coverage` | no | no | no |
| `size` | yes | no | no |
| `complexity` | yes | no | no |
| `deps` | yes | yes | no |
| `mutation` | unavailable | unavailable | unavailable |

Package selection remains owned by the resolved npm, pnpm, Yarn, or Bun adapter
path. `--name` is test-only, and `--file` cannot be combined with `--package`.
Unsupported or ambiguous selectors are rejected before a tool runs. The Node
mutation signal itself is unavailable, so `ayni verify mutation --language
node` is rejected regardless of selectors or command overrides.

Verification commands carry their originating contract and target, for example:
`ayni verify test --host --config './.ayni.toml' --language node --root 'apps/web'
--file 'src/example.test.ts' --name 'renders'`.

## Impact planning

`impact show` and `impact run` resolve the governing Node workspace, map changed
JavaScript and TypeScript files to the deepest owning package, then include
transitive reverse dependencies even when configured roots name individual
workspace members. Only manifests matched by the workspace patterns enter the
graph. npm, pnpm, Yarn, and Bun lock changes broaden the plan, and package-
scoped dependency execution resolves the governing workspace. Tests and dependency checks use package scope;
coverage broadens to the configured root; size and complexity use exact
changed-file scope when safe. Package manifests, npm lockfiles, common
JSON/YAML/TOML and `*.config.*` inputs, environment files, and ambiguous
ownership broaden every enabled signal and record an uncertainty.

## Contract

Enabled checks come from `[checks]`. Configure roots in `[node].roots`
(default `["."]`), size budgets in `[node.size]`, complexity in
`[node.complexity]`, coverage in `[node.coverage]`, and forbidden edges in
`[node.deps.forbidden]`. Command overrides are optional in
`[node.tooling.test]` and `[node.tooling.coverage]`; each override requires
`command` and may set `args`. With both signals enabled,
`[node.tooling].coverage_satisfies_test = true` opts repository `check` into one
coverage-backed execution that must emit both a Vitest JSON test report and a
new coverage summary. Missing either evidence type fails both rows closed. Node
mutation is unavailable, including through
`[node.tooling.mutation]`: Ayni rejects the signal instead of treating command
success as mutation evidence.

Size requires a budget entry and complexity requires `fn_cyclomatic`; either
missing value produces a clear collector error. Coverage thresholds and
dependency rules are optional: without `line_percent`, coverage has no policy
threshold, and without `node.deps.forbidden`, no edges are forbidden.

The dependency graph contains the governing workspace package and the declared
workspace members matched from `package.json`. Edges come from workspace-package
names declared in `dependencies`, `devDependencies`, `peerDependencies`, and
`optionalDependencies`. Forbidden-edge globs match the complete,
repository-relative package-directory endpoints in that graph. For example,
use `apps/web` to match that package or `packages/*` to match package directories
under `packages`; `apps/web/**` describes descendants and does not match the
`apps/web` package endpoint itself.

The dependency signal does not inspect JavaScript, TypeScript, or Svelte import
statements. Keep source-level import boundaries in the owning linter, for example
with ESLint `no-restricted-imports`; use Ayni dependency rules for relationships
declared between workspace packages.

Shared boundary rules are defined under [Threshold semantics](../product/config.md#threshold-semantics).
Node line and branch coverage are independently enforced: a configured metric
with missing or unparseable evidence fails the coverage row.

## Configuration Example

```toml
[languages]
enabled = ["node"]

[node]
roots = ["apps/web", "packages/ui"]

[node.tooling]
coverage_satisfies_test = true

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
"apps/web" = ["apps/legacy-*"]
```
