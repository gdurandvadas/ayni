# Ayni

Ayni is a local-first code quality signal tool for repositories that use AI
agents.

Ayni installs agent-facing repository guidance and runs language-specific
analysis, then normalizes the results into a single local report.

## Why

AI agents need explicit local signals about repository boundaries, test health,
coverage, complexity, size, and architectural rules.

Ayni helps by:

- adding or updating `AGENTS.md`
- creating `.ayni.toml` policy scaffolding
- collecting `test`, `coverage`, `size`, `complexity`, `deps`, and `mutation` signals
- producing terminal and Markdown reports
- writing machine-readable local artifacts for repair loops

## Install

From this repository:

```sh
cargo install --path cli
```

Check the CLI:

```sh
ayni --help
```

## Quick Start

```sh
ayni install
ayni install --apply
ayni analyze
ayni analyze --output md
```

What these do:

- `ayni install` scaffolds `.ayni.toml`, updates the managed block in `AGENTS.md`, ensures `.gitignore` includes `.ayni/`, and lists required tools.
- `ayni install --apply` also installs missing or outdated tools from local language ecosystems.
- `ayni analyze` prints the stdout report and writes `.ayni/last/signals.json`.
- `ayni analyze --output md` prints Markdown to stdout and writes `.ayni/last/signals.json`.

## Commands

### `install`

Updates local scaffolding and reports tool status as `ok`, `outdated`, or
`missing`.

### `analyze`

Analyzes the local repository and prints a quality report. Scope can be narrowed
with `--file`, `--package`, and `--language`.

Output modes:

- `stdout`: colored terminal report, the default
- `md`: markdown report printed to stdout

### `version`

Prints the Ayni CLI version.

## Example Output

## rust (adapters/go) — 5/5 passing

| #     | Signal         | Summary                                    | Status                                                                                                                                 |
| ----- | -------------- | ------------------------------------------ | -------------------------------------------------------------------------------------------------------------------------------------- |
| **1** | **test**       | `total=7 passed=7 failed=0`                | <img src="https://raw.githubusercontent.com/gdurandvadas/ayni/refs/heads/main/assets/pass.svg" alt="pass" width="20" height="20"> pass |
| **2** | **coverage**   | `percent=40.3% status=ok`                  | <img src="https://raw.githubusercontent.com/gdurandvadas/ayni/refs/heads/main/assets/pass.svg" alt="pass" width="20" height="20"> pass |
| **3** | **size**       | `max_lines=235 files=11 fail_count=0`      | <img src="https://raw.githubusercontent.com/gdurandvadas/ayni/refs/heads/main/assets/pass.svg" alt="pass" width="20" height="20"> pass |
| **4** | **complexity** | `functions=90 max_cyclo=14.0 fail_count=0` | <img src="https://raw.githubusercontent.com/gdurandvadas/ayni/refs/heads/main/assets/warn.svg" alt="warn" width="20" height="20"> warn |
| **5** | **deps**       | `crates=1 edges=0 violations=0`            | <img src="https://raw.githubusercontent.com/gdurandvadas/ayni/refs/heads/main/assets/pass.svg" alt="pass" width="20" height="20"> pass |

<details>
<summary>Offenders</summary>

complexity

- **WARN** `adapters/go/src/collectors/test.rs:23` collect cyclo=14.0
- **WARN** `adapters/go/src/collectors/complexity.rs:10` collect cyclo=13.0
- **WARN** `adapters/go/src/collectors/deps.rs:29` collect cyclo=13.0

</details>

## Local-First

The default `install` and `analyze` workflow does not require login, accounts,
or remote services. It reads local files, runs local tooling, and writes local
output only. `ayni install --apply` may download tools from language package
registries.

## Artifacts

```txt
.ayni/
├── last/
│   └── signals.json
└── history/
    └── previous-signals.jsonl
```

`signals.json` is the typed run artifact. `previous-signals.jsonl` stores the
previous local run snapshot used for deltas.

## Contributing

Developer workflow, architecture constraints, and repository checks live in
`CONTRIBUTING.md`.
