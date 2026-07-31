# Rust Adapter

## Installation

Rust roots are directories containing `Cargo.toml`; discovery skips `target`,
`.git`, and `node_modules`. A manifest with `[workspace]` is a workspace
controller, while the repository root is analyzed only when its manifest also
has `[package]`. Cargo commands for a member run from its workspace root.

`cargo` and a Rust toolchain are user-owned prerequisites. Ayni can install
catalog-managed tools when their checks are enabled and installation is applied:
`llvm-tools-preview` through Rustup, and the remaining tools through Cargo.

## Signal Coverage

| Signal | Required tool or method | Version contract |
| --- | --- | --- |
| `test` | `cargo test` | no version enforced |
| `coverage` | `llvm-tools-preview`; `cargo-llvm-cov` | `cargo-llvm-cov` pinned to 0.8.5; `llvm-tools-preview`: no version enforced |
| `size` | built-in Rust source scan | no version enforced |
| `complexity` | `rust-code-analysis-cli` | no version enforced |
| `deps` | Cargo workspace/dependency graph scan | no version enforced |
| `mutation` | `cargo-mutants` (opt-in) | no version enforced |

## Focused verification

`verify` writes requested-scope evidence only to `.ayni/verify/last/signals.json`.
Every command accepts an optional `--language rust`; unscoped verification is
always valid. The accepted selectors are:

| Signal | `--file` | `--package` | `--name` |
| --- | --- | --- | --- |
| `test` | no | yes | yes |
| `coverage` | no | no | no |
| `size` | yes | no | no |
| `complexity` | yes | yes | no |
| `deps` | yes | yes | no |
| `mutation` | no | no | no |

Rust source files do not map reliably to Cargo test targets, so use package and
test-name filters for `test`. `--name` is test-only, and `--file` cannot be
combined with `--package`. Unsupported or ambiguous selectors are rejected
before Cargo or another tool runs.

## Contract

Enabled checks come from `[checks]`. Configure Rust roots in `[rust].roots`
(default `["."]`), size budgets in `[rust.size]`, complexity thresholds in
`[rust.complexity]`, coverage thresholds in `[rust.coverage]`, and forbidden
dependency edges in `[rust.deps.forbidden]`. Command overrides are optional in
`[rust.tooling.test]`, `[rust.tooling.coverage]`, and
`[rust.tooling.mutation]`; each override requires `command` and may set `args`.

Size requires at least one budget entry and complexity requires
`fn_cyclomatic`; either missing value produces a clear collector error.
Coverage thresholds and dependency rules are optional: without `line_percent`,
coverage has no policy threshold, and without `rust.deps.forbidden`, no edges
are forbidden.

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
