# Audit — Usage Improvements

## Original Goal
Make Ayni output actionable for humans and agents, support multi-language setup, and make managed agent-instruction updates explicit. The required report contract includes typed Offenders plus complete optional command Failures, schema-v2 JSON, repeated install languages, and `ayni agents sync` as the sole managed-block writer.

## Presence and Absence Proof
| Property | Evidence | Result |
|---|---|---|
| Target behavior | Schema `0.2.0` validates derived metadata/aggregates/thresholds/summaries against canonical rows (`core/src/signal.rs:110-117,219-309`). `--json` resolves to JSON, conflicts with `--output md|stdout` fail, and one serialized artifact is persisted then emitted (`cli/src/main.rs:892-950`). Repeated install languages become a deduplicated `BTreeSet` and compose bootstrap policy (`cli/src/main.rs:220-235`; `cli/src/install.rs:16-40,323-337,422-483`). `agents sync` owns managed-block writes (`cli/src/agents.rs:7-20,53-87`). | pass |
| Former behavior removed or unreachable | Production search found only `cli/src/agents.rs` writes `AGENTS.md`; install no longer owns a template or write path. Install no longer consumes `Option<Language>` as its language filter. No schema-v1 contract claims remain; `0.1.0` hits are fixture/package versions. | pass |
| Removal inventory complete | **Incomplete:** `failed_signal_row` creates a `CommandFailure`, but `command_failure` drops it for `Size` and `Deps` (`cli/src/main.rs:433-533`; `core/src/signal.rs:312-319`). Those command failures are unreachable from schema-v2 failure summaries, Markdown `Failures`, and Markdown stderr diagnostics. | fail |
| Authoritative gate exercised | Prior implementation report states all fmt, clippy, workspace tests, check, generated CLI docs, and analyze checks passed, but the audit has no durable command output to independently confirm them. Existing tests cover contracts/helpers; they do not run both JSON CLI forms with stdout/stderr assertions or force an actual collector command failure through Markdown mode (`cli/src/tests.rs:182-260`; `cli/src/ui/md_report.rs:310-443`). | fail |

## Commits
- `2bc94e1 feat(core): [0001] define rich analysis artifact contract`
- `bc236fd feat(install): [0001] support repeated language selection`
- `89f997a feat(agents): [0001] add explicit agent sync command`
- `9471350 feat(report): [0001] expose offenders and command failures`
- `b9e9864 feat(cli): [0001] add rich json output selector`
- `af708b9 docs(usage): [0001] document setup and output contracts`
- `1acc937 fix(usage): [0001] complete usage replacement`

All implementation commits use the required `[0001]` prefix. `8748880 chore: version update` is untagged and intervenes between the first task commit and the task implementation commits; its `.gitignore` docs-path and package-version changes are baseline/version work, not usage-feature implementation. The first task commit is an ancestor of `fix/better-errors` but is also currently the local `main` tip; the branch therefore still contains it for a PR against `origin/main`.

## Deviations & Rationale
| Planned | Actual | Rationale |
|---|---|---|
| Surface every command failure in optional Markdown Failures and agent-readable JSON summaries | Size and dependency command failures are discarded after construction | Implementation gap; no rationale established |
| Validate report behavior with forced command failure and both JSON selectors | Unit/contract tests cover report formatting and selector resolution, but no end-to-end CLI evidence is present | Evidence gap; must add meaningful CLI-level coverage |
| `agents sync` is the sole writer | Implemented in `cli/src/agents.rs`; install no longer writes `AGENTS.md` | followed plan |
| Support repeated languages | Implemented through a deduplicated language set and composed policy templates | followed plan |

## Blast Radius
`RunArtifact` schema v2 is persisted to `.ayni/last/signals.json` and emitted by JSON output, so its rows, aggregate, threshold, offender, and failure views are public agent-consumption contracts. Markdown reports and stderr diagnostics depend on the same failure extraction. Dropping Size/Deps failure data prevents CI and agents from diagnosing broken size/dependency collectors, despite the new report contract. `install` consumes adapter discovery and scaffolding paths across Rust, Go, Node, Python, and Kotlin; reverting multi-language selection would make polyglot bootstrap incomplete. `agents sync` is the sole owner of the marked block, so changing its marker/upsert behavior can affect existing user-authored `AGENTS.md` files.

## Key Files Touched
- `core/src/signal.rs` — schema-v2 artifact and summary derivation; excludes Size/Deps command failures today.
- `cli/src/main.rs` — JSON selector/routing and failed-signal construction.
- `cli/src/ui/md_report.rs` — Markdown Offenders and conditional Failures rendering.
- `cli/src/ui/progress_log.rs` — Markdown-mode failure diagnostics.
- `cli/src/install.rs` and `cli/src/discovery.rs` — multi-language install/scaffolding flow.
- `cli/src/agents.rs` — explicit managed agent-block synchronization.
- `cli/src/tests.rs` — parser/install/agents/artifact test coverage.
- `README.md`, `CONTRIBUTING.md`, `docs/cli.md`, `docs/product/*.md`, `docs/adapters/*.md` — reconciled public contract and setup guidance.

## Current Documentation Reconciliation
Current public documentation describes schema v2, `--json` and `--output json`, repeated `--language`, explicit `agents sync`, threshold semantics, and Markdown failure fields. `docs/product/signals.md` includes failure `category`; `README.md:172-176` documents the other fields but omits `category`. Update the README when repairing the failure-propagation gap. No protected-document edits are proposed in this audit.

## Verdict
- **Verdict:** fail
- **Foundational blockers:** 2
  1. Size and dependency collector command failures are dropped and violate the report/JSON diagnostic contract.
  2. Required behavior lacks meaningful CLI-level evidence for JSON stdout/stderr routing and a forced real collector failure flowing to Markdown Failures.
- **Next action:** return to implement
