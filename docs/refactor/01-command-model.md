# Project 01: command model and CLI orchestration

## Goal

Define Ayni's complete greenfield command language and keep the CLI as a thin
orchestration layer.

The command tree should describe user intent. It should not expose internal
collectors, installation mechanisms, or historical product structure.

## Product surface

```text
ayni
├── init
├── env {show,doctor,lock,build,shell,run}
├── contract {show,validate}
├── verify <signal>
├── impact {show,run}
├── check
├── agents sync
└── results {show,compare}
```

### `ayni init`

Prepare a repository for Ayni. It detects supported languages and likely roots,
creates the repository contract, and ignores generated Ayni state. It does not
install tools, build an environment, or change agent guidance.

### `ayni env`

Own the code environment lifecycle:

- `show`: explain the resolved environment plan without modifying state.
- `doctor`: diagnose missing, conflicting, unsupported, or stale state.
- `lock`: resolve exact environment requirements into the committed lock.
- `build`: build the repository code-environment image from a current lock.
- `shell`: enter the managed environment with the checkout mounted.
- `run -- <command>`: run an arbitrary command inside the managed environment.

### `ayni contract`

- `show`: render the effective quality contract.
- `validate`: validate the contract without discovery or tool execution.

### `ayni verify <signal>`

Run one signal against the narrowest adapter-supported target. It produces
focused evidence only.

### `ayni impact`

- `show`: explain the checks affected by a change.
- `run`: execute the calculated impact set.

Neither command produces repository completion evidence.

### `ayni check`

Run the complete configured repository contract. This is the only completion
gate.

### `ayni agents sync`

Create or refresh only Ayni's managed agent-guidance block. This remains an
explicit repository mutation.

### `ayni results`

- `show`: inspect a local check, verify, or impact result.
- `compare`: compare two explicit compatible result files.

## Global behavior

### Execution environment

Quality commands use the managed environment by default when the new product is
complete. `--host` is the explicit escape hatch for local troubleshooting and
unsupported platforms. A command must report which mode it used.

`check` must not silently call `env lock` or `env build`. Environment readiness
is a precondition, not a side effect of measuring quality.

### Output

- Human-readable terminal output is the default.
- JSON writes exactly one deterministic document to stdout.
- Markdown is available where CI presentation is useful.
- Progress and diagnostics use stderr when stdout is a structured output.
- Failure messages include a useful next command whenever possible.

### Exit codes

The stable initial categories are:

| Code | Meaning |
| --- | --- |
| `0` | The requested operation succeeded. |
| `1` | A quality contract or focused verification failed. |
| `2` | CLI input or repository contract is invalid. |
| `3` | The requested environment is missing, stale, or unsupported. |
| `4` | Execution was incomplete or an external tool failed. |

The CLI may use typed internal errors richer than these categories. Rendering
must not collapse environment, product, and tool failures into one message.

## Application flow

Every command follows the same conceptual pipeline:

```text
parse intent
  -> load repository context
  -> construct a typed plan
  -> execute the plan
  -> render the typed outcome
```

Command handlers should primarily map arguments to application operations.
They must not:

- discover language roots directly;
- inspect lockfiles or package managers;
- construct language-tool commands;
- interpret collector output;
- calculate quality status;
- generate product schemas ad hoc.

## Internal organization

The CLI should be split by command domain rather than accumulated in one entry
module:

```text
cli/src/
├── args/
├── commands/
│   ├── init.rs
│   ├── env.rs
│   ├── contract.rs
│   ├── verify.rs
│   ├── impact.rs
│   ├── check.rs
│   ├── agents.rs
│   └── results.rs
├── application/
└── ui/
```

This is illustrative, not a required exact filesystem layout. The important
boundary is between argument parsing, application orchestration, product
semantics, and rendering.

## Non-goals

- Old command aliases or deprecation periods.
- Compatibility with old help text or scripts.
- Language-specific subcommands.
- Installing tooling as a side effect of `check`, `verify`, or `impact`.
- Turning the CLI into a hosted-service client.

## Dependencies

None. This project defines the vocabulary consumed by every other project.

## Deliverables

- The complete command tree and argument model.
- Shared output, environment-selection, and error options.
- Typed application-operation interfaces.
- Help text and generated CLI documentation.
- Parser, conflict, output-routing, and exit-code tests.

## Definition of done

- Every command maps to one typed application operation.
- Command modules are small and contain no language-specific behavior.
- Structured stdout remains free of logs and progress output.
- Mutating behavior exists only in commands whose purpose declares it.
- The checkout binary's complete help reflects this document.
- Superseded commands and compatibility code have been removed.
