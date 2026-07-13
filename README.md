# Ayni

Ayni is an open-source code quality signal tool for repositories that use AI
agents.

It installs agent-facing repository guidance, runs language-specific analysis
locally, and normalizes the results into one report that humans and AI agents
can act on.

## Why

AI agents increase delivery speed, but they also increase change volume beyond
what humans can reliably review line by line.

Ayni shifts quality control from "inspect every diff" to "measure local behavior
and structural health". Repositories define explicit expectations in
`.ayni.toml`; agents respond to those expectations with measurable repairs.

Ayni does not generate code or replace your test and analysis tools. It
orchestrates them, normalizes their output, and turns failures into structured
repair targets.

## What Ayni Does

- can create or update its marked agent-facing guidance in `AGENTS.md` with an explicit command
- defines the repository-agent quality contract in `.ayni.toml`
- collects `test`, `coverage`, `size`, `complexity`, `deps`, and `mutation` signals
- runs language-specific tooling locally through adapters
- writes machine-readable artifacts under `.ayni/`
- prints terminal or Markdown reports for local workflows and AI repair loops

## Install

### macOS and Linux

Install the latest published release:

```sh
curl -fsSL https://raw.githubusercontent.com/gdurandvadas/ayni/main/install.sh | sh
```

The installer detects the current platform, installs `ayni` into
`~/.local/bin` by default, verifies checksums when possible, and can
optionally help add the install directory to `PATH` in interactive shells.

Pin a specific release:

```sh
curl -fsSL https://raw.githubusercontent.com/gdurandvadas/ayni/main/install.sh | VERSION=v0.1.2 sh
```

Choose a custom install directory:

```sh
curl -fsSL https://raw.githubusercontent.com/gdurandvadas/ayni/main/install.sh | INSTALL_DIR="$HOME/bin" sh
```

### From source

Build and install directly from this repository:

```sh
cargo install --path cli
```

## Quick Start

```sh
ayni install
ayni agents sync
ayni analyze
```

Use `install --apply` when you want Ayni to install missing or outdated adapter
tools from local language ecosystems:

```sh
ayni install --apply
```

`install` bootstraps policy, ignores `.ayni/`, and lists or (with `--apply`)
installs catalog tools. It never changes `AGENTS.md`; run `ayni agents sync`
when you intentionally want its marked Ayni section created or refreshed.

For a polyglot repository, repeat `--language`; duplicate values are ignored:

```sh
ayni install --repo-root . --language rust --language node --language python --apply
```

Single-language setup remains valid, for example `ayni install --language go`.

Generate Markdown output:

```sh
ayni analyze --output md
```

Emit the schema-v2 artifact for scripts with either equivalent selector:

```sh
ayni analyze --json
ayni analyze --output json
```

Do not combine `--json` with `--output stdout` or `--output md`; use one JSON
selector instead. JSON is written to stdout and progress to stderr.

For the full CLI reference, see [`docs/cli.md`](docs/cli.md).

## Example Report

<!-- ayni:md branch=feat/kotlin -->
# ayni analyze

**5** / **5** checks passing · schema `0.2.0`

## rust (workspace) — 5/5 passing

| # | Signal | Summary | Status |
|---|--------|---------|--------|
| **1** | **test** | `total=112 passed=112 failed=0` | <img src="https://raw.githubusercontent.com/gdurandvadas/ayni/refs/heads/main/assets/pass.svg" alt="pass" width="20" height="20"> pass |
| **2** | **coverage** | `percent=46.0% status=ok` | <img src="https://raw.githubusercontent.com/gdurandvadas/ayni/refs/heads/main/assets/pass.svg" alt="pass" width="20" height="20"> pass |
| **3** | **size** | `max_lines=979 files=89 fail_count=0` | <img src="https://raw.githubusercontent.com/gdurandvadas/ayni/refs/heads/main/assets/pass.svg" alt="pass" width="20" height="20"> pass |
| **4** | **complexity** | `functions=1353 max_cyclo=15.0 fail_count=0` | <img src="https://raw.githubusercontent.com/gdurandvadas/ayni/refs/heads/main/assets/warn.svg" alt="warn" width="20" height="20"> warn |
| **5** | **deps** | `crates=7 edges=11 violations=0` | <img src="https://raw.githubusercontent.com/gdurandvadas/ayni/refs/heads/main/assets/pass.svg" alt="pass" width="20" height="20"> pass |

<details>
<summary>Offenders</summary>

