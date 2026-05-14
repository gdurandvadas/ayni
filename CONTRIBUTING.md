# Contributing

Thanks for helping improve Ayni.

## Scope

The open-source version currently supports:

- `install` (list required tools; `install --apply` runs catalog installers)
- `analyze`

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

Regenerate CLI docs after changing commands or flags:

```sh
cargo doc-cli > docs/cli.md
```

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
- `AGENTS.md` install behavior remains deterministic and preserves user content.
- `cargo fmt`, `cargo clippy`, `cargo test`, and `cargo check` pass.

## Licensing

By contributing to Ayni, you agree that your contribution is licensed under
the same license as the project: GNU Affero General Public License, version 3
only (`AGPL-3.0-only`).
