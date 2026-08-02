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

Without `--language`, bare `ayni install` creates the Rust policy template. For
a non-Rust or polyglot repository, select every intended language explicitly
with repeatable `--language` values before synchronizing guidance or analyzing.

Use focused verification for the inner TDD loop. `analyze` always evaluates the
configured repository and is the only completion gate and writer of
`.ayni/last/signals.json`:

```sh
ayni verify test --language rust --package my-crate --name test_filter
ayni verify test --language node --file apps/web/src/example.test.ts
ayni verify test --language go --package ./internal/api --name TestCreate
ayni verify test --language python --file tests/test_api.py --name test_create
ayni verify test --language kotlin --package com.example.ApiTest --name createsUser
```

Focused evidence is written to `.ayni/verify/last/signals.json` and never
replaces `.ayni/last/signals.json`. `verify` has one subcommand for each of the
six signals; selector support is signal- and adapter-specific. Unsupported,
conflicting, ambiguous, or out-of-scope selectors are rejected before a tool
runs. Re-run the exact `verification.command` supplied with a finding rather
than broadening it by hand.

Inspect the validated configured signal contract without running discovery,
adapters, or analysis:

```sh
ayni contract display
ayni contract display --config path/to/.ayni.toml
```

This human-readable view shows enabled-language roots, all six signal states,
configured thresholds and rules, and explicit tool overrides. It writes no
artifact; use `ayni analyze` for measured results.

Use `install --apply` when you want Ayni to install missing or outdated adapter
tools from local language ecosystems:

```sh
ayni install --apply
```

`install` bootstraps policy, ignores `.ayni/`, and lists or (with `--apply`)
installs catalog tools. It never changes `AGENTS.md`; run `ayni agents sync`
when you intentionally want its marked Ayni section created or refreshed.

To inspect whether an already-configured repository is ready without changing
any files, use check mode:

```sh
ayni install --check
ayni install --check --output json
```

Check mode requires an existing valid `.ayni.toml`. It only detects configured
targets, resolves their execution contexts, and inspects enabled catalog
requirements; it never scaffolds, prepares, installs, validates writable
artifact paths, or writes output files. The human report is the default. JSON
mode emits one deterministic readiness document on stdout and reserves stderr
for command diagnostics. A missing or outdated requirement, undetected target,
or unresolved target produces `not_ready` and a non-zero exit status. `--check`
cannot be combined with `--apply`, and `--output` is check-only.

For a non-Rust or polyglot repository, repeat `--language`; duplicate values
are ignored:

```sh
ayni install --repo-root . --language rust --language node --language python --apply
```

Single-language setup remains valid, for example `ayni install --language go`.

Generate Markdown output:

```sh
ayni analyze --output md
```

Emit the schema-v3 artifact for scripts with either equivalent selector:

```sh
ayni analyze --json
ayni analyze --output json
```

Do not combine `--json` with `--output stdout` or `--output md`; use one JSON
selector instead. JSON is written to stdout and progress to stderr.

Compare two already-produced complete schema-v3 artifacts explicitly. This
command reads only the two supplied files: it does not discover a repository,
consult Git or history, fetch or store artifacts, or write files. Differences
are reported successfully; invalid, incomplete, or incompatible inputs fail
with diagnostics on stderr.

```sh
ayni artifact compare --baseline before.json --candidate after.json
ayni artifact compare --baseline before.json --candidate after.json --output json
```

The JSON form writes exactly one deterministic comparison document to stdout.

For the full CLI reference, see [`docs/cli.md`](docs/cli.md).

## Example Report

This is an illustrative point-in-time snapshot from a validated repository run,
not a promised baseline for every repository or release.

# ayni analyze

**5** / **5** checks passing · aggregate **pass** · schema `0.3.0`

**Completion:** scope `repository` · state **complete** · targets **1** / **1** completed · **1** detected · **0** skipped

The five rows shown here reflect a policy with mutation disabled. This snapshot
recorded 310 passing tests, about 70.99% line coverage, and zero failing
offenders. In schema v3, completion separately reconciles expected, detected,
completed, and skipped targets; an incomplete artifact includes one ordered
issue per skipped target and always aggregates to failure, even if its emitted
rows pass.

## rust (workspace) — 5/5 passing

| # | Signal | Summary | Status |
|---|--------|---------|--------|
| **1** | **test** | `total=310 passed=310 failed=0` | <img src="https://raw.githubusercontent.com/gdurandvadas/ayni/refs/heads/main/assets/pass.svg" alt="pass" width="20" height="20"> pass |
| **2** | **coverage** | `percent=70.99% status=ok` | <img src="https://raw.githubusercontent.com/gdurandvadas/ayni/refs/heads/main/assets/pass.svg" alt="pass" width="20" height="20"> pass |
| **3** | **size** | `max_lines=1456 files=115 fail_count=0` | <img src="https://raw.githubusercontent.com/gdurandvadas/ayni/refs/heads/main/assets/pass.svg" alt="pass" width="20" height="20"> pass |
| **4** | **complexity** | `functions=1983 max_cyclo=14.0 fail_count=0` | <img src="https://raw.githubusercontent.com/gdurandvadas/ayni/refs/heads/main/assets/warn.svg" alt="warn" width="20" height="20"> warn |
| **5** | **deps** | `crates=8 edges=18 violations=0` | <img src="https://raw.githubusercontent.com/gdurandvadas/ayni/refs/heads/main/assets/pass.svg" alt="pass" width="20" height="20"> pass |

<details>
<summary>Offenders</summary>

complexity
- **WARN** `cli/src/completion.rs:33` reconcile cyclo=14.0
- **WARN** `cli/src/install.rs:45` print_install_requirements cyclo=14.0
- **WARN** `core/src/policy.rs:252` normalize_and_validate cyclo=14.0

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
is [schema v3](docs/product/signals/v3.md). Schema v2 remains available only
as a [historical reference](docs/product/signals/v2.md); there is no automatic
conversion or compatibility payload.

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
- [Signal contract index](docs/product/signals.md) ([current v3](docs/product/signals/v3.md), [historical v2](docs/product/signals/v2.md), [historical v1](docs/product/signals/v1.md))
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
