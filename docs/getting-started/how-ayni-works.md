# How Ayni works

Ayni separates quality policy from runtime provisioning, then joins them when a signal is executed.

```text
Quality contract       Managed environment       Signal execution
.ayni.toml          +  .ayni.lock / OCI image  → check / verify / impact
"What is healthy?"    "Where does it run?"       "What did we measure?"
```

That separation gives reviewers three explicit questions instead of one opaque script:

1. Which measurements and thresholds define a healthy repository?
2. Which exact tools and dependencies produce those measurements?
3. Which scope should be measured for this feedback cycle?

## The quality contract: what is healthy?

`.ayni.toml` is repository policy. It defines:

- which languages and roots Ayni evaluates;
- which signals are enabled;
- warning and failure thresholds;
- structural dependency rules;
- concurrency and report controls;
- optional tool-command overrides; and
- optional runtime capabilities such as network or Docker-socket access.

The contract is deterministic and versioned with the code. `ayni contract validate` checks its shape; `ayni contract show` renders the effective policy.

## Signals: what is measured?

A signal turns repository state into normalized evidence. Ayni currently models:

| Signal | Question |
| --- | --- |
| Tests | Does expected behavior pass? |
| Coverage | How much code is exercised? |
| Size | Which files exceed repository limits? |
| Complexity | Which functions exceed structural limits? |
| Dependencies | Do source dependencies respect architectural rules? |
| Mutation | Do tests detect injected behavioral changes? |

Adapters own language-specific command selection and evidence parsing. The contract remains language-neutral at the signal level: a test signal means the same kind of evidence whether its adapter invokes Cargo, npm, Go, uv, or Gradle.

Signals have `pass`, `warn`, or `fail` status. A warning threshold provides feedback without failing the gate. Failure-level findings, tool failures, or missing expected results fail the complete contract.

## The managed environment: where does it run?

A managed environment is derived from:

- configured language roots;
- enabled signals and the tools they require;
- native project metadata and dependency locks;
- repository-wide environment configuration; and
- an immutable Ayni base-image digest.

`ayni env show` explains that derived plan without modifying state. `ayni env lock` resolves it into `.ayni.lock`, and `ayni env build` turns the lock into an OCI image with prepared dependencies.

```text
.ayni.toml + native project metadata
                 │
                 ▼
          ayni env show
                 │
                 ▼
          ayni env lock ───────→ .ayni.lock
                                      │
                                      ▼
                               ayni env build
                                      │
                                      ▼
                              local OCI image
```

Ayni does not silently lock or rebuild during a quality run. Changes to versions, dependencies, tools, capabilities, or preparation inputs therefore remain visible in the lock and image lifecycle.

## Execution: what runs now?

The command chooses scope; Ayni and the adapter choose the exact signal command.

| Command | Scope | Default runtime |
| --- | --- | --- |
| `ayni check` | Every enabled signal across all configured targets | Managed |
| `ayni verify <signal>` | One signal, optionally narrowed to a language, root, file, package, or test | Managed |
| `ayni impact run --base <revision>` | Quality work affected by an explicit Git change | Managed |

At launch, Ayni validates the current lock and image, selects the language target, mounts the checkout, runs the adapter command, parses its output, applies the contract thresholds, and writes normalized evidence.

Use the explicit `--host` option on these commands only when managed execution is not yet available for the repository. Host execution preserves the contract and evidence model but relies on user-installed tools.

## Why `env run` is different

`ayni env run` and `ayni env shell` expose the managed environment for arbitrary development work:

```sh
ayni env run -- cargo test parser::tests
ayni env shell
```

They do not add signal semantics, normalize evidence, or apply quality thresholds. Their checkout is intentionally read-write so development commands can generate files.

By contrast, this is redundant:

```sh
# Do not wrap Ayni's quality commands.
ayni env run -- ayni check
```

Run `ayni check` directly. Managed launch is already part of the command.

## The reviewable repository boundary

A reproducible Ayni setup has several versioned inputs:

| Input | Purpose | Commit it? |
| --- | --- | --- |
| `.ayni.toml` | Quality and runtime policy | Yes |
| `.ayni.lock` | Exact managed environment resolution | Yes |
| Native tool metadata | Toolchain selectors such as `rust-toolchain.toml`, `packageManager`, or Gradle JVM configuration | Yes |
| Native dependency locks | Exact project dependencies | Yes |
| `.ayni/` | Local evidence, history, and materialized runtime state | No |
| OCI image | Local executable environment derived from the lock | No |

The CLI installation itself is machine-level tooling. Installing or upgrading `ayni` does not provision a repository environment or change these files.

## The normal lifecycle

```sh
# Policy work
ayni contract validate
ayni env show

# Rebuild only when environment inputs change
ayni env lock
ayni env build
ayni env doctor

# Daily quality loop
ayni verify test
ayni impact run --base origin/main
ayni check
```

The result is one quality contract, reproducible execution for humans and agents, focused feedback while code changes, and one definitive full gate before integration.

## Next steps

- Follow the [Quickstart](/getting-started/quickstart) for the first repository setup.
- Read [Managed environments](/product/environments) for lock and runtime details.
- Read [Signals](/product/signals) for evidence semantics.
- Read [Runtime and verification](/product/runtime) and [Impact analysis](/product/impact) for focused execution.
