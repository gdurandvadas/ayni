# Ayni — Agent Rules

Ayni is an open-source code-quality signal tool for AI agents.

## Checkout Dogfooding

- When behavior changes, exercise the checkout binary via `cargo run -p ayni-cli -- …`;
  never rely on a globally installed Ayni.
- Use focused `verify` during implementation.
- Run the full repository contract only at the final milestone gate.

## Pull Requests

- Use PR titles in the format `<change-type>(<scope>): <description>`.

## Release Workflows

- Treat creation of the public GitHub release as a durable boundary. Every
  downstream publication step—binary assets, checksums, OCI images, signatures,
  and released-evidence validation—must be idempotent and manually recoverable
  for an existing release tag.
- Do not assume rerunning a failed workflow will use a later workflow fix;
  GitHub reruns use the original event revision. Keep an explicit
  `workflow_dispatch` recovery path on the default branch.
- Recovery must verify that the release already exists, preserve provenance from
  the tagged source commit, intentionally overwrite same-named assets, and fail
  closed when any architecture, scan, signature, or validation evidence is
  missing.
- When changing release orchestration or upgrading a release action, validate
  both initial publication and existing-tag recovery, including every supported
  architecture.
- Normalize third-party release-action outputs in an explicit shell step before
  exporting job outputs. Never combine skipped-step and action outputs with
  `||` in job outputs: downstream `if` conditions can see an empty value and
  silently skip every publication job.
- A workflow run that creates or recovers a public release must not succeed when
  publication jobs are skipped. Keep an unconditional final completion job that
  independently detects a release for the triggering commit, requires every
  binary/image/validation job to succeed, and verifies the public asset count.
- Keep Release Please lightweight on ordinary `main` pushes. Repository quality
  belongs in the required PR/status workflow; run binary, image, and released-
  evidence matrices only when a release is created or explicitly recovered.
- Preserve the established `ayni-<release-tag>-<target>.tar.gz` archive contract
  across the workflow, installer, checksums, documentation, and tests. Remember
  that the release tag already starts with `ayni-v`.
- If a GitHub App creates the release, mint a fresh contents-write app token in
  the publication job for release updates, obsolete-asset deletion, and uploads;
  do not assume `GITHUB_TOKEN` can mutate a release owned by that integration.
- After publication or recovery, inspect the actual GitHub release assets and run
  the checkout installer against the public release; green build artifacts alone
  are not release evidence.

## Documentation

Read these before implementing anything. They are the source of truth for
decisions that are not visible in the code.

- `ARCHITECTURE.md` — layer boundaries, dependency rules, and change decision guide
- `README.md` — product framing, AI feedback loop, and high-level architecture
- `docs/product/config.md` — `.ayni.toml` reference
- `docs/product/environments.md` — managed environment lifecycle, lock, build, and execution contract
- `docs/product/signals.md` — canonical signal vocabulary, schema selection, and compatibility posture
- `docs/product/capabilities.md` — adapter capability tiers and truthful signal availability
- `docs/product/signals/v4.md` — current `0.4.0` serialized signal-artifact contract
- `docs/product/signals/v3.md` — historical `0.3.0` serialized signal-artifact reference only
- `docs/product/signals/v2.md` — historical `0.2.0` serialized signal-artifact reference only
- `docs/product/signals/v1.md` — historical `0.1.0` serialized signal-artifact reference only
- `docs/adapters/rust.md` — Rust adapter installation, signal coverage, and policy contract
- `docs/adapters/node.md` — Node adapter package-manager resolution, signal coverage, and policy contract
- `docs/adapters/go.md` — Go adapter installation, signal coverage, and policy contract
- `docs/adapters/python.md` — Python adapter package-manager resolution, signal coverage, and policy contract
- `docs/adapters/kotlin.md` — Kotlin Gradle adapter installation, signal coverage, and policy contract
- `docs/contributing/adapters.md` — how to build a new language adapter
- `docs/cli.md` — CLI reference; regenerate after CLI changes

After adding or modifying any CLI command or flag, regenerate with:

```sh
cargo doc-cli > docs/cli.md
```

## Invariants

- Keep one-way dependency flow: `core` <- `adapters/common` <- `adapters/<lang>` <- `cli`, with the parallel backend path `core` <- `adapters/common` <- `environment` <- `cli`.
- Keep language-specific detection, root discovery, package-manager resolution,
  tool catalogs, and collector behavior inside the owning language adapter.
  The CLI may orchestrate adapters but must not hard-code language-specific
  root markers, lockfiles, package managers, or tool behavior.
- Keep `env` lifecycle commands and `check --host` runnable from the repository
  checkout with local artifacts.
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
- If policy behavior changed, run `cargo run -p ayni-cli -- check --host --config ./.ayni.toml`.

## Quality Command Index

- classic: formatting, linting, tests, and compile check as listed above
- contract: `cargo run -p ayni-cli -- contract show --config ./.ayni.toml`
- check: `cargo run -p ayni-cli -- check --host --config ./.ayni.toml`
- full: run classic gates, then check

## Ayni (Rust)

- `env show`, multi-language `env lock`, and the lock-driven OCI `env doctor`,
  `env build`, `env shell`, and `env run` lifecycle are implemented for Rust,
  npm/pnpm Node, Go modules, uv Python, and locked Gradle Kotlin. Do not substitute
  removed `install` behavior.
- `cargo run -p ayni-cli -- agents sync --repo-root .` is the only command that
  creates or refreshes the Ayni-managed `AGENTS.md` block.
- `cargo test -p <pkg>` runs package-scoped tests.
- `cargo run -p ayni-cli -- check --host --config ./.ayni.toml`
  runs repository completion analysis for this checkout, which does not commit an environment lock.
- Artifact output: `.ayni/last/signals.json`.

<!-- AYNI:BEGIN -->
## Code quality guidance for AI agents

Treat `.ayni.toml` as the authoritative repository quality policy. Run
`ayni contract show` for its validated effective summary, and inspect the
policy itself before proposing a policy change.

During an edit, use the narrowest supported `ayni verify <signal>`:

```sh
ayni verify <signal> [selectors]
```

Use `ayni verify list` to list exact commands from the last repository artifact,
then rerun the exact verification command supplied by a finding. For a change-scoped
loop, run `ayni impact show --base <revision>` and then `ayni impact run`,
copying the same explicit base. Impact success is not repository completion;
run one unscoped `ayni check` at the caller's completion boundary.

Run quality commands directly; never wrap them in `ayni env run`. Treat `env
shell` and `env run` as intentional advanced access because they mount the
checkout read-write and do not produce normalized quality evidence.

Treat incomplete or missing required artifacts as failure, and never loosen
`.ayni.toml` merely to silence a finding.

Use the full repository analysis as the completion gate:

```sh
ayni check
```

A non-zero exit code means the quality contract was not satisfied: a signal
may have failed, expected work may be incomplete, or setup may be invalid.
Read `.ayni/last/signals.json` when present for typed completion and target
accounting. For each finding, rerun its exact verification command and repair
the listed offenders.
<!-- AYNI:END -->
