# Python Adapter

## Installation

Python roots are directories containing `pyproject.toml`, `requirements.txt`,
or `Pipfile`; discovery excludes virtual environments, caches, `.git`, and
`.ayni`. A uv workspace or root `uv.lock` controls workspace discovery, and uv
workspace exclusions are respected. Configure roots in `[python].roots`.

Runner resolution prefers direct `uv.lock`, `poetry.lock`, `pdm.lock`,
`Pipfile.lock`, or `hatch.toml`, then an ancestor uv workspace, then
`pyproject.toml` or `requirements.txt` using `python -m`. Python and its
resolved package manager are user-owned prerequisites. Applied installation
uses the resolved manager for local development packages and `uv tool` for
`complexipy`.

## Signal Coverage

| Signal | Required tool or method | Version contract |
| --- | --- | --- |
| `test` | `pytest`; `pytest-json-report` | no version enforced |
| `coverage` | `pytest`; `pytest-cov`; `coverage` | no version enforced |
| `size` | built-in Python source scan | no version enforced |
| `complexity` | `complexipy` | no version enforced |
| `deps` | Python import scan | no version enforced |
| `mutation` | `mutmut` (opt-in) | no version enforced |

Focused verification

`ayni verify test --language python` supports a repository-relative `--file`
or package path, plus an optional `--name` selector. Selectors are translated
to pytest node IDs and use the configured Python test command when one exists.

## Contract

Enabled checks come from `[checks]`. Configure roots in `[python].roots`
(default `["."]`), optional runner settings in `[python.foundation]`, size
budgets in `[python.size]`, cognitive complexity in `[python.complexity]`,
coverage in `[python.coverage]`, and forbidden edges in
`[python.deps.forbidden]`. Command overrides are optional in
`[python.tooling.test]`, `[python.tooling.coverage]`, and
`[python.tooling.mutation]`; each override requires `command` and may set `args`.

Size requires a budget entry and complexity requires `fn_cognitive`; either
missing value produces a clear collector error. Coverage thresholds and
dependency rules are optional: without `line_percent`, coverage has no policy
threshold, and without `python.deps.forbidden`, no edges are forbidden.

## Configuration Example

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

[python.size]
"**/*.py" = { warn = 400, fail = 800, exclude = [".venv/**", "venv/**", "__pycache__/**", ".git/**", ".ayni/**"] }

[python.complexity]
fn_cognitive = { warn = 10, fail = 15 }

[python.coverage]
line_percent = { warn = 80, fail = 60 }

[python.deps.forbidden]
"src/domain/**" = ["src/presentation/**"]
```
