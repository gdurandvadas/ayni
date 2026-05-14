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

- adds or updates agent-facing guidance in `AGENTS.md`
- defines the repository-agent quality contract in `.ayni.toml`
- collects `test`, `coverage`, `size`, `complexity`, `deps`, and `mutation` signals
- runs language-specific tooling locally through adapters
- writes machine-readable artifacts under `.ayni/`
- prints terminal or Markdown reports for local workflows and AI repair loops

## Quick Start

From this repository:

```sh
cargo install --path cli
ayni install
ayni analyze
```

Use `install --apply` when you want Ayni to install missing or outdated adapter
tools from local language ecosystems:

```sh
ayni install --apply
```

Generate Markdown output:

```sh
ayni analyze --output md
```

For the full CLI reference, see [`docs/cli.md`](docs/cli.md).

## Example Report

<!-- ayni:md branch=feat/kotlin -->
# ayni analyze

**5** / **5** checks passing · schema `0.1.0`

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

For the field-level signal contract, see
[`docs/product/signals.md`](docs/product/signals.md).

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
- [Signal contract](docs/product/signals.md)
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
