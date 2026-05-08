# Architecture

Ayni is an open-source, multi-language signal tool with strict layer boundaries.

## Dependency Flow

```text
core  <-  adapters  <-  cli
```

Changes flow outward: core defines contracts, adapters implement local signal
collection, and the CLI orchestrates user intent and output.

## Layer Responsibilities

| Layer | Owns | Does not own |
| --- | --- | --- |
| `core/` | Signal schema, policy model, adapter traits, runtime context | Tool invocation, CLI ergonomics, persistence |
| `adapters/` | Local tool execution, output parsing, normalization to core types | New signal kinds, untyped payloads, CLI coupling |
| `cli/` | User interface, orchestration, argument parsing, local output | Product semantics, adapter internals |

## Dependency Rules

1. `core` has zero dependencies on other workspace crates.
2. `adapters/*` depend only on `core`.
3. `cli` depends on `core` and `adapters/*`.
4. No reverse dependencies are permitted.
5. Default analysis runs from the repository checkout and writes local artifacts.

## Decision Guide

| Question | Answer |
| --- | --- |
| Where do I add a new signal kind? | `core/` defines the schema first, then adapters implement it |
| Where do I add a new language? | `adapters/<lang>/` implements core traits |
| Where do I change CLI flags? | `cli/` |
| Where do I add local tool invocation? | `adapters/` |
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

- [Product overview](docs/product/overview.md)
- [Signal contract](docs/product/signals.md)
- [Adapter template](docs/adapters/template.md)
- [Configuration reference](docs/product/config.md)
