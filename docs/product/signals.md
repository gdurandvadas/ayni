# Signals

A signal is a normalized quality measurement, not just a shell command. The
language adapter chooses and runs ecosystem-specific tooling; Ayni parses the
result into common evidence and evaluates it against `.ayni.toml`. One physical
tool execution may provide more than one signal only when the adapter can parse
complete independent evidence for every emitted row.

## Signals, policy, and execution

The three parts of a signal run remain explicit:

```text
Signal policy             Managed tools             Requested scope
.ayni.toml            +   .ayni.lock / OCI image +  check / verify / impact
thresholds and rules      exact execution            measured evidence
```

Enabling a signal contributes its required analysis tools to the managed
environment plan. After that environment is locked and built, `ayni check`,
`ayni verify <signal>`, and `ayni impact run` launch it automatically. Use
`--host` only as an explicit escape hatch; do not wrap quality commands in
`ayni env run`.

See [How Ayni works](/getting-started/how-ayni-works) for the complete mental
model and [Managed environments](/product/environments) for provisioning.

## Canonical vocabulary

All schema versions document rows from this closed vocabulary. New adapters
must emit only these `kind` values.

| `kind` | Question answered |
| --- | --- |
| `test` | Did expected behavior pass? |
| `coverage` | How much code was exercised? |
| `size` | Which files or modules exceeded their budgets? |
| `complexity` | Which functions exceeded structural budgets? |
| `deps` | Did source dependencies respect architectural constraints? |
| `mutation` | Did the test suite detect injected behavioral changes? |

The currently serialized language values are `rust`, `go`, `node`, `python`,
and `kotlin`. A row scope identifies a measurement target with a workspace root
and optional path, package, and file.

Repository policy lives in `.ayni.toml`; for enabled checks, languages,
thresholds, and excluded paths, see the [Configuration reference](/product/config).
For command failure categories and runtime diagnostics, see [Runtime and
verification](/product/runtime).

## Signal contract versions

This page is the stable entry point for Ayni's canonical vocabulary. It does
not define a single JSON run-artifact envelope: select the versioned contract
named by an artifact's `schema_version` before reading envelope or row fields.

[Schema v4 (`0.4.0`)](/product/signals/v4) is the current, self-contained
emitted contract and the only contract in primary documentation navigation.
Exact serialized row fields, optionality, provenance, and payload shapes live
there.

Schemas v1 through v3 remain archived beside v4 for consumers of old saved artifacts.
They are not current navigation targets and carry no parsing, conversion, or
compatibility promise.

## Version selection and compatibility

`ayni check` and the explicit `ayni check --host` path both write
`.ayni/last/signals.json`; adding `--output json` prints the same artifact.
Current output uses schema `0.4.0`. Consumers must inspect `schema_version` and
use the matching version page rather than assuming fields from another
envelope.

Schema v4 is a breaking replacement for v3 consumers. Explicit artifact
comparison accepts only two valid, complete artifacts whose `schema_version`
is the current schema string. Use `ayni results compare --baseline <artifact>
--candidate <artifact> [--output human|json]`; it reads exactly those files and
has no implicit prior-artifact, repository, Git, fetch, storage, or write
behavior. There is no compatibility payload or automatic conversion from an
earlier schema.

Impact planning and execution use a separate versioned envelope because impact
evidence records Git changes, inclusion reasons, uncertainty, and selected-job
accounting rather than repository completion. It may contain the same typed
signal rows, but it is not a schema-v4 `RunArtifact`, cannot replace
`.ayni/last/signals.json`, and is not accepted by `results compare`. See the
[impact execution contract](/product/impact).

V1, v2, and v3 are retained only as historical documentation. Ayni makes no
current parsing, conversion, migration, or compatibility promise for them.

## Vocabulary evolution

Existing signal names and documented semantics are the canonical vocabulary.
When an envelope changes, publish its field contract under a new version page
instead of changing this index to describe that envelope. Unknown or
adapter-specific detail belongs in explicitly named extension sub-objects, not
free-form top-level keys.
