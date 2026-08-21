# Rust Adapter

## Installation

Rust roots are directories containing `Cargo.toml`; discovery skips `target`,
`.git`, and `node_modules`. A manifest with `[workspace]` is a workspace
controller, while the repository root is analyzed only when its manifest also
has `[package]`. Cargo commands for a member run from its workspace root.

`cargo` and a Rust toolchain remain user-owned prerequisites for `--host`
execution. `ayni env show` discovers Rust requirements and `ayni env lock`
resolves exact runtime and Cargo catalog-tool versions through `mise`; locking
does not install tools or modify the checkout. `env build` stages the locked
Cargo manifests, requires `Cargo.lock`, and runs `cargo fetch --locked` inside
the image build. `env doctor`, `env shell`, `env run`, managed `check`, and
managed focused `verify` consume that image with Cargo online access disabled;
container networking is disabled unless bridge networking is locked and the
operator authorizes that invocation. Cargo `package.workspace` values that
point to a non-ancestor workspace fail closed because the current environment
ownership contract is ancestry-based.

## Signal Coverage

| Signal | Required tool or method | Version contract |
| --- | --- | --- |
| `test` | `cargo test` | managed: Cargo from the exact locked Rust toolchain; host: no version enforced |
| `coverage` | `llvm-tools-preview`; `cargo-llvm-cov` | managed: component from the exact Rust toolchain and `cargo-llvm-cov` pinned to `0.8.5`; host: no version enforced |
| `size` | built-in Rust source scan | no external tool |
| `complexity` | `rust-code-analysis-cli`; Cargo metadata | managed: an exact tool version selected during locking plus exact-toolchain Cargo; host: no version enforced |
| `deps` | Cargo workspace/dependency graph scan | managed: Cargo from the exact locked Rust toolchain; host: no version enforced |
| `mutation` | unsupported | Rust mutation measurement is not supported; enabling it fails explicitly before tool execution |

## Focused verification

Shared artifact, completion, validation, and exact-command reuse semantics are
defined in [Completion and focused verification](../product/runtime.md#completion-and-focused-verification).
The accepted Rust selectors are:

| Signal | `--file` | `--package` | `--name` |
| --- | --- | --- | --- |
| `test` | no | yes | yes |
| `coverage` | no | no | no |
| `size` | yes | no | no |
| `complexity` | yes | yes | no |
| `deps` | yes | yes | no |
| `mutation` | unsupported | unsupported | unsupported |

Rust source files do not map reliably to Cargo test targets, so use package and
test-name filters for `test`. `--name` is test-only, and `--file` cannot be
combined with `--package`. Unsupported or ambiguous selectors are rejected
before Cargo or another tool runs.

Verification commands carry their originating contract and target, for example:
`ayni verify test --host --config './.ayni.toml' --language rust --root '.' --package
'my-crate' --name 'my_test'`.

## Impact planning

`impact show` and `impact run` resolve the governing Cargo workspace, map
changed Rust source files to the deepest owning package, then include transitive
reverse dependencies even when configured roots name individual members.
Dependency mapping includes normal, development, build, target-specific, and
workspace-inherited aliased Cargo tables. Declared workspace membership and
exclusions are honored; a changed source below a non-member manifest broadens
rather than being assigned to an enclosing package.
Tests and dependency checks use package scope; coverage broadens to the
configured root; size and complexity use exact changed-file scope when the
file still exists. Cargo manifests, lockfiles, Rust toolchain files, `.cargo`
configuration, other configuration-sensitive inputs, and ambiguous ownership
broaden every enabled signal and record an uncertainty.

## Contract

Enabled checks come from `[checks]`. Configure Rust roots in `[rust].roots`
(default `["."]`), size budgets in `[rust.size]`, complexity thresholds in
`[rust.complexity]`, coverage thresholds in `[rust.coverage]`, and forbidden
dependency edges in `[rust.deps.forbidden]`. Command overrides are optional in
`[rust.tooling.test]` and `[rust.tooling.coverage]`; each override requires
`command` and may set `args`. Rust mutation is unsupported, including command
overrides.

Size requires at least one budget entry and complexity requires
`fn_cyclomatic`; either missing value produces a clear collector error.
Coverage thresholds and dependency rules are optional: without `line_percent`,
coverage has no policy threshold, and without `rust.deps.forbidden`, no edges
are forbidden.

Shared boundary rules are defined under [Threshold semantics](../product/config.md#threshold-semantics).
Rust supports independent line and branch coverage; every configured metric
must have finite, parseable evidence or the coverage row fails closed.

## Configuration Example

```toml
[languages]
enabled = ["rust"]

[rust]
roots = ["core", "adapters/rust", "cli"]

[rust.tooling.test]
command = "cargo"
args = ["test"]

[rust.size]
"*.rs" = { warn = 1000, fail = 1600, exclude = ["target/**", ".git/**", ".ayni/**"] }

[rust.complexity]
fn_cyclomatic = { warn = 10, fail = 15 }
fn_cognitive = { warn = 20, fail = 30 }

[rust.coverage]
line_percent = { warn = 40, fail = 35 }

[rust.deps.forbidden]
"core" = ["adapters/*", "cli"]
```
