# Ayni

Ayni is an open-source code quality signal tool for repositories that use AI
agents.

It helps teams define a clear quality contract, run language-aware analysis
locally, and turn results into signals that humans and agents can act on.

## Start here

- The clean-slate command tree is active. For an already configured repository,
  run `ayni contract show`, `ayni env show`, `ayni env lock`,
  `ayni agents sync`, and `ayni check --host`. Environment planning, locking,
  building, and managed execution are available for Rust, npm Node, Go modules,
  uv Python projects, and locked Gradle Kotlin builds; unsupported ecosystem
  variants fail explicitly.
- [CLI reference](/cli)
- [Configuration reference](/product/config)
- [Signal contract index](/product/signals)
- [Current signal schema v3](/product/signals/v3)
- [Historical signal schema v2](/product/signals/v2)
- [Historical signal schema v1](/product/signals/v1)
- [Runtime and setup rules](/product/runtime)
- [Impact-aware execution](/product/impact)
- [Rust adapter](/adapters/rust)
- [Go adapter](/adapters/go)
- [Node adapter](/adapters/node)
- [Python adapter](/adapters/python)
- [Kotlin adapter](/adapters/kotlin)
- [Contributing language adapters](/contributing/adapters)

## Project docs

- [README](https://github.com/gdurandvadas/ayni)
- [Architecture](https://github.com/gdurandvadas/ayni/blob/main/ARCHITECTURE.md)
- [Contributing](https://github.com/gdurandvadas/ayni/blob/main/CONTRIBUTING.md)
- [Changelog](https://github.com/gdurandvadas/ayni/blob/main/CHANGELOG.md)

## What you’ll find

- `docs/cli.md` for command usage and flags
- `docs/product/*.md` for the product contract and runtime behavior
- `docs/adapters/*.md` for language-specific adapter guidance
