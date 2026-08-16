# Project 06: impact-aware execution

## Goal

Give coding agents the smallest safe validation loop for a change while
preserving `ayni check` as the final repository completion boundary.

Impact analysis optimizes iteration. It is not evidence that the rest of the
repository was checked.

## Commands

### `ayni impact show`

Calculate and explain an impact plan without running quality tools.

Typical inputs include:

- an explicit base revision;
- the working tree, index, or an explicit candidate revision;
- optional output selection.

### `ayni impact run`

Execute the calculated impact plan and store an impact-scoped result. The
result always explains that `ayni check` is still required before completion.

## Planning pipeline

```text
changed files
  -> configured language roots
  -> packages or modules
  -> internal dependency closure
  -> signal-specific affected targets
  -> conservative execution plan
```

Inputs include:

- changed paths and change kinds;
- configured roots;
- workspace/package ownership;
- internal dependency edges;
- available test relationships;
- enabled signals;
- adapter capability and confidence;
- contract or environment changes that invalidate broad scopes.

## Signal-specific strategies

### Test

Select directly affected tests and tests belonging to reverse-dependent
packages. If source-to-test mapping is unavailable, broaden to package or root
tests.

### Coverage

Executable source or test changes affect coverage. Run coverage at the
narrowest scope whose tool output remains valid and comparable. Many coverage
tools will require package- or root-level execution.

### Size

Measure changed source files, plus any files whose generated or moved status
changes scope accounting. A config or exclusion change broadens to every
affected root.

### Complexity

Measure changed source files or functions when the collector supports that
scope. Parser, threshold, or exclusion changes broaden scope.

### Dependencies

Follow changed manifests, imports, modules, and package topology. Architecture
rule changes invalidate every target governed by the changed rule.

### Mutation

Select changed executable code and affected tests only when the adapter can
justify the relationship. Otherwise broaden to the owning package or root.

## Explainability

Every included target carries one or more machine-readable reasons, such as:

```text
apps/web test
  included because src/cart.ts changed
  included because checkout depends on cart

services/api test
  excluded because no dependency path reaches the changed files
```

Exclusions need not list every unaffected file, but meaningful target-level
exclusions should be explainable. Plans also report uncertainty and the scope
increase it caused.

## Conservative broadening

False negatives are more costly than extra checks. Therefore:

- missing topology broadens from file to package or root;
- ambiguous ownership includes every plausible owner;
- contract changes invalidate all governed targets;
- environment-lock changes invalidate affected runtime targets;
- adapter uncertainty increases work rather than dropping it;
- unsupported impact capability falls back to a safe broader plan.

No confidence score may be used to silently omit required work.

## Result semantics

Impact results have an explicit `impact` scope and contain:

- base and candidate identity;
- changed inputs;
- selected targets and reasons;
- broadened scopes and uncertainties;
- executed signal rows and findings;
- incomplete-execution issues;
- a clear non-completion marker.

Impact success means the selected impact plan passed. It does not mean the
repository contract is complete.

## Initial implementation decisions

The first implementation keeps the published schema-v3 repository/focused
artifact unchanged. Impact uses its own versioned envelope and the dedicated
`.ayni/impact/last/impact.json` slot. The envelope repeats the plan and typed
rows, records exact selected-job accounting, and always carries an explicit
`ayni check` requirement.

The frozen CLI requires `--base`; the candidate is the complete current working
tree: commits through `HEAD`, index changes, unstaged changes, and untracked
non-ignored files. Both identities and a deterministic candidate fingerprint
are visible. A plan is recomputed before persistence so candidate drift fails
closed.

Git invocation belongs to CLI application infrastructure. Core owns normalized
change, reason, confidence, selection, uncertainty, and plan types. Adapters own
repository relevance, package ownership, and internal dependency topology.
Rust Cargo and npm Node adapters resolve the governing workspace above
configured member roots and calculate transitive reverse dependencies. Cargo
mapping includes target-specific and workspace-inherited aliased dependencies.
Go, Python, and Gradle Kotlin treat all in-root changes plus governing ancestor
runtime/dependency inputs as relevant, then use adapter-owned conservative root
broadening until equally strong topology mapping is available.

## Caching direction

After selection is proven correct, results may be cached using inputs such as:

```text
environment fingerprint
+ contract fingerprint
+ signal and target
+ relevant source and dependency inputs
= cache key
```

Cache hits must reproduce the same typed evidence and remain visible in the
result. Cache design is deferred until impact planning works without it.

## Non-goals

- Replacing the final full check.
- Aggressive minimalism when topology is incomplete.
- General-purpose build graph execution.
- Distributed execution or remote caching in the first implementation.
- Automatically choosing an implicit baseline that hides repository state.

## Dependencies

- [Project 04](04-quality-contract-execution.md) for signal execution.
- [Project 05](05-adapter-catalog-platform.md) for impact capabilities.
- [Project 07](07-results-comparison-reporting.md) for impact results.

## Deliverables

- Typed change, reason, confidence, and impact-plan models.
- Explicit base/candidate Git integration.
- `impact show` and `impact run` operations.
- Rust and Node dependency and test impact mapping, followed by other adapters.
- Conservative fallback strategies.
- Human and JSON impact explanations.

## Definition of done

- Rust and Node fixtures demonstrate cross-package reverse-dependency impact.
- Every selected target has a machine-readable inclusion reason.
- Uncertain cases broaden safely and report why.
- Contract and environment changes invalidate the correct scopes.
- Impact output cannot be mistaken for repository completion.
- All remaining built-in adapters implement or safely fall back within the same
  impact contract.
