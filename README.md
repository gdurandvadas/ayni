# Ayni

Ayni is a local-first tool that helps AI agents understand and improve code
quality in a repository.

It installs agent-facing repository guidance and analyzes codebases for
structure, boundaries, maintainability, and clarity.

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
```

During development, the same commands can be run through Cargo:

```sh
cargo run -p ayni-cli -- install --repo-root .
cargo run -p ayni-cli -- install --repo-root . --apply
cargo run -p ayni-cli -- analyze --config ./.ayni.toml
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

Analyzes the local repository and prints a quality report. Ayni writes generated
artifacts under `.ayni/`, which should stay out of source control.

Output modes:

- `human`: terminal report
- `llm-md`: markdown summary for AI agents
- `both`: both outputs

## Local-First

The default workflow does not require login, accounts, or remote services. It
reads local files, runs local tooling, and writes local output only.

## Architecture

The workspace is organized around one-way dependencies:

```txt
core <- adapters <- cli
```

- `core/`: policy, language, runtime, registry, and signal domain types
- `adapters/rust/`: Rust signal collectors
- `adapters/go/`: Go signal collectors
- `adapters/node/`: Node.js and TypeScript signal collectors
- `cli/`: command-line orchestration and output
- `.ayni.toml`: repository policy configuration
- `.ayni/`: generated local artifacts

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
