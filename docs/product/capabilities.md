# Adapter capability tiers

Ayni does not imply identical measurement depth merely because an adapter can detect a language. Capabilities are published per signal so repository policy can distinguish reproducible evidence from experimental support.

## Tier definitions

- **Supported** — the adapter runs a real ecosystem tool or deterministic source analysis, parses typed evidence, and supports managed execution for the documented project shape.
- **Experimental** — the adapter runs a real tool, but normalization is not yet semantically comparable enough for a stable cross-language claim. Opt in only after reviewing the adapter guide and resulting artifact.
- **Unavailable** — Ayni refuses the signal for that adapter. It does not substitute a proxy command or fabricate a score.

## Current matrix

| Adapter | Test | Coverage | Size | Complexity | Dependencies | Mutation |
| --- | --- | --- | --- | --- | --- | --- |
| Rust | Supported | Supported | Supported | Supported | Supported | Unavailable |
| Node | Supported | Supported | Supported | Supported | Supported | Unavailable |
| Go | Supported | Supported | Supported | Supported | Supported | Unavailable |
| Python | Supported | Supported | Supported | Supported | Supported | Supported |
| Kotlin | Supported | Supported | Supported | Supported | Supported | Supported |

“Supported” applies only to the managed project shapes documented in each adapter guide. Unsupported package managers, missing native locks, ambiguous workspaces, and unavailable signal tools fail explicitly rather than falling back to host execution.

## Product rule

Ayni prioritizes truthful evidence over a visually complete matrix. A signal is advertised only when it answers the canonical signal question with real, parsed measurement. New languages and broader project shapes are deferred until existing supported capabilities remain reproducible and semantically honest.

The explicit `--host` mode is an evaluation and compatibility path. It exercises the same policy and typed artifact model, but runtime and tool versions are not locked and its evidence is not provenance-compatible with managed results.
