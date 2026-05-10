# Python Adapter

The Python adapter implements the same `LanguageAdapter` and `SignalCollector`
contracts as the Rust, Go, and Node adapters.

It detects Python project roots, resolves per-root package manager behavior, and
emits canonical `SignalRow` values for each enabled `SignalKind`.
Runtime behavior follows the product-level [runtime and setup rules](../product/runtime.md).

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

Per root, package manager resolution is workspace-aware:

1. direct root markers such as `uv.lock`, `poetry.lock`, `pdm.lock`, `Pipfile.lock`, and `hatch.toml`
2. ancestor workspace markers, currently `uv.lock` or `[tool.uv.workspace]`
3. direct manifest fallback (`pyproject.toml` or `requirements.txt`) to `python -m ...`

This lets member roots inside a shared `uv` workspace execute with `uv run ...`
instead of falling back to plain `python -m ...`.

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
- optional `[python.foundation]` (`runner`, `validate_install`)
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

[python.foundation]
runner = "workspace"
validate_install = true

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

The flow is deterministic and idempotent. `ayni install --apply` also validates
that the resolved runner can invoke the installed Python tools for each root.

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

When Python tooling fails for repo code or setup reasons, collectors prefer
failed signal rows with normalized command diagnostics over adapter aborts.
