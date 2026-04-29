# Rust Adapter

The Rust adapter is the reference implementation of the Ayni language adapter contracts.

It detects Cargo workspaces, resolves Rust tooling requirements from a typed catalog, and emits typed `SignalRow` values for each enabled `SignalKind`.

## Module layout

```text
adapters/rust/src/
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

- `test`: `collectors/test.rs` using `cargo test`
- `coverage`: `collectors/coverage.rs` using `cargo llvm-cov`
- `size`: `collectors/size.rs` using walkdir + `[rust.size]` budgets
- `complexity`: `collectors/complexity.rs` using `rust-code-analysis-cli`
- `deps`: `collectors/deps.rs` using Cargo workspace/dependency graph scan
- `mutation`: `collectors/mutation.rs` using `cargo mutants`

Every collector outputs:

- a canonical `SignalResult` variant
- a matching typed offender list
- policy-aware `pass` evaluation

## Tool catalog

Rust tools are declared in `catalog.rs` using `CatalogEntry` + `Installer`:

- `Installer::Bundled` for built-in `cargo`
- `Installer::Rustup` for `llvm-tools-preview`
- `Installer::Cargo` for `cargo-llvm-cov`, `rust-code-analysis-cli`, `cargo-mutants`

Each entry declares `for_signals`, so `ayni install` can infer install needs from enabled signal kinds.

## Policy expectations

Rust collectors read these policy sections:

- `[checks]`
- `[rust.size]` (Rust line-count budgets as `glob = { warn, fail, exclude? }`)
- `[rust.complexity]`
- `[rust.coverage]`
- `[rust.deps]` (including `[rust.deps.forbidden]` for glob-based rules)
- optional `[rust.tooling.test]`, `[rust.tooling.coverage]`, `[rust.tooling.mutation]` command overrides
- Optional empty `[rust]` for forward-compatible extras

If required policy fields are missing, collectors return explicit errors.

## Full TOML example

```toml
[languages]
enabled = ["rust"]

[rust]
roots = ["core", "adapters/rust", "cli"]

[rust.tooling.test]
command = "cargo"
args = ["test"]

[rust.tooling.coverage]
command = "cargo"
args = ["llvm-cov", "--workspace", "--json", "--summary-only"]

[rust.tooling.mutation]
command = "cargo"
args = ["mutants", "--list"]

[rust.size]
"*.rs" = { warn = 1000, fail = 1600, exclude = ["target/**", ".git/**", ".ayni/**"] }

[rust.complexity]
fn_cyclomatic = { warn = 10, fail = 15 }
fn_cognitive = { warn = 20, fail = 30 }

[rust.coverage]
line_percent = { warn = 40, fail = 35 }

[rust.deps.forbidden]
"core" = ["adapters/*", "cli"]
"adapters/*" = ["cli"]
```

## Output guarantees

The adapter must never emit ad-hoc free-form row shapes. It only emits core-defined signal kinds and typed payloads, so output remains consistent with future Go and Node adapters.

## Catalog as a data structure

The tool catalog above is not just documentation — it is a first-class data structure inside the adapter crate. Each catalog entry declares:

- `name` — identifier (e.g. `cargo-llvm-cov`)
- `install_cmd` — the command to install it (e.g. `cargo install cargo-llvm-cov`)
- `check_cmd` — a command that exits zero if the tool is already present
- `for_signals` — which signals require this tool (e.g. `["coverage"]`)
- `opt_in` — whether the tool is only installed when the corresponding check is enabled in `.ayni.toml`

`ayni install --language rust` iterates this catalog at install time. This is the single source of truth for "what does the Rust adapter need to collect signals" — if a tool is added to the adapter, it must be added to the catalog, and `ayni install` picks it up automatically.

Every tool documented in the catalog section above must be present in this data structure. If documentation and the catalog drift, the catalog wins — the documentation is updated to match.

---

## `ayni install --language rust`

`--language rust` scopes installation to Rust only. Omitting `--language` installs for every enabled language.

```bash
ayni install --language rust --repo-root <path>
```

**What it does:**

1. Creates `.ayni.toml` at the repo root if missing — policy config with default thresholds for size, complexity, deps, and coverage toggle
2. Ensures `.ayni/` is in `.gitignore` — keeps generated artifacts out of source control
3. For each entry in the Rust catalog:
   - Skip if the entry is `opt_in` and its corresponding check is disabled in `.ayni.toml`
   - Run `check_cmd`. If it exits zero, the tool is already present — skip.
   - Otherwise, run `install_cmd` to install the missing tool.
4. Reports a summary: which tools were installed, which were already present, which were skipped, and any install failures.

The flow is deterministic and idempotent. Running `ayni install --language rust` twice in a row performs zero installs the second time.

---

## Branch-level diff strategy

All signals run on the full workspace. There is no diff-scoped analysis for size, complexity, deps, or coverage.

**Why full analysis is required:**

- Coverage is cumulative. If a test file changes but the source file doesn't, a diff-only scan reports zero coverage change — which is wrong.
- Dependency analysis requires the full graph. Partial graphs produce false negatives on forbidden edges.
- Cross-file effects: a change in file A can affect coverage or complexity of file B through macros, traits, or shared test modules.

**Exception — mutation testing:**

`cargo-mutants --in-diff` scopes mutant generation to files changed in the current branch. This is appropriate and intentional: full mutation testing on a large workspace takes hours; diff-scoped mutation on a PR takes minutes. This is the one signal where diff-scope is correct by design.

To compute the diff for `--in-diff`:

```bash
git diff origin/main...HEAD > .ayni/branch.diff
cargo mutants --in-diff .ayni/branch.diff
```

**New code annotation:**

The CLI computes `git diff <base>...HEAD --name-only` at run time and annotates offenders from all adapters with `new_code: true` when the offender file is in changed paths. This allows the GitHub check run to distinguish:

- _"This complexity violation exists in code you changed this PR"_ (new code)
- _"This complexity violation is pre-existing in untouched code"_ (existing code)

This mirrors SonarCloud's "New Code" vs "Overall Code" separation — the same full analysis, with a different presentation layer on top.

Set `AYNI_MERGE_BASE` to override the default diff base (`origin/main`, then `main`, then `HEAD~1`).
