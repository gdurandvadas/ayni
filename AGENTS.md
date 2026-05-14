# Ayni — Agent Rules

Ayni is an open-source code-quality signal tool for AI agents.

## Documentation

Read these before implementing anything. They are the source of truth for
decisions that are not visible in the code.

- `ARCHITECTURE.md` — layer boundaries, dependency rules, and change decision guide
- `README.md` — product framing, AI feedback loop, and high-level architecture
- `docs/product/config.md` — `.ayni.toml` reference
- `docs/product/signals.md` — canonical signal vocabulary and field-level contract
- `docs/adapters/rust.md` — Rust adapter module layout and collector mapping
- `docs/adapters/node.md` — Node adapter toolchain, lockfile manager resolution, and collector mapping
- `docs/adapters/go.md` — Go adapter collectors, tool catalog, and policy mapping
- `docs/adapters/python.md` — Python adapter package managers, collectors, and policy mapping
- `docs/adapters/template.md` — how to build a new language adapter
- `docs/cli.md` — CLI reference; regenerate after CLI changes

After adding or modifying any CLI command or flag, regenerate with:

```sh
cargo doc-cli > docs/cli.md
```

## Invariants

- Keep one-way dependency flow: `core` <- `adapters` <- `cli`.
- Keep language-specific detection, root discovery, package-manager resolution,
  tool catalogs, and collector behavior inside the owning language adapter.
  The CLI may orchestrate adapters but must not hard-code language-specific
  root markers, lockfiles, package managers, or tool behavior.
- Keep `install` and `analyze` runnable from the repository checkout with local
  artifacts.
- Keep the repository-agent quality contract in `.ayni.toml` at repo root.
- Keep `.ayni/` generated artifacts out of source control.
- Keep workspace checks runnable from repository root.
- Keep open-source licensing metadata consistent: `LICENSE`, `NOTICE`, README,
  contribution guidance, Cargo package metadata, and release archives must all
  agree on `AGPL-3.0-only`.

## Before Editing

- Confirm target crate boundaries and dependency direction.
- Prefer scoped checks with `--file`, `--package`, and `--language` where supported.
- Avoid adding network dependencies unless explicitly required and documented.
- If changing legal, packaging, or release files, check whether `LICENSE`,
  `NOTICE`, README, `CONTRIBUTING.md`, `Cargo.toml`, and release artifacts need
  matching updates.

## After Editing

- Run `cargo fmt --all -- --check`.
- Run `cargo clippy --workspace --all-targets --all-features -- -D warnings`.
- Run `cargo test --workspace --all-features`.
- Run `cargo check --workspace --all-features`.
- If policy behavior changed, run `cargo run -p ayni-cli -- analyze --config ./.ayni.toml`.

## Quality Command Index

- classic: formatting, linting, tests, and compile check as listed above
- install (list tools): `cargo run -p ayni-cli -- install --repo-root .`
- install (apply tooling): `cargo run -p ayni-cli -- install --repo-root . --apply`
- analyze: `cargo run -p ayni-cli -- analyze --config ./.ayni.toml`
- full: run classic gates, then analyze

## Ayni (Rust)

- `cargo run -p ayni-cli -- install --repo-root . --language rust` scaffolds
  `.ayni.toml`, ensures `.gitignore` contains `.ayni/`, updates the Ayni-managed
  `AGENTS.md` block, and lists Rust adapter tools; add `--apply` to install them.
- `cargo test -p <pkg>` runs package-scoped tests.
- `cargo run -p ayni-cli -- analyze --config ./.ayni.toml --package <pkg>`
  runs scoped analysis.
- Artifact output: `.ayni/last/signals.json`.

## Example Workspaces

- Use `install` bootstrap checks only on `examples/<language>/single`; monorepo
  examples already include `.ayni.toml`.
- Example install command:
  `cargo run -p ayni-cli -- install --repo-root examples/go/single --language go --apply`
- Remove installed single-fixture files with:
  `rm -rf examples/go/single/.ayni.toml examples/go/single/.gitignore examples/go/single/AGENTS.md`

<!-- AYNI:BEGIN -->
## Code quality guidance for AI agents

When modifying this repository:

- Preserve clear module boundaries.
- Prefer small, testable units.
- Keep CLI, core logic, command execution, and reporting separate.
- Avoid adding network dependencies unless explicitly required.
- Update tests when behavior changes.

Run:

```sh
ayni analyze
```
<!-- AYNI:END -->
