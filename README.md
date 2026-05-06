# Ayni

Ayni is a local-first tool that helps AI agents understand and improve code
quality in a repository.

It installs agent-facing repository guidance and analyzes codebases for
structure, boundaries, maintainability, and clarity.

**Project status:** Ayni is in early development. Expect APIs,
behavior, and workflows to change quickly; pin versions or vendor carefully if
you depend on it today.

## Why

AI agents need clear signals about how a codebase is organized, what quality
means in that project, and which boundaries should be preserved.

Ayni helps by:

- adding or updating `AGENTS.md`
- analyzing repository structure
- collecting language-specific quality signals
- identifying unclear boundaries and maintainability risks
- producing local reports for humans and agents

## Install

From this repository:

```sh
cargo install --path cli
```

Or run directly during development:

```sh
cargo run -p ayni-cli -- --help
```

## Usage

```sh
ayni install
ayni install --apply
ayni analyze
ayni analyze --output md
ayni version
```

During development, the same commands can be run through Cargo:

```sh
cargo run -p ayni-cli -- install --repo-root .
cargo run -p ayni-cli -- install --repo-root . --apply
cargo run -p ayni-cli -- analyze --config ./.ayni.toml
cargo run -p ayni-cli -- analyze --config ./.ayni.toml --output md
```

## Commands

### `install`

Updates local scaffolding and prints the external tools each adapter expects
(names, version checks, and current status: ok / outdated / missing).

- **`ayni install`**: writes `.ayni.toml` when missing, ensures `.gitignore`
  mentions `.ayni/`, updates the managed block in `AGENTS.md`, then **lists**
  required tooling for enabled languages and roots.
- **`ayni install --apply`**: same scaffolding, then **installs** missing or
  outdated tools via `cargo`, `rustup`, `go install`, `npm` / `pnpm` / `yarn` /
  `bun`, etc. (may download from registries).

### `analyze`

Analyzes the local repository and prints a quality report. Scope can be narrowed
with `--file`, `--package`, and `--language`.

Output modes:

- `stdout`: colored terminal report, the default
- `md`: markdown tables written to `.ayni/last/summary.llm.md` and printed to stdout

Ayni always writes `.ayni/last/signals.json` and
`.ayni/history/previous-signals.jsonl` for local delta comparison. Generated
artifacts under `.ayni/` should stay out of source control.

### `version`

Prints the Ayni CLI version.

## Local-First

The default `install` and `analyze` workflow does not require login, accounts,
or remote services. It reads local files, runs local tooling, and writes local
output only. `ayni install --apply` may download tools from language package
registries.

## Architecture

The workspace is organized around one-way dependencies:

```txt
core <- adapters <- cli
```

- `core/`: policy, language, runtime, adapter contracts, and signal domain types
- `adapters/rust/`: Rust signal collectors
- `adapters/go/`: Go signal collectors
- `adapters/node/`: Node.js and TypeScript signal collectors
- `cli/`: command-line orchestration and output
- `.ayni.toml`: repository policy configuration
- `.ayni/`: generated local artifacts

## Artifacts

```txt
.ayni/
├── last/
│   ├── signals.json
│   └── summary.llm.md
└── history/
    └── previous-signals.jsonl
```

`signals.json` is the typed run artifact. `summary.llm.md` is produced by
`ayni analyze --output md`. `previous-signals.jsonl` stores the previous local
run snapshot used for deltas.

## Development

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
cargo check --workspace --all-features
```

Run Ayni against this repository:

```sh
make ayni
```

## Contributing

See `CONTRIBUTING.md`.
