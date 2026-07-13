# Contributing Language Adapters

This guide is for implementing a new language adapter. For runtime resolution,
setup validation, and failure categories, see the [runtime and setup
rules](../product/runtime.md).

## Layer boundaries

Keep the dependency flow `core <- adapters/common <- adapters/<lang> <- cli`.
`core` owns signal and policy contracts; `adapters/common` owns shared command,
path, discovery, parsing, and catalog infrastructure; a language adapter owns
local detection, runner resolution, tool invocation, parsing, and normalization;
and the CLI owns orchestration and presentation.

An adapter must detect language presence, declare tool requirements, collect
enabled existing signal kinds, normalize tool output to core types, and resolve
execution context using product runtime rules. Do not add a signal kind or an
ad-hoc top-level payload shape in an adapter.

## Required interfaces

Implement `LanguageAdapter` and `SignalCollector` from `ayni-core`.

- `language() -> Language` identifies the language.
- `detect(root) -> DetectResult` reports language presence and confidence.
- `resolve_execution(repo_root, root) -> ExecutionResolution` resolves the
  ancestry-aware runner and setup context.
- `catalog() -> &[CatalogEntry]` declares install requirements.
- `collector() -> &dyn SignalCollector` provides typed collection.

Collectors return `SignalRow` values with a canonical `SignalKind`, language,
typed `result`, `budget`, and `offenders`, plus deterministic `pass`
calculation. Use repository-relative POSIX offender paths, respect `scope`,
`file`, and `package` when supplied, and never emit absolute paths.

## Module and collector layout

Use this crate structure unless a language-specific need requires a small,
documented variation:

```text
src/
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

Each collector module owns one signal kind. For coverage, populate
`CoverageResult.percent` with the headline 0–100 percentage when available;
use `line_percent` and `branch_percent` for available breakdowns. Follow the
[signal contract](../product/signals.md) for all typed fields.

## Catalog conventions

Every external tool invoked for collection is a `CatalogEntry`; the catalog is
the source of truth for `ayni install`. Include a stable tool name, a typed
installer (`Cargo`, `GoInstall`, `NpmGlobal`, `Bundled`, `Custom`, or the
language-appropriate alternative), an optional check command or version probe,
the `for_signals` mapping, and `opt_in` for expensive checks such as mutation.

## Policy conventions

Read global toggles from `[checks]`, language thresholds from
`[<language>.<signal>]`, optional adapter settings from `[<language>]`, and
optional command overrides from
`[<language>.tooling.test|coverage|mutation]`. Fail with a clear error when a
collector's required threshold is missing.

## Documentation format

The adapter user page must use this ordered H2 outline: Installation; Signal
Coverage; Contract; Configuration Example. State roots and detection,
language-specific package-manager or build-system resolution, each tool's
required/optional ownership, and only versions enforced or selected by code;
write “no version enforced” otherwise. Map all six canonical signals to their
tools. Document policy fields, command overrides, and missing-policy behavior
with a language-specific TOML example.

Catalog-managed dependencies are installed only when their related check is
enabled and installation is applied. Runtime and package-manager prerequisites
without catalog installers remain user-owned. Mark mutation tooling optional
when its catalog entry is `opt_in`.

## Prohibited patterns

Do not:

- introduce a signal kind without a core change;
- emit free-form untyped top-level payloads;
- parse source directly when an available tool supplies the metric;
- couple adapter internals to CLI crates; or
- bypass the catalog installation flow.

## Validation checklist

Before merging an adapter:

1. `ayni install` installs or validates every catalog tool.
2. `ayni analyze` emits typed rows for every enabled signal kind.
3. Offender fields match the signal contract.
4. Paths are relative and stable.
5. Adapter documentation names the exact tools, version contract, and policy
   controls.
6. Run `cargo fmt --all -- --check`,
   `cargo clippy --workspace --all-targets --all-features -- -D warnings`,
   `cargo test --workspace --all-features`, and
   `cargo check --workspace --all-features`.
