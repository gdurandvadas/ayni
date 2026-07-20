# Go Adapter

## Installation

Go roots are directories containing `go.mod`; discovery excludes VCS and
`vendor` directories. A repository `go.work` marks a workspace controller, and
the repository root is analyzed only when it contains `go.mod`.

The Go toolchain is a user-owned prerequisite. Ayni uses it for built-in Go
operations and can install `gocyclo` with `go install` when complexity is
enabled and installation is applied.

## Signal Coverage

| Signal | Required tool or method | Version contract |
| --- | --- | --- |
| `test` | `go test` | no version enforced |
| `coverage` | `go test` and `go tool cover` | no version enforced |
| `size` | built-in Go source scan | no version enforced |
| `complexity` | `gocyclo` | no version enforced |
| `deps` | `go list` dependency graph | no version enforced |
| `mutation` | `go test` mutation proxy, or a configured Go mutation command | no version enforced |

Focused verification

`ayni verify test --language go` supports a repository-relative `--file` or Go
package passed through to `go test`, plus an optional `--name` selector. The
name selector is passed as an exact `-run` regular expression.

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
