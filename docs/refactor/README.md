# Ayni clean-slate refactor

This directory is the local source of truth for Ayni's clean-slate product
refactor. It mirrors the projects in the private GitHub roadmap so agents can
understand the complete program without access to GitHub Projects.

The documents describe the **target product**, not the currently released CLI.
Until a project is implemented, the repository's public documentation and the
checkout binary continue to describe current behavior.

## Product definition

> Ayni gives coding agents the correct code environment, the smallest safe
> feedback loop, and one definitive repository quality gate.

The refactored product has three primary capabilities:

```text
Environment -> Can this repository be worked on correctly?
Impact      -> What is the smallest safe validation for this change?
Check       -> Has the complete repository quality contract been satisfied?
```

Ayni remains a local-first CLI. A hosted control plane may be considered after
the local workflow is mature, but it is not part of this program.

## Clean-slate rule

This is a hard reset of the product surface:

- Do not preserve old command names, aliases, schemas, or migration paths.
- Do not add compatibility layers for released configuration or artifacts.
- Reuse proven concepts and implementation where they fit the new model.
- Prefer deletion or direct replacement over parallel old/new systems.
- Rewrite public documentation when the corresponding project becomes real.

The parts worth carrying forward are the six-signal vocabulary, typed results,
fail-closed completion, stable finding identity, focused verification,
language-owned adapters, deterministic output, and explicit agent guidance.

## Roadmap documents

The numbering matches the GitHub roadmap and the order in which the product is
usually explained. It is not a strict implementation order.

1. [Command model and CLI orchestration](01-command-model.md)
2. [Code-environment base and repository image](02-code-environment-image.md)
3. [Environment discovery, planning, and locking](03-environment-planning-locking.md)
4. [Quality contract and execution engine](04-quality-contract-execution.md)
5. [Adapter and tool-catalog platform](05-adapter-catalog-platform.md)
6. [Impact-aware execution](06-impact-aware-execution.md)
7. [Results, comparison, and reporting](07-results-comparison-reporting.md)
8. [Repository initialization and agent guidance](08-initialization-agent-guidance.md)

## Cross-project invariants

These rules apply to every project:

1. The CLI translates user intent into typed operations; it does not own
   language or product semantics.
2. Language-specific detection, version interpretation, package management,
   tool selection, execution, parsing, and impact rules belong to the owning
   adapter.
3. Quality commands do not install tools, modify dependency manifests, or
   update locks.
4. The code environment is for development and analysis, not production
   deployment.
5. `verify` and `impact` produce limited-scope evidence and never claim
   repository completion.
6. `check` is the only complete repository gate.
7. Missing evidence, skipped targets, unsupported required work, and malformed
   tool output fail closed.
8. Human output is the default. Machine output is deterministic and isolated
   on stdout; diagnostics use stderr.
9. Generated state belongs under `.ayni/`. Committed contracts and locks do
   not.
10. Ayni stays useful without a hosted service.

## Target command tree

```text
ayni
├── init
├── env
│   ├── show
│   ├── doctor
│   ├── lock
│   ├── build
│   ├── shell
│   └── run
├── contract
│   ├── show
│   └── validate
├── verify
│   └── <signal>
├── impact
│   ├── show
│   └── run
├── check
├── agents
│   └── sync
└── results
    ├── show
    └── compare
```

## Target architecture

The physical crate layout may evolve, but ownership must remain clear:

```text
core
├── contract
├── environment plan and lock
├── execution and impact plans
├── results and completion
└── finding identity

adapters/common
├── process execution
├── filesystem safety
├── cache primitives
└── catalog execution

adapters/<language>
├── repository discovery
├── environment requirements
├── package resolution
├── signal collection
└── impact mapping

environment
├── mise backend
├── OCI builder
└── workspace launcher

cli
├── argument parsing
├── application orchestration
└── rendering
```

Dependencies continue to point inward. Environment backends consume validated
core plans and adapter requirements; they do not become a place for product or
language semantics.

## Implementation sequence

Project numbering expresses the product narrative. A practical build sequence
is:

1. Freeze the command vocabulary and global CLI behavior.
2. Define the new contract, target, environment-plan, execution-plan, and
   result types.
3. Define adapter capabilities and shared conformance tests.
4. Implement environment discovery and locking for Rust and Node.
5. Build the minimal base image and repository environment launcher.
6. Implement full `check` and focused `verify` on the new models.
7. Add Python, Go, and Kotlin environment support.
8. Implement impact planning for Rust and Node, then the remaining adapters.
9. Finalize results comparison, initialization, and agent guidance.
10. Update the public documentation and remove superseded implementation.

## How agents should use this directory

Before implementing a roadmap project:

1. Read this index and the project's document completely.
2. Read every dependency document linked by that project.
3. Inspect the current implementation only to identify reusable behavior; do
   not infer compatibility requirements from it.
4. Confirm ownership and dependency direction before editing.
5. Record new cross-project decisions in this directory, not only in a PR or
   GitHub Project item.

If these documents disagree with current public docs, the distinction is
intentional: public docs describe released behavior, while this directory
describes the refactor target.