complexity
- **WARN** `adapters/rust/src/collectors/deps.rs:109` analyze_deps cyclo=15.0
- **WARN** `adapters/rust/src/tools/signals.rs:560` run_mutation_check cyclo=15.0
- **WARN** `adapters/go/src/collectors/test.rs:23` collect cyclo=14.0
- **WARN** `adapters/rust/src/collectors/size.rs:12` collect cyclo=14.0
- **WARN** `core/src/catalog/python_resolution.rs:8` resolve_python_package_manager cyclo=14.0
- **WARN** `adapters/go/src/collectors/complexity.rs:10` collect cyclo=13.0
- **WARN** `adapters/go/src/collectors/deps.rs:29` collect cyclo=13.0
- **WARN** `adapters/kotlin/src/collectors/test.rs:90` parse_junit_xml cyclo=13.0
- **WARN** `adapters/node/src/collectors/complexity.rs:10` collect cyclo=13.0
- **WARN** `adapters/python/src/collectors/mutation.rs:161` parse_junit_xml cyclo=13.0
- **WARN** `cli/src/delta.rs:106` signal_result_metrics cyclo=13.0
- **WARN** `cli/src/ui/md_report.rs:148` offender_lines cyclo=13.0
- **WARN** `cli/src/ui/report.rs:537` row_status cyclo=13.0
- **WARN** `cli/src/ui/runner.rs:151` run_internal cyclo=13.0
- **WARN** `adapters/python/src/collectors/coverage.rs:33` collect cyclo=12.0
- **WARN** `adapters/rust/src/collectors/complexity.rs:9` collect cyclo=12.0
- **WARN** `adapters/rust/src/collectors/coverage.rs:192` collect_coverage_percents cyclo=12.0
- **WARN** `cli/src/install.rs:40` print_install_requirements cyclo=12.0
- **WARN** `cli/src/install.rs:136` installer_summary cyclo=12.0
- **WARN** `cli/src/main.rs:242` collect_targets_with_ui cyclo=12.0
- **WARN** `cli/src/ui/report.rs:573` stylize cyclo=12.0
- **WARN** `adapters/kotlin/src/collectors/complexity.rs:14` collect cyclo=11.0
- **WARN** `adapters/node/src/collectors/deps.rs:13` collect cyclo=11.0
- **WARN** `adapters/node/src/discovery.rs:9` discover_project_roots cyclo=11.0
- **WARN** `adapters/node/src/discovery.rs:155` discover_file_parent_roots cyclo=11.0
- **WARN** `adapters/python/src/collectors/complexity.rs:13` collect cyclo=11.0
- **WARN** `adapters/python/src/collectors/deps.rs:14` collect cyclo=11.0
- **WARN** `adapters/python/src/discovery.rs:117` discover_file_parent_roots cyclo=11.0
- **WARN** `adapters/rust/src/discovery.rs:45` discover_file_parent_roots cyclo=11.0
- **WARN** `cli/src/install.rs:385` validate_install_foundation cyclo=11.0
- **WARN** `cli/src/install.rs:538` update_foundation_settings cyclo=11.0
- **WARN** `cli/src/main.rs:488` signal_metrics cyclo=11.0
- **WARN** `cli/src/ui/report.rs:150` summarize cyclo=11.0
- **WARN** `core/src/catalog.rs:382` status_in cyclo=11.0
- **WARN** `core/src/catalog.rs:436` install_with cyclo=11.0
- **WARN** `core/src/policy.rs:192` normalize_and_validate cyclo=11.0
- **WARN** `core/src/policy.rs:280` normalize_root_entry cyclo=11.0

</details>

Markdown always groups typed findings in **Offenders**. It adds **Failures** only
when a collector command failed; each failure includes its category,
classification, command, working directory, exit code when available, and message. Reports and
JSON artifacts can therefore expose repository paths and raw tool output; treat
them as repository diagnostics when sharing them.

## Signals

Ayni emits a closed signal vocabulary shared across language adapters.

| Signal | Purpose |
| --- | --- |
| `test` | Test execution health and failures |
| `coverage` | Coverage depth and weak or uncovered areas |
| `size` | Module and file growth against line-count budgets |
| `complexity` | Function-level complexity against thresholds |
| `deps` | Forbidden architectural dependency edges |
| `mutation` | Test effectiveness against simulated behavioral change |

For the canonical vocabulary and version selection, see
[`docs/product/signals.md`](docs/product/signals.md); the current JSON envelope
is [schema v2](docs/product/signals/v2.md).

## Configuration

`.ayni.toml` is the handoff point between humans and agents: which languages and
roots are in scope, which signals run, and which limits define healthy code.

```toml
[checks]
test = true
coverage = true
size = true
complexity = true
deps = true
mutation = false

[languages]
enabled = ["rust"]

[rust.size]
"*.rs" = { warn = 400, fail = 700, exclude = ["target/**", ".ayni/**"] }

[rust.complexity]
fn_cyclomatic = { warn = 10, fail = 20 }
```

For the full configuration reference, see
[`docs/product/config.md`](docs/product/config.md).

Size and complexity are maximums: at `warn` or higher they warn, and at `fail`
or higher they fail. Coverage is a minimum: below `warn` it warns and below
`fail` it fails. Thus `{ warn = 400, fail = 700 }` warns at 400 lines and fails
at 700, while `{ warn = 80, fail = 70 }` warns below 80% coverage and fails
below 70%. Warnings remain visible but do not make a row fail.

## How It Fits Together

The platform keeps product semantics, language integrations, and CLI behavior
separate:

```text
core  <-  adapters  <-  cli
```

Adapters own language-specific tool invocation and normalization. Core owns the
typed product contract. The CLI owns local orchestration and output.

For layer boundaries and change rules, see [ARCHITECTURE.md](ARCHITECTURE.md).

## Documentation

- [CLI reference](docs/cli.md)
- [Configuration reference](docs/product/config.md)
- [Signal contract index](docs/product/signals.md) ([current v2](docs/product/signals/v2.md), [historical v1](docs/product/signals/v1.md))
- [Runtime and setup rules](docs/product/runtime.md)
- [Architecture](ARCHITECTURE.md)
- Language adapters:
  [Rust](docs/adapters/rust.md),
  [Go](docs/adapters/go.md),
  [Node](docs/adapters/node.md),
  [Python](docs/adapters/python.md),
  [Kotlin](docs/adapters/kotlin.md)

## Contributing

Developer workflow, architecture constraints, and repository checks live in
[`CONTRIBUTING.md`](CONTRIBUTING.md).

## License

Copyright (C) 2026 Gastón Durand Vadas.

Ayni is licensed under the GNU Affero General Public License, version 3 only
(`AGPL-3.0-only`). See [`LICENSE`](LICENSE) for the full license text and
[`NOTICE`](NOTICE) for the repository copyright notice.
