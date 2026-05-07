# Python Adapter

The Python adapter implements the same `LanguageAdapter` and `SignalCollector`
contracts as the Rust, Go, and Node adapters.

It detects Python project roots, resolves per-root package manager behavior, and
emits canonical `SignalRow` values for each enabled `SignalKind`.

## Module layout

```text
adapters/python/src/
├── lib.rs
├── adapter.rs
├── catalog.rs
└── collectors/
    ├── mod.rs
    ├── test.rs
    ├── coverage.rs
    ├── size.rs
    ├── complexity.rs
    ├── deps.rs
    └── mutation.rs
```

## Signal coverage

| Signal kind | Collector module | Source tool / method |
| --- | --- | --- |
| `test` | `collectors/test.rs` | `pytest` with `pytest-json-report` |
| `coverage` | `collectors/coverage.rs` | `pytest-cov` / `coverage.py` JSON |
| `size` | `collectors/size.rs` | file walk + `[python.size]` glob budgets |
| `complexity` | `collectors/complexity.rs` | `complexipy` JSON cognitive complexity |
| `deps` | `collectors/deps.rs` | Rust-side import extraction + forbidden rules |
| `mutation` | `collectors/mutation.rs` | `mutmut` + JUnit XML |

Every collector outputs:

- a canonical `SignalResult` variant
- a matching typed offender list
- policy-aware pass evaluation

## Root and package manager detection

Python roots are discovered from `[python].roots` and adapter detection.

Per root, package manager resolution precedence is:

1. `uv.lock`
2. `poetry.lock`
3. `pdm.lock`
4. `Pipfile.lock`
5. `hatch.toml`
6. `pyproject.toml`
7. `requirements.txt`

When no manager can be confidently inferred, runtime behavior falls back to
`python -m` assumptions.

## Tool catalog

Python tools are declared in `catalog.rs` with typed installers. Local
test/coverage tools are installed through the detected Python package manager.
`complexipy` is installed as an isolated `uv tool`.

Each entry declares install/check behavior, required signal kinds
(`for_signals`), and opt-in status for expensive checks such as mutation.

## Policy expectations

Python collectors read these `.ayni.toml` sections:

- `[checks]`
- `[python]` (`roots = [...]`)
- `[python.size]` (glob budgets)
- `[python.complexity]` (`fn_cognitive`)
- `[python.coverage]` (`line_percent`)
- `[python.deps.forbidden]` (forbidden dependency edges)
- optional `[python.tooling.test]`, `[python.tooling.coverage]`, `[python.tooling.mutation]` command overrides

If required policy fields are missing, collectors return explicit errors.

## Full TOML example

```toml
[languages]
enabled = ["python"]

[python]
roots = ["."]

[python.tooling.test]
command = "uv"
args = ["run", "pytest", "--json-report", "--json-report-file", ".ayni/pytest-report.json"]

[python.tooling.coverage]
command = "uv"
args = ["run", "pytest", "--cov=.", "--cov-report=json:.ayni/coverage.json"]

[python.tooling.mutation]
command = "uv"
args = ["run", "mutmut", "run"]

[python.size]
"**/*.py" = { warn = 400, fail = 800, exclude = [".venv/**", "venv/**", "env/**", "__pycache__/**", ".pytest_cache/**", ".ruff_cache/**", ".tox/**", ".nox/**", ".git/**", ".ayni/**"] }

[python.complexity]
fn_cognitive = { warn = 10, fail = 15 }

[python.coverage]
line_percent = { warn = 80, fail = 60 }

[python.deps.forbidden]
"src/domain/**" = ["src/presentation/**"]
```

## `ayni install --language python`

`--language python` scopes installation to Python catalog entries only.

```sh
ayni install --language python --repo-root <path>
```

The flow is deterministic and idempotent.

## `ayni analyze --language python`

`--language python` limits analysis planning to Python roots and Python
collectors.

```sh
ayni analyze --config ./.ayni.toml --language python --output stdout
```

Use markdown output when needed:

```sh
ayni analyze --config ./.ayni.toml --language python --output md
```

## Output guarantees

The adapter emits only core-defined signal kinds and typed payloads. It must not
emit ad-hoc row shapes, so reporting remains stable across languages and output
formats.
