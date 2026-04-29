# Adapter Template

This document is the implementation guide for adding a new language adapter.

## Goals

Each adapter must:

1. Detect whether the language is present in a repository.
2. Declare tool requirements in a typed catalog.
3. Collect enabled signal kinds.
4. Normalize tool output into Ayni core signal types.

Adapters must not define new signal kinds or invent ad-hoc top-level payload shapes.

## Required interfaces

Implement `LanguageAdapter` and `SignalCollector` from `ayni-core`.

- `language() -> Language`: language identity.
- `detect(root) -> DetectResult`: language presence and confidence.
- `catalog() -> &[CatalogEntry]`: install requirements.
- `collector() -> &dyn SignalCollector`: typed signal collection.

`SignalCollector` must return `SignalRow` values with:

- canonical `SignalKind`
- language tag
- typed `result`, `budget`, `offenders`
- deterministic `pass` calculation

## Module layout

Recommended crate structure:

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

Each collector module should own only one signal kind.

For **`coverage`**, populate `CoverageResult.percent` with the headline percentage (0–100) when the tool provides one, and use `line_percent` / `branch_percent` for breakdowns when available (see the **[signal contract](../product/signals.md)**).

## Catalog conventions

Each `CatalogEntry` should include:

- stable tool name
- typed installer (`Cargo`, `GoInstall`, `NpmGlobal`, `Bundled`, `Custom`, etc.)
- optional check command/version probe
- `for_signals` listing required signal kinds
- `opt_in` for expensive checks (for example mutation)

Catalog entries are the source of truth for `ayni install`.

## Policy conventions

Adapters must read:

- global toggles from `[checks]`
- language-specific thresholds from `[<language>.<signal>]`
- optional adapter-specific settings from `[<language>]`
- optional command overrides from `[<language>.tooling.test|coverage|mutation]`

Adapters should fail with clear errors when required thresholds are missing.

## Full TOML example (`[<lang>]`)

Use this as the documentation template for any new adapter:

```toml
[languages]
enabled = ["<lang>"]

[<lang>]
roots = ["."]

[<lang>.tooling.test]
command = "<test-command>"
args = ["<arg1>", "<arg2>"]

[<lang>.tooling.coverage]
command = "<coverage-command>"
args = ["<arg1>", "<arg2>"]

[<lang>.tooling.mutation]
command = "<mutation-command>"
args = ["<arg1>", "<arg2>"]

[<lang>.size]
"<glob-pattern>" = { warn = 300, fail = 600, exclude = ["<generated-dir>/**", ".git/**", ".ayni/**"] }

[<lang>.complexity]
fn_cyclomatic = { warn = 10, fail = 20 }

[<lang>.coverage]
line_percent = { warn = 70, fail = 50 }

[<lang>.deps.forbidden]
"<source-path-glob>" = ["<target-path-glob>", "<target-path-glob-2>"]
```

Replace placeholders (`<lang>`, command names, globs, thresholds) with language-specific values and commands.

## Scope and path rules

- Use repo-relative POSIX paths in offenders.
- Respect `scope`, `file`, and `package` when provided by CLI.
- Avoid absolute paths in emitted rows.

## Prohibited patterns

Do not:

- add new signal kinds without core changes
- emit free-form untyped payloads at top level
- parse source code directly when a tool already provides metrics
- couple adapter internals to CLI crates
- bypass catalog installation flow

## Validation checklist

Before merging an adapter:

1. `ayni install` installs/validates all catalog tools.
2. `ayni analyze` emits typed rows for each enabled signal kind.
3. Offender fields match the signal contract.
4. Paths are relative and stable.
5. Adapter docs reference the exact tools, commands, and policy knobs.
