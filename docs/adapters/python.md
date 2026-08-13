# Python Adapter

## Installation

Repository initialization and managed tool provisioning are not implemented in
the clean-slate command model yet. Configure roots in `.ayni.toml` and provide
the documented runtime, package manager, and signal tools when using `--host`.

## Signal Coverage

| Signal | Required tool or method | Version contract |
| --- | --- | --- |
| `test` | `pytest`; `pytest-json-report` | no version enforced |
| `coverage` | `pytest`; `pytest-cov`; `coverage` | no version enforced |
| `size` | built-in Python source scan | no version enforced |
| `complexity` | `complexipy` | no version enforced |
| `deps` | Python import scan | no version enforced |
| `mutation` | `mutmut` (opt-in) | no version enforced |

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
`ayni verify test --config './.ayni.toml' --language python --root '.' --file
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
