# Project 07: results, comparison, and reporting

## Goal

Produce local, typed results that are immediately useful to humans and coding
agents.

Results record what Ayni measured and whether the requested scope completed.
They are not regulatory attestations and do not depend on a hosted service.

## Local layout

Generated result state lives below `.ayni/`, for example:

```text
.ayni/
└── results/
    ├── check.json
    ├── verify.json
    └── impact.json
```

The exact filenames may add signal or target detail when concurrent workflows
need it, but full, focused, and impact evidence must never overwrite one
another.

## Shared result envelope

Every result records:

- result schema version;
- scope: repository, focused, or impact;
- generation time and Ayni version;
- normalized invocation;
- repository and contract identity;
- expected, detected, completed, and skipped targets;
- environment summary;
- ordered typed signal rows;
- findings and applied budgets;
- command failures;
- structured incomplete-execution issues;
- derived aggregate status.

Derived summaries are rebuilt and validated on input rather than trusted as
independent truth.

## Scope rules

### Repository

Produced only by `ayni check`. It can claim complete repository evidence when
every planned row exists.

### Focused

Produced by `ayni verify`. It identifies the requested signal and selectors and
cannot be promoted to repository scope.

### Impact

Produced by `ayni impact run`. It records change identity, selection reasons,
and uncertainty. It explicitly states that repository completion was not
evaluated.

## Environment summary

Environment data is diagnostic and comparison context, not a signed
attestation. The result should include enough information to answer why two
runs differ:

- execution mode: managed or host;
- environment-lock fingerprint when available;
- image reference or digest when managed;
- OS, architecture, and libc;
- selected runtime and package-manager versions per target.

Sensitive environment variables, credentials, and cache paths are never
serialized.

## Typed rows and findings

Rows use the closed signal vocabulary and typed signal-specific payloads.
Free-form tool output may be included only as bounded diagnostics associated
with a typed command failure.

Findings retain stable semantic identity and exact verification commands.
Duplicate canonical findings are removed deterministically.

## Completion and aggregation

Completion validates actual row coverage, not only reconciled counters. For
each expected target, the result validator knows the requested signal set and
requires the exact corresponding row keys.

Aggregate status passes only when:

- the requested scope completed;
- every required row exists and validates;
- no row fails its quality contract.

Warnings remain visible without becoming failures unless the contract says
otherwise.

## Output projections

### Terminal

Concise progress and summary for humans, with actionable findings and explicit
completion state.

### JSON

Exactly one deterministic document on stdout. Progress and diagnostics use
stderr. Non-finite metrics and malformed typed payloads are rejected.

### Markdown

A deterministic projection suitable for CI job summaries and pull-request
comments. It must communicate scope prominently so focused or impact output is
not mistaken for a full gate.

## `ayni results show`

Read one explicit result file or a well-defined local result slot and render it
without repository discovery or quality execution.

## `ayni results compare`

Compare two explicit complete and compatible result files. Comparison reports:

- matched, added, and removed row keys;
- typed metric changes;
- pass/fail changes;
- added and removed stable finding IDs;
- environment differences relevant to interpretation.

The command rejects incomplete, malformed, or incompatible inputs. It does not
fetch history, inspect Git, discover an implicit baseline, or write a new
result.

Comparing different environment fingerprints may be allowed with a prominent
compatibility warning when typed semantics remain compatible, or rejected when
the difference makes measurements incomparable. This rule must be explicit per
schema rather than inferred by the renderer.

## Privacy and sharing

Results can expose repository-relative paths, package names, commands, and
bounded raw diagnostics. Documentation must tell users to treat them as
repository diagnostics when sharing. No result includes secrets by design.

## Non-goals

- Signing, attestations, or regulatory compliance positioning.
- Hosted storage or fleet dashboards.
- Implicit baseline selection.
- Backwards compatibility with old artifact schemas.
- Treating presentation summaries as independently authoritative data.

## Dependencies

- Core contract, signal, finding, environment, and completion models.
- [Project 06](06-impact-aware-execution.md) for impact-specific context.

## Deliverables

- New versioned result schema.
- Scope-aware local persistence.
- Strict serialization, deserialization, and derived-view validation.
- Terminal, JSON, and Markdown renderers.
- `results show` and explicit result comparison.
- Golden schemas and malformed-input tests.

## Definition of done

- Repository, focused, and impact results serialize deterministically.
- Actual expected row sets are validated before completion can pass.
- Structured stdout remains clean and automation-safe.
- Focused and impact evidence cannot replace full-check evidence.
- Comparison reports typed changes and rejects invalid inputs.
- Environment context is useful for diagnosis without claiming attestation.
