# Project 08: repository initialization and agent guidance

## Goal

Prepare repositories for the new Ayni workflow without hidden installation or
implicit agent-file mutation.

Initialization establishes the repository contract. Agent guidance teaches the
workflow. Environment provisioning remains a separate explicit lifecycle.

## `ayni init`

`init` performs repository bootstrap:

- detect supported languages;
- discover likely workspace roots through adapters;
- generate a minimal `.ayni.toml`;
- ensure generated `.ayni/` state is ignored;
- summarize detected environment inputs;
- print explicit next steps.

Example outcome:

```text
Created .ayni.toml
Updated .gitignore

Detected:
  rust    .
  node    apps/web

Next:
  ayni contract show
  ayni env lock
  ayni env build
  ayni agents sync
```

The exact generated contract must be valid and intentionally conservative. Ayni
does not invent strict thresholds merely to fill every field. Defaults should
be documented product decisions.

## Detection behavior

Language adapters own markers, root discovery, workspace boundaries, and
exclusions. `init` orchestrates their findings and resolves user-facing
ambiguity.

When multiple plausible layouts exist, `init` should explain them and require
an explicit selection if choosing incorrectly would materially change the
contract. It must not scan or write outside the canonical repository boundary.

Repeated language detections and normalized roots are deduplicated
deterministically.

## Mutation rules

`init` may modify only:

- `.ayni.toml` according to explicit initialization behavior;
- the relevant ignore file to exclude `.ayni/`.

It does not:

- install runtimes, package managers, dependencies, or signal tools;
- generate or build `.ayni.lock`;
- build an OCI image;
- run quality checks;
- create or update `AGENTS.md`;
- add project dependencies or Gradle plugins.

Re-running `init` must be deterministic and preserve user-owned configuration
according to a documented update policy. If safe merging is not possible, it
should explain the conflict rather than overwrite work.

## `ayni agents sync`

This is the only command that creates or refreshes Ayni's managed guidance
block. It preserves all content outside explicit markers.

The managed guidance teaches agents:

- `.ayni.toml` is the authoritative quality contract;
- `ayni contract show` explains effective expectations;
- `ayni env doctor` diagnoses environment readiness;
- managed environment commands are preferred over host assumptions;
- `ayni verify <signal>` is the focused repair loop;
- the exact verification command on a finding should be copied;
- `ayni impact run` is safe iteration evidence, not completion;
- `ayni check` is the final repository boundary;
- contracts, thresholds, locks, and environment inputs must not be weakened to
  silence failures.

The block stays short. CLI help and repository documentation carry detailed
reference material.

## Agent workflow

The intended workflow after initialization is:

```sh
ayni contract show
ayni env doctor
ayni env build       # only when the lock is current and the image is absent
ayni env shell

# During work
ayni impact run --base <explicit-base>
ayni verify <signal> <selectors>

# Completion boundary
ayni check
```

An agent should not rebuild or relock the environment unless its task changes
environment inputs or readiness explicitly requires it.

## Repository-local source of truth

Generated guidance should point contributors to `docs/refactor/` while the
clean-slate work is underway only when the repository itself is implementing
the refactor. Consumer repositories receive product workflow guidance, not
Ayni's internal roadmap.

## Non-goals

- Silent installation or provisioning.
- Implicit changes to agent guidance.
- Vendor-specific Codex, Copilot, Claude, or Cursor orchestration.
- Long generated instruction manuals.
- Treating root detection guesses as confirmed user intent.

## Dependencies

- [Project 01](01-command-model.md) for command vocabulary.
- [Project 03](03-environment-planning-locking.md) for environment explanation.
- [Project 04](04-quality-contract-execution.md) for contract generation and
  final-check semantics.
- [Project 05](05-adapter-catalog-platform.md) for language detection.
- [Project 06](06-impact-aware-execution.md) for impact guidance.

## Deliverables

- Adapter-driven repository detection and initialization planner.
- Minimal clean-slate contract templates.
- Safe ignore-file update behavior.
- Explicit next-step report.
- Managed agent-guidance template and marker-preserving synchronizer.
- Idempotence, ambiguity, containment, and content-preservation tests.

## Definition of done

- A new supported repository can move from `init` to `env lock` and `env build`
  using the printed instructions.
- Generated policy reflects detected languages and roots without unsupported
  assumptions.
- No installation, image build, or agent-guidance mutation happens implicitly.
- Managed guidance uses the new `verify`, `impact`, and `check` semantics.
- Re-running initialization and guidance sync is deterministic and preserves
  user-owned content.
