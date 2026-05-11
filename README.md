# Ayni

Ayni is an open-source code quality signal tool for repositories that use AI
agents.

Ayni comes from Quechua. `ayni` means reciprocity: mutual support through
exchange. That is the model for this tool. Repositories define explicit quality
expectations, and agents respond to those expectations with local, measurable
repairs.

Ayni installs agent-facing repository guidance and runs language-specific
analysis locally, then normalizes the results into a single report.

## Why

AI agents need explicit local signals about repository boundaries, test health,
coverage, complexity, size, and architectural rules.

AI agents increase delivery speed, but they also increase change volume beyond
what humans can reliably review line by line. Ayni shifts quality control from
"inspect every diff" to "measure behavior and structural health locally".

Ayni treats code quality as a stream of machine-readable signals that can guide
humans and AI agents.

Ayni helps by:

- adding or updating `AGENTS.md`
- defining the repository-agent contract in `.ayni.toml`
- collecting `test`, `coverage`, `size`, `complexity`, `deps`, and `mutation` signals
- producing terminal and Markdown reports
- writing machine-readable local artifacts for repair loops

It does not generate code or replace your test and analysis tools. It
orchestrates and normalizes local signals.

## What Ayni Provides

Ayni is open-source and language-agnostic. It runs language-specific tooling
locally through adapters, then normalizes everything into one typed schema.

It provides:

- a closed signal vocabulary shared across languages
- a policy file describing what healthy code means
- an `AGENTS.md` managed block with repository guidance for AI coding agents
- a unified local artifact that humans and AI agents can consume
- terminal and markdown reports for local workflows

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

For the field-level contract, see [`docs/product/signals.md`](docs/product/signals.md).

## How AI Models Benefit

AI models need structured feedback, not prose-only summaries. Ayni gives
models:

- deterministic targets from explicit budgets
- local file and function context through offender rows
- prioritization through pass/fail status and offender severity
- cross-language consistency through shared signal kinds
- local markdown and JSON artifacts that can feed iterative repair loops

This enables loops such as: generate code -> run Ayni -> read offending rows ->
patch -> rerun.

## Architecture

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

For layer boundaries and change rules, see [ARCHITECTURE.md](ARCHITECTURE.md).

## Configuration Model

`.ayni.toml` is the handoff point between humans and agents. Humans encode the
repository's quality expectations in `.ayni.toml`; agents run Ayni against that
contract and use failing rows as repair targets.

At a glance:

- `[checks]` controls which signal kinds run
- `[languages]` selects enabled languages
- `[rust.size]`, `[node.size]`, `[python.size]`, `[kotlin.size]`, and similar tables define
  glob-keyed line-count budgets
- `[rust.complexity]`, `[rust.coverage]`, `[rust.deps.forbidden]`, and similar
  sections define language-scoped thresholds and rules

Configuration uses single-bracket tables and inline maps only.

For the full reference, see [`docs/product/config.md`](docs/product/config.md).

## Install

From this repository:

```sh
cargo install --path cli
```

Check the CLI:

```sh
ayni --help
```

## Quick Start

```sh
ayni install
ayni install --apply
ayni analyze
ayni analyze --output md
```

What these do:

- `ayni install` scaffolds `.ayni.toml`, updates the managed block in `AGENTS.md`, ensures `.gitignore` includes `.ayni/`, and lists required tools.
- `ayni install --apply` also installs missing or outdated tools from local language ecosystems and validates the resulting setup before succeeding.
- `ayni analyze` prints the stdout report and writes `.ayni/last/signals.json`.
- `ayni analyze --output md` prints Markdown to stdout and writes `.ayni/last/signals.json`.

`.ayni.toml` is the contract between the repository and the agent: which
languages and roots are in scope, which signals run, and which limits define
healthy code.

## Unified Artifact And Flow

End-to-end flow:

1. `ayni install` scaffolds local guidance and configuration, lists required tools and versions, and can install and validate them with `ayni install --apply`.
2. `ayni analyze` executes enabled signal kinds per adapter.
3. Results are merged into one `RunArtifact`.
4. Ayni writes `.ayni/last/signals.json`.
5. Output mode controls report rendering (`stdout` or `md`).

## Commands

### `install`

Updates local scaffolding and reports tool status as `ok`, `outdated`, or
`missing`.

### `analyze`

Analyzes the local repository and prints a quality report. Scope can be narrowed
with `--file`, `--package`, and `--language`.

Output modes:

- `stdout`: colored terminal report, the default
- `md`: markdown report printed to stdout

### `version`

Prints the Ayni CLI version.

## Example Output

## rust (adapters/go) — 5/5 passing

| #     | Signal         | Summary                                    | Status                                                                                                                                 |
| ----- | -------------- | ------------------------------------------ | -------------------------------------------------------------------------------------------------------------------------------------- |
| **1** | **test**       | `total=7 passed=7 failed=0`                | <img src="https://raw.githubusercontent.com/gdurandvadas/ayni/refs/heads/main/assets/pass.svg" alt="pass" width="20" height="20"> pass |
| **2** | **coverage**   | `percent=40.3% status=ok`                  | <img src="https://raw.githubusercontent.com/gdurandvadas/ayni/refs/heads/main/assets/pass.svg" alt="pass" width="20" height="20"> pass |
| **3** | **size**       | `max_lines=235 files=11 fail_count=0`      | <img src="https://raw.githubusercontent.com/gdurandvadas/ayni/refs/heads/main/assets/pass.svg" alt="pass" width="20" height="20"> pass |
| **4** | **complexity** | `functions=90 max_cyclo=14.0 fail_count=0` | <img src="https://raw.githubusercontent.com/gdurandvadas/ayni/refs/heads/main/assets/warn.svg" alt="warn" width="20" height="20"> warn |
| **5** | **deps**       | `crates=1 edges=0 violations=0`            | <img src="https://raw.githubusercontent.com/gdurandvadas/ayni/refs/heads/main/assets/pass.svg" alt="pass" width="20" height="20"> pass |

<details>
<summary>Offenders</summary>

complexity

- **WARN** `adapters/go/src/collectors/test.rs:23` collect cyclo=14.0
- **WARN** `adapters/go/src/collectors/complexity.rs:10` collect cyclo=13.0
- **WARN** `adapters/go/src/collectors/deps.rs:29` collect cyclo=13.0

</details>

## Local Workflow

The default `install` and `analyze` workflow runs from the repository checkout.
It reads project files, runs configured tooling, and writes artifacts under
`.ayni/`. `ayni install --apply` uses adapter catalogs to install missing tools and validate the configured foundation.

## Artifacts

```txt
.ayni/
├── last/
│   └── signals.json
└── history/
    └── previous-signals.jsonl
```

`signals.json` is the typed run artifact. `previous-signals.jsonl` stores the
previous local run snapshot used for deltas.

## Product Contract Stability

The signal schema is versioned and treated as a product contract. Core Rust
types define canonical semantics. Adapters map tool output into that contract
without adding ad-hoc fields. Reports are derived from the local artifact.

## Contributing

Developer workflow, architecture constraints, and repository checks live in
`CONTRIBUTING.md`.
