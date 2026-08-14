# Go Adapter

## Installation

Go roots are directories containing `go.mod`; discovery excludes VCS and
`vendor` directories. A repository `go.work` marks a workspace controller, and
the repository root is analyzed only when it contains `go.mod`.

The Go toolchain and `gocyclo` are user-owned prerequisites for `--host`
execution. Go environment discovery and locking are not implemented yet, so
`env show` and `env lock` fail explicitly for configured Go targets rather than
installing tools or changing the checkout.

## Signal Coverage

| Signal | Required tool or method | Version contract |
| --- | --- | --- |
| `test` | `go test` | no version enforced |
| `coverage` | `go test` and `go tool cover` | no version enforced |
| `size` | built-in Go source scan | no version enforced |
| `complexity` | `gocyclo` | no version enforced |
| `deps` | `go list` dependency graph | no version enforced |
| `mutation` | `go test` mutation proxy, or a configured Go mutation command | no version enforced |

## Focused verification

`verify` writes requested-scope evidence only to `.ayni/verify/last/signals.json`.
Every command accepts an optional `--language go`; unscoped verification is
always valid. The accepted selectors are:

| Signal | `--file` | `--package` | `--name` |
| --- | --- | --- | --- |
| `test` | no | yes | yes |
| `coverage` | no | no | no |
| `size` | yes | no | no |
| `complexity` | yes | no | no |
| `deps` | yes | yes | no |
| `mutation` | no | no | no |

For `test`, pass a Go package and optional name; the name becomes an exact
`-run` regular expression. `--name` is test-only, and `--file` cannot be
combined with `--package`. Unsupported or ambiguous selectors are rejected
before `go` runs.

Verification commands carry their originating contract and target, for example:
`ayni verify test --host --config './.ayni.toml' --language go --root 'services/api'
--package './internal/api' --name 'TestCreate'`. Use only the selectors marked
above; copy the exact command in an artifact finding rather than synthesizing one.

## Contract

Enabled checks come from `[checks]`. Configure Go roots in `[go].roots`
(default `["."]`), size budgets in `[go.size]`, the cyclomatic threshold in
`[go.complexity]`, coverage in `[go.coverage]`, and forbidden edges in
`[go.deps.forbidden]`. Command overrides are optional in `[go.tooling.test]`,
`[go.tooling.coverage]`, and `[go.tooling.mutation]`; each override requires
`command` and may set `args`.

Size requires a budget entry and complexity requires `fn_cyclomatic`; either
missing value produces a clear collector error. Coverage thresholds and
dependency rules are optional: without `line_percent`, coverage has no policy
threshold, and without `go.deps.forbidden`, no edges are forbidden.

Maximum size and complexity boundaries are inclusive (`warn` and `fail` trigger
at equality); coverage is an exclusive minimum boundary (equality passes that
threshold). Line and branch coverage are independent and configured evidence
fails closed when it is missing or unparseable. Standard Go coverage profiles
provide statement coverage only: configuring `branch_percent` therefore fails
the coverage row rather than reinterpreting statement coverage as branches.

## Configuration Example

```toml
[languages]
enabled = ["go"]

[go]
roots = ["services/api", "services/worker"]

[go.tooling.coverage]
command = "go"
args = ["test", "./...", "-coverprofile=.ayni/go.cover.out"]

[go.size]
"**/*.go" = { warn = 300, fail = 600, exclude = ["vendor/**", ".git/**", ".ayni/**"] }

[go.complexity]
fn_cyclomatic = { warn = 10, fail = 20 }

[go.coverage]
line_percent = { warn = 70, fail = 50 }

[go.deps.forbidden]
"internal/domain/**" = ["internal/http/**"]
```
