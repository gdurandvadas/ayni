# Go Adapter

## Installation

Go roots are directories containing `go.mod`; discovery excludes VCS and
`vendor` directories. A repository `go.work` marks a workspace controller, and
the repository root is analyzed only when it contains `go.mod`.

For managed execution, Ayni discovers `.go-version`, `.tool-versions`, `go`
and `toolchain` directives, validates `go.work` membership, and locks an exact
Go runtime. Modules with declared dependencies require a committed `go.sum`.
`env build` runs `go mod download all` only against staged, digest-checked
manifests, stores module/build data below the managed cache, disables Go's own
toolchain downloads, keeps cached modules writable so generated environment
state remains removable, and runs quality commands with read-only module
metadata. Container networking is disabled unless bridge networking is locked
and the operator authorizes that invocation. Complexity additionally provisions
pinned `gocyclo` `0.6.0` through its Go module provider.

The Go toolchain and `gocyclo` remain user-owned prerequisites for `--host`
execution. Managed support does not cover module-less GOPATH projects, external
local replacements, private registries requiring undeclared credentials, or
cgo system libraries absent from the base image.

## Signal Coverage

| Signal | Required tool or method | Version contract |
| --- | --- | --- |
| `test` | `go test` | managed: exact locked Go runtime; host: no version enforced |
| `coverage` | `go test` and `go tool cover` | managed: exact locked Go runtime; host: no version enforced |
| `size` | built-in Go source scan | no external tool |
| `complexity` | `gocyclo` | managed: `0.6.0`; host: no version enforced |
| `deps` | `go list` dependency graph | managed: exact locked Go runtime; host: no version enforced |
| `mutation` | unsupported | Go mutation measurement is not supported; enabling it fails explicitly before tool execution |

## Focused verification

Shared artifact, completion, validation, and exact-command reuse semantics are
defined in [Completion and focused verification](../product/runtime.md#completion-and-focused-verification).
The accepted Go selectors are:

| Signal | `--file` | `--package` | `--name` |
| --- | --- | --- | --- |
| `test` | no | yes | yes |
| `coverage` | no | no | no |
| `size` | yes | no | no |
| `complexity` | yes | no | no |
| `deps` | yes | yes | no |
| `mutation` | unsupported | unsupported | unsupported |

For `test`, pass a Go package and optional name; the name becomes an exact
`-run` regular expression. `--name` is test-only, and `--file` cannot be
combined with `--package`. Unsupported or ambiguous selectors are rejected
before `go` runs.

Verification commands carry their originating contract and target, for example:
`ayni verify test --host --config './.ayni.toml' --language go --root 'services/api'
--package './internal/api' --name 'TestCreate'`.

## Impact planning

Go impact mapping currently treats every changed input below the configured root
as relevant, along with governing ancestor module/workspace, `go.work.sum`, and
runtime markers. Because package topology
is not yet used for narrowing, every enabled signal broadens to the configured
Go root and records a `missing_topology` uncertainty. This is intentionally
conservative and does not replace the final unscoped `ayni check`.

## Contract

Enabled checks come from `[checks]`. Configure Go roots in `[go].roots`
(default `["."]`), size budgets in `[go.size]`, the cyclomatic threshold in
`[go.complexity]`, coverage in `[go.coverage]`, and forbidden edges in
`[go.deps.forbidden]`. Command overrides are optional in `[go.tooling.test]`,
`[go.tooling.coverage]`; each override requires `command` and may set `args`.
With both signals enabled, `[go.tooling].coverage_satisfies_test = true`
opts repository `check` into one `go test` execution that emits `-json` test
events and a coverage profile. Ayni then parses that profile with `go tool cover`
without rerunning tests and emits independent test and coverage rows. Ayni adds
`-json` when needed and replaces any custom `-coverprofile` destination with a
fresh managed path. An explicit coverage command is an attestation that it runs
the complete suite and preserves both evidence formats. Missing either evidence
type fails both rows closed. Go
mutation is unsupported, including command overrides.

Size requires a budget entry and complexity requires `fn_cyclomatic`; either
missing value produces a clear collector error. Coverage thresholds and
dependency rules are optional: without `line_percent`, coverage has no policy
threshold, and without `go.deps.forbidden`, no edges are forbidden.

Shared boundary rules are defined under [Threshold semantics](../product/config.md#threshold-semantics).
Configured Go coverage evidence fails closed when it is missing or unparseable.
Standard Go coverage profiles provide statement coverage only: configuring
`branch_percent` therefore fails the coverage row rather than reinterpreting
statement coverage as branches.

## Configuration Example

```toml
[languages]
enabled = ["go"]

[go]
roots = ["services/api", "services/worker"]

[go.tooling]
coverage_satisfies_test = true

[go.tooling.coverage]
command = "go"
args = ["test", "./...", "-json"]

[go.size]
"**/*.go" = { warn = 300, fail = 600, exclude = ["vendor/**", ".git/**", ".ayni/**"] }

[go.complexity]
fn_cyclomatic = { warn = 10, fail = 20 }

[go.coverage]
line_percent = { warn = 70, fail = 50 }

[go.deps.forbidden]
"internal/domain/**" = ["internal/http/**"]
```
