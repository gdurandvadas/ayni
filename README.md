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
curl -fsSL https://raw.githubusercontent.com/gdurandvadas/ayni/main/install.sh | VERSION=ayni-v0.8.1 sh
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

Rust, npm-based Node, Go module, uv Python, and locked Gradle Kotlin
repositories can lock and build a managed OCI environment, warm locked native
dependencies, and run the repository gate offline by default:

```sh
ayni contract show
ayni agents sync
ayni env lock
ayni env build
ayni check
```

Use `check --host` as the explicit escape hatch for repositories whose language
or package manager does not yet support managed preparation.

Use focused verification for the inner TDD loop. `check` evaluates the configured
repository and is the only completion gate and writer of `.ayni/last/signals.json`:

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

Use impact planning for a conservative change-scoped loop against an explicit
Git base:

```sh
ayni impact show --base main
ayni impact run --base main
```

The candidate is the final local working-tree state relative to the explicit
base: commits through `HEAD`, tracked index/worktree changes, and non-ignored
untracked files. Ayni invokes local Git only; it has no GitHub, GitLab,
Bitbucket, pull-request, remote-fetch, or forge-specific integration. Rust and
npm Node workspaces follow reverse dependencies; every other built-in adapter
broadens uncertain work rather than omitting it. Impact results live at
`.ayni/impact/last/impact.json`, explicitly require a final `ayni check`, and
never replace full-check or focused evidence. See
[`docs/product/impact.md`](docs/product/impact.md).

Inspect the validated configured signal contract without running discovery,
adapters, or analysis:

```sh
ayni contract show
ayni contract show --config path/to/.ayni.toml
```

This human-readable view shows enabled-language roots, all six signal states,
configured thresholds and rules, and explicit tool overrides. It writes no
artifact; use `ayni check` for managed measured results or `ayni check --host`
for the explicit host path.

Managed environment setup is owned by the explicit `env` lifecycle:

```sh
ayni env show
ayni env lock
ayni env doctor
ayni env build
ayni env run -- cargo test
```

Locking may query `mise` for exact adapter-owned runtime and tool versions and
Docker Buildx for the published base-image digest.
`--base <reference>@sha256:<digest>` supplies an explicit base instead. If the
published base is unavailable, run `scripts/build-local-environment-image.sh`;
it compiles Ayni inside a Linux container and prints the exact local
`env lock --base` command. Project tools must already be represented in native
npm, uv, or Gradle inputs. Doctor, build, shell, run, and managed check consume
the validated lock without creating or refreshing it. Multi-target shell/run
requests use `--language` and `--root`.

Environment builds stage only digest-verified Cargo/npm/Go/uv/Gradle inputs,
warm provider caches, and retain only declared dependency outputs. Managed
quality commands materialize seeded npm dependencies and fresh uv environments
below `.ayni/environment/`, run without network access, and mount repository
source read-only with only generated `.ayni/` state writable. Interactive
`env shell` and arbitrary `env run` intentionally mount the checkout read-write
so humans and agents can edit it. Managed check and focused verification support
locked Rust, npm Node, Go module, uv Python, and Gradle Kotlin targets. pnpm,
Yarn, Bun, non-uv Python managers, and unsupported Gradle build shapes remain
explicit failures.

Run `ayni agents sync` explicitly when you want the managed guidance block
created or refreshed.

Generate Markdown output:

```sh
ayni check --output markdown
```

Emit the schema-v3 artifact for scripts:

```sh
ayni check --output json
``` JSON is written to stdout and progress to stderr.

Compare two already-produced complete schema-v3 artifacts explicitly. This
command reads only the two supplied files: it does not discover a repository,
consult Git or history, fetch or store artifacts, or write files. Differences
are reported successfully; invalid, incomplete, or incompatible inputs fail
with diagnostics on stderr.

```sh
ayni results compare --baseline before.json --candidate after.json
ayni results compare --baseline before.json --candidate after.json --output json
```

The JSON form writes exactly one deterministic comparison document to stdout.

For the full CLI reference, see [`docs/cli.md`](docs/cli.md).

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
core  <-  adapters/common  <-  adapters/<language>  <-  cli
core  <-  adapters/common  <-  environment          <-  cli
```

Language adapters own ecosystem-specific discovery, tool invocation, impact
mapping, and normalization. Core owns typed product contracts. The environment
backend consumes validated locks and preparation plans without interpreting
language manifests. The CLI owns orchestration and output.

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
