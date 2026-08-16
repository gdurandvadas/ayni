# Project 04: quality contract and execution engine

## Goal

Evaluate the complete repository quality contract with typed, fail-closed
execution semantics.

This project preserves Ayni's strongest existing product idea while rebuilding
its configuration and execution surface without compatibility constraints.

## Repository contract

`.ayni.toml` remains the human-owned contract between the repository and coding
agents. The new schema owns:

- enabled languages and configured roots;
- enabled quality signals;
- thresholds, budgets, and exclusions;
- architectural dependency rules;
- explicit tool-command overrides where necessary;
- execution preferences that are genuinely product semantics.

The schema is designed for the clean-slate product. Old fields do not need to
parse, migrate, warn, or retain their old meaning.

## Signal vocabulary

The initial closed vocabulary remains:

| Signal | Purpose |
| --- | --- |
| `test` | Test discovery and execution health. |
| `coverage` | Measured line and branch coverage against minimums. |
| `size` | Source file or module size against maximum budgets. |
| `complexity` | Function complexity against maximum budgets. |
| `deps` | Repository architecture and forbidden dependency edges. |
| `mutation` | Test effectiveness against introduced behavioral changes. |

Adapters may support a subset, but an enabled required signal that cannot run
for a configured target must not silently disappear.

## Execution planning

A repository check expands the contract into a deterministic matrix:

```text
configured target × enabled adapter-supported signal
```

Planning produces explicit row keys before scheduling. Every planned row must
finish as exactly one of:

- a typed signal result, including a quality failure; or
- a structured incomplete-execution issue.

There is no state where an omitted row is treated as passing.

Target detection, environment resolution, signal selection, scheduling, and
collection failures remain distinct stages so agents can repair the actual
problem.

## `ayni check`

`check` is the only repository completion gate. It:

- evaluates every configured target and enabled signal;
- uses the managed environment by default;
- records exact target accounting;
- writes the canonical full-check result;
- fails when quality violates the contract;
- fails when required execution is incomplete;
- never installs tools, edits manifests, or changes locks.

A complete result may contain failing quality rows. Completion means all
required evidence was produced, not that the repository passed.

## `ayni verify <signal>`

Focused verification is the repair-loop primitive. It:

- runs exactly one requested signal;
- accepts only selectors declared by the adapter capability;
- requires an explicit root when the requested selector would otherwise be
  ambiguous;
- writes focused results separately from full-check results;
- never claims repository completion.

Selectors may include root, file, package, and test name. The supported
combinations are adapter- and signal-specific and must be validated before a
tool runs.

## Findings

Every actionable finding contains:

- a stable identity based on semantic offender fields;
- signal, language, and target context;
- typed measured values;
- applied budget, threshold, or rule;
- normalized repository-relative location;
- severity;
- an exact, copyable verification command.

Finding identity excludes values that should not break correlation, such as
current severity, output order, timestamps, and checkout location.

The verification command includes every option required to rerun the current
finding against the originating contract and root. Consumers must copy it
rather than synthesize selectors.

## Failure semantics

A quality failure is not a command failure. A test runner's non-zero exit is
ordinary quality evidence when a parseable report identifies failed tests; it
is incomplete execution only when failed-test evidence is absent. The result
model distinguishes:

- a tool ran successfully and measured a policy violation;
- a test runner reported failed tests and exited non-zero;
- a tool exited unsuccessfully without complete quality evidence;
- output was missing or unparseable;
- a target could not be detected or resolved;
- the environment was not ready;
- the command exceeded its timeout;
- the requested selector was invalid.

Configured evidence fails closed. For example, a configured coverage metric
with no parsed measurement cannot pass.

## Scheduling and command execution

- Plans have deterministic order even when work runs concurrently.
- Adapter-declared concurrency restrictions are honored.
- Every external process uses the shared command runner.
- Configured timeouts apply consistently to every process.
- Streaming output should be observable while commands run.
- Cancellation terminates process trees, not only immediate children.
- Command evidence records executable, arguments, working directory, outcome,
  and classified failure without losing typed context.

## Initial execution-boundary decisions

- Full and focused artifacts derive one core `RunOutcome`: incomplete execution
  takes precedence over quality failure, which takes precedence over pass.
- CLI application services own scheduling and persistence, while core owns
  outcome and result semantics. Argument parsing and the binary entrypoint do
  not calculate aggregate status.
- Reconciliation validates emitted language, signal, root, package, and file
  identity against the planned row before evidence can count as completed.

## Non-goals

- Impact-based target selection.
- Environment provisioning.
- Hosted history or organization policy distribution.
- Compatibility with previous policy or result schemas.
- Allowing a focused run to promote itself to full completion.

## Dependencies

- [Project 01](01-command-model.md) for command semantics.
- [Project 02](02-code-environment-image.md) for managed execution.
- [Project 05](05-adapter-catalog-platform.md) for collectors and selector
  capabilities.
- [Project 07](07-results-comparison-reporting.md) for the canonical result
  model.

## Deliverables

- New contract schema and validation.
- Deterministic target/signal execution planner.
- Full-check and focused-verification application operations.
- Unified scheduler and command-runner integration.
- Stable finding construction and verification-command rendering.
- Completion reconciliation and fail-closed aggregation.
- Cross-adapter contract and selector tests.

## Definition of done

- Missing rows and skipped targets cannot aggregate to pass.
- Full check and focused verification have distinct scopes and storage.
- Every built-in adapter satisfies shared completion and selector tests.
- Every actionable finding has a valid exact verification command.
- Quality, environment, tool, and incomplete-execution failures remain
  distinguishable in terminal and machine output.
