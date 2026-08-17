# Quickstart

This guide takes a repository from an explicit quality contract to its first reproducible Ayni check.

## 1. Install Ayni

Follow [Installation](/getting-started/installation), then confirm the CLI is available:

```sh
ayni --version
```

Managed execution also needs Docker with Buildx, and every `ayni env lock` requires Mise. See the [managed-environment prerequisites](/getting-started/installation#managed-environment-prerequisites).

## 2. Define what healthy means

Ayni reads the quality contract from `.ayni.toml`. It does not infer or silently create policy.

For an initial Rust repository that only enables tests:

```toml
[checks]
test = true
coverage = false
size = false
complexity = false
deps = false
mutation = false

[languages]
enabled = ["rust"]

[rust]
roots = ["."]
```

Enable additional signals and thresholds as the contract matures. Policy examples are available for [Rust](https://github.com/gdurandvadas/ayni/tree/main/examples/rust/mono), [Node](https://github.com/gdurandvadas/ayni/tree/main/examples/node/mono), [Go](https://github.com/gdurandvadas/ayni/tree/main/examples/go/mono), [Python](https://github.com/gdurandvadas/ayni/tree/main/examples/python/mono), and [Kotlin](https://github.com/gdurandvadas/ayni/tree/main/examples/kotlin/mono). These demonstrate signal configuration; a repository must also satisfy its adapter's native runtime and dependency metadata requirements before `env lock` can succeed.

Validate the file and inspect the effective policy:

```sh
ayni contract validate
ayni contract show
```

See [Configuration](/product/config) for every contract field and [Signals](/product/signals) for the available measurements.

## 3. Make runtime inputs explicit

The language adapter derives an environment plan from the enabled signals and native project metadata. Keep exact tool and dependency inputs in the repository—for example `rust-toolchain.toml` and `Cargo.lock` for Rust, or `packageManager` and a native Node lockfile for Node.

Inspect the plan before creating anything:

```sh
ayni env show
```

If the plan is incomplete, the command explains which project metadata is missing or unsupported. Per-language requirements are documented in the [Rust](/adapters/rust), [Node](/adapters/node), [Go](/adapters/go), [Python](/adapters/python), and [Kotlin](/adapters/kotlin) adapter guides.

## 4. Lock and build the managed environment

Resolve exact tool versions and the immutable Ayni base image:

```sh
ayni env lock
```

Review and commit the resulting `.ayni.lock`. The lock is generated state, but it is part of the repository's reproducibility boundary.

Build the OCI image and verify it is ready:

```sh
ayni env build
ayni env doctor
```

Ayni deliberately does not create or refresh the lock and image when a quality command starts. This keeps environment changes visible and reviewable.

::: tip Version-control boundary
Commit `.ayni.toml`, `.ayni.lock`, and the native dependency/tool locks. Ignore `.ayni/`, which contains local evidence and materialized runtime state.
:::

## 5. Run the quality contract

```sh
ayni check
```

`ayni check` automatically launches the locked managed environment and evaluates every enabled signal for every configured language root. You do **not** need to wrap it in `ayni env run`.

The result uses three statuses:

- **pass** — the measurement is within policy;
- **warn** — the measurement crossed a warning threshold but not a failure threshold; and
- **fail** — the measurement or tool execution failed the contract.

The full repository artifact is written to `.ayni/last/signals.json`. For the same evidence on standard output:

```sh
ayni check --output json
ayni check --output markdown
```

## 6. Use focused feedback while editing

Run one signal instead of the complete contract:

```sh
ayni verify test
ayni verify coverage
ayni verify complexity
```

For a specific language root in a polyglot or multi-root repository:

```sh
ayni verify test --language node --root apps/web
```

Run only the quality work affected by a change:

```sh
ayni impact run --base origin/main
```

Managed execution is also the default for `verify` and `impact run`.

## 7. Share the workflow with coding agents

Create or refresh Ayni's managed guidance block in the repository's `AGENTS.md`:

```sh
ayni agents sync
```

This is an explicit operation; quality commands do not modify agent instructions. Review and commit the resulting guidance with the repository.

## 8. Run development commands when needed

`env run` and `env shell` are for commands outside Ayni's quality interface:

```sh
ayni env run -- cargo test parser::tests
ayni env shell
```

When more than one target is locked, select one explicitly:

```sh
ayni env run --language node --root apps/web -- npm test
```

Use `--host` only as an explicit escape hatch when the repository cannot yet use managed execution:

```sh
ayni check --host
ayni verify test --host
ayni impact run --host --base origin/main
```

The host path uses your installed tools, so it does not provide the same runtime reproducibility.

## What to read next

- [How Ayni works](/getting-started/how-ayni-works) explains the contract, environment, and execution model.
- [Managed environments](/product/environments) covers locking, image builds, target selection, and runtime behavior.
- [Signals](/product/signals), [Verification](/product/runtime), and [Impact analysis](/product/impact) cover the feedback loop in detail.
- [CLI reference](/cli) lists every command and option.
