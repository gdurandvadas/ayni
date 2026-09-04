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
| `size` | built-in Python source scan | no external tool |
| `complexity` | `complexipy` through the resolved package manager | managed: exact `uv.lock`; host: no version enforced |
| `deps` | built-in Python import scan | no external tool |
| `mutation` | `mutmut` (opt-in) | managed: exact `uv.lock`; host: no version enforced |

## Focused verification

Shared artifact, completion, validation, and exact-command reuse semantics are
defined in [Completion and focused verification](../product/runtime.md#completion-and-focused-verification).
The accepted Python selectors are:

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
'tests/test_api.py' --name 'test_create'`.

## Impact planning

Python impact mapping currently treats every changed input below the configured
root as relevant, along with governing ancestor project, package-manager lock,
requirements, and runtime inputs. Because package topology is not yet used for narrowing, every enabled
signal broadens to the configured Python root and records a `missing_topology`
uncertainty. This is intentionally conservative and does not replace the final
unscoped `ayni check`.

## Contract

Enabled checks come from `[checks]`. Configure roots in `[python].roots`
(default `["."]`), size budgets in `[python.size]`, cognitive complexity in `[python.complexity]`,
coverage in `[python.coverage]`, and forbidden edges in
`[python.deps.forbidden]`. Command overrides are optional in
`[python.tooling.test]`, `[python.tooling.coverage]`, and
`[python.tooling.mutation]`; each override requires `command` and may set `args`.
With both signals enabled, `[python.tooling].coverage_satisfies_test = true`
opts repository `check` into one pytest execution that emits both the
pytest-json-report test artifact and pytest-cov coverage JSON. A coverage
command with omitted arguments receives both Ayni-owned report paths. Explicit
arguments are an attestation that the command runs the complete suite and writes
both report formats to those canonical `.ayni/work/python/<root-slug>/` paths.
Missing either report, or malformed evidence in either report, fails both rows
closed.

Size requires a budget entry and complexity requires `fn_cognitive`; either
missing value produces a clear collector error. Coverage thresholds and
dependency rules are optional: without `line_percent`, coverage has no policy
threshold, and without `python.deps.forbidden`, no edges are forbidden.

Shared boundary rules are defined under [Threshold semantics](../product/config.md#threshold-semantics).
The default Python coverage collection requests branch evidence with
`--cov-branch`. Line and branch coverage are independently enforced, and a
configured metric with missing or unparseable evidence fails the row; overrides
must preserve evidence for every configured metric.

## Configuration Example

```toml
[languages]
enabled = ["python"]

[python]
roots = ["."]

[python.tooling]
coverage_satisfies_test = true

[python.size]
"**/*.py" = { warn = 400, fail = 800, exclude = [".venv/**", "venv/**", "__pycache__/**", ".git/**", ".ayni/**"] }

[python.complexity]
fn_cognitive = { warn = 10, fail = 15 }

[python.coverage]
line_percent = { warn = 80, fail = 60 }

[python.deps.forbidden]
"src/domain/**" = ["src/presentation/**"]
```
