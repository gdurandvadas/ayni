# Quickstart

This guide takes a supported repository from a reviewable policy proposal to its first reproducible Ayni check.

## What the first proof looks like

A complete run produces both a human summary and `.ayni/last/signals.json`. The exact rows depend on the repository, but completion is explicit:

```text
Completion: repository / complete (1 of 1 targets)
rust:workspace test pass
Result: pass
```

A failed signal can still belong to a **complete** run. Missing expected work instead produces `completion.state = "incomplete"` and fails closed.

## 1. Install Ayni

Follow [Installation](/getting-started/installation), then confirm the CLI is available:

```sh
ayni --version
```

Managed execution also needs Docker with Buildx and Mise. See the [managed-environment prerequisites](/getting-started/installation#managed-environment-prerequisites).

## 2. Preview a minimal policy

Ayni can discover supported project roots and propose a minimal, test-only contract without writing anything:

```sh
ayni init --dry-run
```

Review the complete TOML printed to standard output. The proposal enables tests only; it does not guess thresholds or silently enable deeper signals. Create the file explicitly when it is correct:

```sh
ayni init --write
```

`--write` refuses to overwrite an existing `.ayni.toml`. Re-run `--dry-run` whenever you want a fresh proposal to compare by hand.

If automatic project discovery is not appropriate, author `.ayni.toml` directly. For example:

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

Inspect and validate the effective policy with the single contract command:

```sh
ayni contract show
```

See [Configuration](/product/config) for every field and [adapter capability tiers](/product/capabilities) before enabling more signals.

## 3. Share the workflow with coding agents

Create or refresh only Ayni's managed block in `AGENTS.md`:

```sh
ayni agents sync
```

Review and commit that guidance with the policy. Quality commands never modify agent instructions implicitly.

## 4. Inspect runtime requirements

The owning language adapters derive an environment plan from the configured roots, enabled signals, and native project metadata:

```sh
ayni env show
```

The plan explains missing or unsupported runtime inputs before anything is locked. Keep native declarations and dependency locks in the repository—for example `rust-toolchain.toml` and `Cargo.lock`, or Node's `packageManager` and native lockfile.

## 5. Lock and build the managed environment

```sh
ayni env lock
ayni env build
ayni env doctor
```

Review and commit `.ayni.lock`. Ayni deliberately does not create or refresh the lock or image when a quality command starts, so changes to the evidence environment remain visible.

::: tip Version-control boundary
Commit `.ayni.toml`, `.ayni.lock`, and native dependency/tool locks. Ignore `.ayni/`, which contains local evidence and materialized runtime state.
:::

## 6. Run the reproducible repository gate

```sh
ayni check
```

`check` launches the locked managed environment automatically and evaluates every enabled, supported signal for every configured root. Do not wrap it in `ayni env run`.

The full artifact is written to `.ayni/last/signals.json`. To render the same evidence in another format:

```sh
ayni check --output json
ayni check --output markdown
```

Artifacts record whether execution was `managed` or `host`, the contract digest, source fingerprint, environment-lock fingerprint, and relevant managed runtime/tool versions. Results from incompatible provenance are rejected by `ayni results compare`.

## 7. Use focused feedback while editing

```sh
ayni verify test
ayni verify coverage
ayni impact run --base origin/main
```

Use the narrowest adapter-supported selectors and copy exact rerun commands from artifact findings. Impact success never replaces the final unscoped `ayni check`.

## Evaluation-only host path

When a repository cannot yet satisfy managed prerequisites, `--host` can demonstrate the policy and artifact loop with user-installed tools:

```sh
ayni check --host
ayni verify test --host
```

This is an **evaluation and compatibility path**, not equivalent evidence. Host artifacts are labeled `execution_mode = "host"`, have no environment-lock fingerprint, and are provenance-incompatible with managed artifacts.

## Advanced development access

`env shell` and `env run` expose one locked target for arbitrary development commands. They add no quality semantics and mount the host checkout read-write, so they are intentionally outside the first-run workflow. See [Managed environments](/product/environments#advanced-development-access).

## What to read next

- [How Ayni works](/getting-started/how-ayni-works) explains policy, environment, and evidence.
- [Adapter capability tiers](/product/capabilities) states where each signal is supported, experimental, or unavailable.
- [Managed environments](/product/environments) covers locking, image builds, and runtime behavior.
- [Signals](/product/signals), [Verification](/product/runtime), and [Impact analysis](/product/impact) define the feedback loop.
- [CLI reference](/cli) lists every command and option.
