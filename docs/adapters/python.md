# Python Adapter

## Installation

Managed Python support is intentionally bounded to uv-locked `pyproject.toml`
projects. Add an exact or bounded `[tool.uv].required-version`, a
`.python-version` or compatible `project.requires-python`, and commit `uv.lock`.
Every enabled project tool (`pytest`, `pytest-json-report`, `pytest-cov`,
`coverage`, `complexipy`, or opt-in `mutmut`) must be declared by the project
and have one unambiguous exact version in `uv.lock`.

`env build` warms uv's cache from staged, digest-checked workspace manifests and
the lock. Managed launch creates a fresh root-specific `.venv` offline, mounts
it over the checkout without modifying repository files, and forces uv's frozen,
o-sync, offline behavior. Poetry, PDM, Pipenv, Hatch, plain pip, excluded uv
workspace members, ambiguous locked tool versions, and undeclared project tools
fail closed for managed execution. They remain available through the explicit
`--host` path with their documented user-owned prerequisites.

## Signal Coverage

| Signal | Required tool or method | Version contract |
| --- | --- | --- |
| `test` | `pytest`; `pytest-json-report` | managed: exact `uv.lock`; host: no version enforced |
| `coverage` | `pytest`; `pytest-cov`; `coverage` | managed: exact `uv.lock`; host: no version enforced |
| `size` | built-in Python source scan | no external version |
| `complexity` | `complexipy` | managed: exact `uv.lock`; host: no version enforced |
| `deps` | Python import scan | no external version |
| `mutation` | `mutmut` (opt-in) | managed: exact `uv.lock`; host: no version enforced |

## Focused verification

`verify` writes requested-scope evidence only to `.ayni/verify/last/signals.json`.
Every command accepts an optional `--language python`; unscoped verification is
always valid. The accepted selectors are:

| Signal | `--file` | `--package` | `--name` |
| --- | --- | --- | --- |
| `test` | yes | yes | yes |
| `coverage` | no | no | no |
| `size` | yes | no | no |
| `complexity` | yes | no | no |
| `deps` | no | no | no |
| `mutation` | no | no | no |

Test selectors are translated to pytest node IDs and use the configured Python
test command when one exists. `--name` is test-only, and `--file` cannot be
combined with `--package`. Unsupported or ambiguous selectors are rejected
before a tool runs.

Verification commands carry their originating contract and target, for example:
`ayni verify test --host --config './.ayni.toml' --language python --root '.' --file
'tests/test_api.py' --name 'test_create'`. Use only the selectors marked above;
copy the exact command in an artifact finding rather than synthesizing one.

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

Maximum size and complexity boundaries are inclusive (`warn` and `fail` trigger
at equality); coverage is an exclusive minimum boundary (equality passes that
threshold). The default coverage collection requests branch evidence with
`--cov-branch`. Line and branch coverage are independently enforced, and a
configured metric with missing or unparseable evidence fails the row; overrides
must preserve evidence for every configured metric.

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
