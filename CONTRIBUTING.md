# Contributing

Thanks for helping improve Ayni.

## Scope

The open-source version currently supports:

- `install` (list required tools; `install --apply` runs catalog installers)
- `analyze`
- `agents sync` (explicitly create or refresh Ayni's marked `AGENTS.md` section)

Out of scope:

- Managed service workflows
- Managed product features
- External run storage

## Development

Run the standard checks from the repository root:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
cargo check --workspace --all-features
```

Run local analysis:

```sh
cargo run -p ayni-cli -- analyze --config ./.ayni.toml
```

## Documentation

The docs site lives under `docs/` and uses the root npm scripts:

```sh
npm install
npm run docs:dev
npm run docs:build
npm run docs:preview
```

Use `npm ci` instead of `npm install` when you want a clean, lockfile-driven install.

Regenerate the CLI reference after changing commands or flags:

```sh
cargo doc-cli > docs/cli.md
```

Release docs are built from the released code when release-please publishes a GitHub Release, and the same workflow can also be triggered manually with `workflow_dispatch`. The docs workflow regenerates `docs/cli.md`, builds VitePress, and publishes only the static site output to the `documentation` branch for GitHub Pages.

The `documentation` branch is generated deployment output only; the source of truth for docs remains the normal source branch under `docs/`.

## Architecture

- CLI handles arguments, orchestration, and local output.
- Core owns analysis policy, signal types, and adapter contracts.
- Adapters own language-specific local tool execution.
- Default analysis runs from the repository checkout and writes local artifacts.
- No reverse dependencies are allowed: `core` <- `adapters` <- `cli`.

## Pull Request Checklist

- Tests added or updated when behavior changes.
- No managed service dependency introduced.
- Local artifact behavior preserved.
- README or docs updated if behavior changed.
- `install` does not modify `AGENTS.md`; `ayni agents sync` is idempotent and preserves user content outside Ayni's marked block.
- `cargo fmt`, `cargo clippy`, `cargo test`, and `cargo check` pass.

## Licensing

By contributing to Ayni, you agree that your contribution is licensed under
the same license as the project: GNU Affero General Public License, version 3
only (`AGPL-3.0-only`).
