# Signal Contract

This is the stable entry point for Ayni's canonical signal vocabulary. It does
not define a single JSON run-artifact envelope: select the versioned contract
named by an artifact's `schema_version` before reading envelope or row fields.

- [Schema v3 (`0.3.0`)](signals/v3.md) is the current emitted contract.
- [Schema v2 (`0.2.0`)](signals/v2.md) is a historical reference only.
- [Schema v1 (`0.1.0`)](signals/v1.md) is a historical reference only.

Repository policy lives in `.ayni.toml`; for checks, languages, thresholds, and
excluding paths (for example, skipping `target/**` in the size signal), see the
[Configuration reference](config.md). For command failure categories and runtime
diagnostics, see [Runtime and setup rules](runtime.md).

## Canonical vocabulary

All versions document rows for this closed vocabulary. New adapters must emit
only these `kind` values.

| `kind` | Purpose |
| --- | --- |
| `test` | Test execution outcome |
| `coverage` | Coverage quality |
| `size` | File or module size budgets |
| `complexity` | Function complexity budgets |
| `deps` | Architectural dependency constraints |
| `mutation` | Test-suite fault-detection strength |

The currently serialized language values are `rust`, `go`, `node`, `python`,
and `kotlin`. A row scope identifies a measurement target with a workspace root
and optional path, package, and file. Exact serialized row fields, optionality,
and payload shapes are version-specific; use the selected version reference.

## Version selection and compatibility

`ayni check` and the explicit `ayni check --host` path both write
`.ayni/last/signals.json`; adding `--output json` prints the same artifact.
Current output uses
schema `0.3.0`. Consumers must inspect `schema_version` and use the matching
version page rather than assuming fields from another envelope.

Schema v3 is a breaking replacement for v2 consumers. Explicit artifact
comparison accepts only two valid, complete artifacts whose `schema_version`
is the current schema string. Use `ayni results compare --baseline <artifact>
--candidate <artifact> [--output human|json]`; it reads exactly those files
and has no implicit prior-artifact, repository, Git, fetch, storage, or write
behavior. There is no compatibility payload or automatic conversion from an
earlier schema.

Impact planning and execution use a separate versioned envelope because impact
evidence records Git changes, inclusion reasons, uncertainty, and selected-job
accounting rather than repository completion. It may contain the same typed
signal rows, but it is not a schema-v3 `RunArtifact`, cannot replace
`.ayni/last/signals.json`, and is not accepted by `results compare`. See the
[impact execution contract](impact.md).

V1 and v2 are retained only as historical documentation. Ayni makes no current
parsing, conversion, migration, or compatibility promise for either schema.

## Vocabulary evolution

Existing signal names and documented semantics are the canonical vocabulary.
When an envelope changes, publish its field contract under a new version page
instead of changing this index to describe that envelope. Unknown or
adapter-specific detail belongs in explicitly named extension sub-objects, not
free-form top-level keys.
