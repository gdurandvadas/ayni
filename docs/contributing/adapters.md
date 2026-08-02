# Contributing Language Adapters

This guide is for implementing a new language adapter. For runtime resolution,
setup validation, and failure categories, see the [runtime and setup
rules](../product/runtime.md).

## Layer boundaries

Keep the dependency flow `core <- adapters/common <- adapters/<lang> <- cli`.
`core` owns signal and policy contracts; `adapters/common` owns shared command,
path, discovery, parsing, and neutral catalog execution infrastructure; a
language adapter owns local detection, runner resolution, tool invocation,
parsing, normalization, and any adapter-managed catalog behavior; and the CLI
owns orchestration and presentation.

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

For each of the six signals, declare only selectors that the collector applies
faithfully: `file`, `package`, and test-only `name`. The CLI rejects unsupported
or conflicting selectors before a tool starts. Provide a verification target for
each actionable offender that can be revalidated; it is checked against that
signal's declared support and rendered as an exact `ayni verify <signal>`
command. Do not advertise a selector merely because a tool accepts a similar
flag.

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
use `line_percent` and `branch_percent` for available breakdowns. Evaluate each
configured line or branch threshold independently. Configured metrics require
finite, parseable evidence; never substitute another metric or fabricate zero
for missing evidence. Follow the
[signal contract](../product/signals.md) for all typed fields.

## Catalog conventions

Every external tool invoked for collection is a `CatalogEntry`; the catalog is
the source of truth for `ayni install`. Include a stable tool name, a typed
installer (`Cargo`, `GoInstall`, `Bundled`, `Custom`, `AdapterManaged`, or the
language-appropriate alternative), an optional check command or version probe,
the `for_signals` mapping, and `opt_in` for expensive checks such as mutation.
The common catalog runtime is deliberately neutral: an adapter-managed entry
must be handled by its owning adapter runtime, including its manager selection,
status, preparation, and apply behavior. List and `install --check` status
paths must be read-only; preparation belongs only to normal applied setup.

## Policy conventions

Read global toggles from `[checks]`, language thresholds from
`[<language>.<signal>]`, optional adapter settings from `[<language>]`, and
optional command overrides from
`[<language>.tooling.test|coverage|mutation]`. Fail with a clear error when a
collector's required threshold is missing.

## Documentation format

The adapter user page must use this ordered H2 outline: Installation; Signal
Coverage; Focused verification; Contract; Configuration Example. State roots and detection,
language-specific package-manager or build-system resolution, each tool's
required/optional ownership, and only versions enforced or selected by code;
write “no version enforced” otherwise. Map all six canonical signals to their
tools. The focused-verification section must include a six-signal matrix for
`--file`, `--package`, and `--name`, explain rejection behavior, and say that
requested-scope evidence is written to `.ayni/verify/last/signals.json` rather
than the repository completion artifact. Document policy fields, command
overrides, and missing-policy behavior with a language-specific TOML example.

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
2. Unscoped `ayni analyze` emits typed rows for every enabled signal kind and
   is the only repository-completion artifact writer.
3. Each supported focused selector is faithfully applied; unsupported selectors
   are rejected before tool invocation.
4. Offender fields, stable IDs, and exact verification commands match the signal
   contract.
5. Paths are relative and stable.
6. Adapter documentation names the exact tools, version contract, and policy
   controls.
7. Exercise the adapter with real tool fixtures in local and CI coverage,
   including collection, configured thresholds, missing/unparseable configured
   evidence, supported selectors, catalog readiness, and applied installation
   where its manager can make changes. Do not satisfy the contract only with
   mocked command output.
8. Run `cargo fmt --all -- --check`,
   `cargo clippy --workspace --all-targets --all-features -- -D warnings`,
   `cargo test --workspace --all-features`, and
   `cargo check --workspace --all-features`.
