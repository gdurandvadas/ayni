# Product Overview

## Why Ayni Exists

AI agents increase delivery speed, but they also increase change volume beyond
what humans can reliably review line by line. Ayni shifts quality control from
"inspect every diff" to "measure behavior and structural health locally".

Ayni treats code quality as a stream of machine-readable signals that can guide
humans and AI agents.

## What Ayni Provides

Ayni is a local-first, language-agnostic signal tool. It runs language-specific
tooling through adapters, then normalizes everything into one typed schema.

It provides:

- a closed signal vocabulary shared across languages
- a policy file describing what healthy code means
- an `AGENTS.md` managed block with repository guidance for AI coding agents
- a unified local artifact that humans and AI agents can consume
- terminal and markdown reports for local workflows

It does not generate code, review style, upload data, require accounts, or
replace your test and analysis tools. It orchestrates and normalizes local
signals.

## Signal Vocabulary

Ayni emits six signal kinds:

| Signal kind | Purpose |
| --- | --- |
| `test` | Test execution health and failures |
| `coverage` | Coverage depth and weak or uncovered areas |
| `size` | Module and file growth against line-count budgets |
| `complexity` | Function-level complexity against thresholds |
| `deps` | Forbidden architectural dependency edges |
| `mutation` | Test effectiveness against simulated behavioral change |

Each row has a stable shape:

- `kind`: which signal this is
- `language`: adapter language that produced it
- `scope`: where measurement was taken
- `pass`: whether the signal meets policy
- `result`: typed measurement payload
- `budget`: typed thresholds or budget payload
- `offenders`: typed list of violations
- `delta_vs_previous` and `delta_vs_baseline`: optional local comparison fields

For the detailed field-level contract, see `docs/product/signals.md`.

## How AI Models Benefit

AI models need structured feedback, not prose-only summaries. Ayni gives models:

- deterministic targets from explicit budgets
- local file and function context through offender rows
- prioritization through pass/fail status and offender severity
- cross-language consistency through shared signal kinds
- local markdown and JSON artifacts that can feed iterative repair loops

This enables loops such as: generate code -> run Ayni -> read offending rows ->
patch -> rerun.

## Language Adapters And Role Separation

The platform is intentionally separated:

- `core`: schema, policy model, adapter contracts, and runtime context
- `adapters/*`: language tooling integration and normalization to core types
- `cli`: local entrypoint for `install`, `analyze`, and output

Dependency direction remains one-way:

```text
core  <-  adapters  <-  cli
```

Adapters own tool invocation details. Core owns product semantics. The CLI owns
argument parsing, orchestration, and local output.

## Configuration Model

`.ayni.toml` is language-aware. The full reference is in
[Configuration reference](config.md).

At a glance:

- `[checks]` controls which signal kinds run.
- `[languages]` selects enabled languages.
- `[rust.size]`, `[node.size]`, and similar tables define glob-keyed line-count
  budgets.
- `[rust.complexity]`, `[rust.coverage]`, `[rust.deps.forbidden]`, and similar
  sections define language-scoped thresholds and rules.

Configuration uses single-bracket tables and inline maps only.

## Unified Artifact And Flow

End-to-end flow:

1. `ayni install` scaffolds local guidance and configuration, lists required tools and versions, and can install them with `ayni install --apply`.
2. `ayni analyze` executes enabled signal kinds per adapter.
3. Results are merged into one `RunArtifact`.
4. Ayni writes `.ayni/last/signals.json`.
5. Output mode controls report rendering (`stdout` or `md`).

Artifact layout:

```text
.ayni/
├── last/
│   ├── signals.json
│   └── summary.llm.md
└── history/
    └── previous-signals.jsonl
```

`signals.json` is the single source of truth for local runs.

## Product Contract Stability

The signal schema is versioned and treated as a product contract. See
`AYNI_SIGNAL_SCHEMA_VERSION` in `core/src/signal.rs` and the `schema_version`
field in artifacts.

- Core Rust types define canonical semantics.
- Adapters map tool output into that contract without adding ad-hoc fields.
- Reports are derived from the local artifact.

This guarantees consistent behavior for humans and AI consumers as new
languages are added.
