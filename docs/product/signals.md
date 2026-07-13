# Signal Contract

This is the stable entry point for Ayni's canonical signal vocabulary. It does
not define a single JSON run-artifact envelope: select the versioned contract
named by an artifact's `schema_version` before reading envelope or row fields.

- [Schema v2 (`0.2.0`)](signals/v2.md) is the current emitted contract.
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

`ayni analyze` writes `.ayni/last/signals.json`; `ayni analyze --json` and
`ayni analyze --output json` print the same artifact. Current output uses
schema `0.2.0`. Consumers must inspect `schema_version` and use the matching
version page rather than assuming fields from another envelope.

Schema v2 is a breaking replacement for v1 consumers. Current delta loading
only uses a previously stored artifact when its `schema_version` equals the
current schema string. There is no compatibility payload or automatic v1-to-v2
conversion.

V1 is retained only as documentation of the pinned historical source. Ayni
makes no current v1 parsing, conversion, migration, or compatibility promise.

## Vocabulary evolution

Existing signal names and documented semantics are the canonical vocabulary.
When an envelope changes, publish its field contract under a new version page
instead of changing this index to describe that envelope. Unknown or
adapter-specific detail belongs in explicitly named extension sub-objects, not
free-form top-level keys.
