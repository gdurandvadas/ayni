# Architecture

Ayni is an open-source, multi-language signal tool with strict layer boundaries.

## Dependency Flow

```text
core  <-  adapters/common  <-  adapters/<lang>  <-  cli
```

Changes flow outward: core defines contracts, `adapters/common` provides shared
execution infrastructure (command runner with timeouts, path normalization,
failure scaffolding, catalog execution), language adapters implement local
signal collection, and the CLI orchestrates user intent and output.

## Layer Responsibilities

| Layer | Owns | Does not own |
| --- | --- | --- |
| `core/` | Signal schema, policy model, adapter traits, runtime context, catalog contract types | Tool invocation, CLI ergonomics, persistence |
| `adapters/common/` | Command execution with timeouts, catalog status/install execution, shared path/XML/failure/discovery helpers | Language-specific tool selection or parsing |
| `adapters/<lang>/` | Local tool execution, output parsing, normalization to core types | New signal kinds, untyped payloads, CLI coupling |
| `cli/` | User interface, orchestration, argument parsing, local output | Product semantics, adapter internals |

## Dependency Rules

1. `core` has zero dependencies on other workspace crates.
2. `adapters/common` depends only on `core`.
3. Language adapters depend only on `core` and `adapters/common`.
4. `cli` depends on `core`, `adapters/common`, and `adapters/*`.
5. No reverse dependencies are permitted.
6. Default analysis runs from the repository checkout and writes local artifacts.

## Decision Guide

| Question | Answer |
| --- | --- |
| Where do I add a new signal kind? | `core/` defines the schema first, then adapters implement it |
| Where do I add a new language? | `adapters/<lang>/` implements core traits |
| Where do I change CLI flags? | `cli/` |
| Where do I add local tool invocation? | `adapters/<lang>/`, through the `adapters/common` command runner |
| Where do I add shared adapter plumbing? | `adapters/common/` |
| Where do I add policy thresholds? | `core/` policy model, read from `.ayni.toml` |
| Where do I add report formatting? | `cli/` output modules |

## Prohibited Patterns

| Pattern | Why prohibited |
| --- | --- |
| Core imports adapter or CLI crates | Breaks one-way dependency flow |
| Modifying core to fit CLI ergonomics | CLI adapts to core, not the reverse |
| Adapter defines a new signal kind | Signal vocabulary is closed and owned by core |
| Adapter emits untyped payloads | All output must conform to the core schema |
| CLI directly invokes language analysis tools | Tool invocation belongs in adapters |
| Adapter couples to CLI argument types | Adapters depend only on core |
| Default analysis bypasses repository files or adapter tooling | Breaks the local workflow contract |

## Change Checklist

Before proposing edits:

- [ ] Identified which layer owns the change
- [ ] Verified no reverse dependencies introduced
- [ ] If touching core: change is product-semantic, not CLI ergonomics
- [ ] If touching adapters: output conforms to core signal schema
- [ ] If adding signal kind: defined in core first, then adapter implements
- [ ] Confirmed `install` and `analyze` still work from the repository root
- [ ] Ran `cargo check --workspace --all-features`
- [ ] Ran `cargo clippy --workspace --all-targets --all-features -- -D warnings`

## See Also

- [README](README.md)
- [Signal contract](docs/product/signals.md)
- [Adapter template](docs/adapters/template.md)
- [Configuration reference](docs/product/config.md)
