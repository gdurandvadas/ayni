# Decisions — Usage Improvements

## State Transition
The intended transition replaces limited analysis output, single-language install selection, and install-time agent-file mutation with schema-v2 agent-oriented output, repeated language selection, and explicit `ayni agents sync`. Compatibility retained: default progress behavior, one-value `--language`, and `--output json`. The transition is incomplete because Size and Deps command failures do not survive into the output contract.

## Decisions
| Decision | Rationale |
|---|---|
| Keep typed Offenders separate from optional command Failures | Findings and failed tool execution need different remediation and report presentation. |
| Retain typed rows while adding schema-v2 summaries | Agents need aggregate metadata and quick diagnosis without losing canonical per-signal data. |
| Add `--json` without removing `--output json` | Preserves existing invocation compatibility while providing the requested convenience flag. |
| Use repeated, deduplicated `--language` flags | Supports polyglot setup while preserving one-language and no-language behavior. |
| Make `ayni agents sync` the sole managed `AGENTS.md` writer | Agent guidance updates become explicit and have one ownership boundary. |
| Return to implementation | The report contract is materially incomplete for Size/Deps command failures and lacks sufficient CLI-level proof. |

## Removed
- Install-time `AGENTS.md` synchronization and install-owned managed-block template.
- Single-value install language filtering and single-template selected-language bootstrap.
- Schema-v1 output-contract claims and stale install/agent-update documentation.

## Blast Radius
Schema-v2 JSON, persisted artifacts, Markdown reports, and stderr diagnostics are consumed by CI and agents. Any failure-extraction defect hides collector execution failures from those consumers. Multi-language install spans all CLI-supported adapters, and `agents sync` directly changes only the Ayni-marked section of user agent guidance.

## Verification Evidence
- `git status --short --branch && git diff --check && git log --oneline --decorate --graph -15` — clean tracked working tree, branch topology, and task commit sequence inspected.
- `git log --oneline --all --grep='\[0001\]'` — seven task-tagged implementation commits found in required format.
- Read-only implementation audit — confirmed schema v2, selector conflict handling, repeated-language selection, agents-sync isolation, and report rendering paths; identified Size/Deps failure loss.
- Prior implementation report: `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets --all-features -- -D warnings`, `cargo test --workspace --all-features`, `cargo check --workspace --all-features`, `cargo doc-cli > docs/cli.md`, and `cargo run -p ayni-cli -- analyze --config ./.ayni.toml` — reported passing, but audit cannot independently substantiate output artifacts.

## Remaining Work
- **foundational blocker:** Preserve `CommandFailure` for Size and Deps result paths so it appears in schema-v2 summaries, Markdown Failures, and stderr diagnostics; add a regression test for each affected signal.
- **foundational blocker:** Add CLI-level tests that execute both JSON selectors and a forced collector command failure, asserting stdout/stderr and Markdown Failure behavior.
- **historical record:** `2bc94e1` was created on local `main` before the rest of the branch work; it is already an ancestor of `fix/better-errors`, so it remains included in the branch task history.
