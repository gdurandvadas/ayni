# Go Adapter

The Go adapter implements the same `LanguageAdapter` and `SignalCollector` contracts as the Rust and Node adapters.

It detects Go module roots, resolves required tooling from a typed catalog, and emits canonical `SignalRow` values for each enabled `SignalKind`.

## Module layout

```text
adapters/go/src/
├── lib.rs
├── adapter.rs
├── catalog.rs
└── collectors/
    ├── mod.rs
    ├── util.rs
    ├── test.rs
    ├── coverage.rs
    ├── size.rs
    ├── complexity.rs
    ├── deps.rs
    └── mutation.rs
```

## Signal coverage

- `test`: `collectors/test.rs` using `go test ./... -json`
- `coverage`: `collectors/coverage.rs` using `go test -coverprofile` + `go tool cover -func`
- `size`: `collectors/size.rs` using file walk + `[go.size]` glob budgets
- `complexity`: `collectors/complexity.rs` using `gocyclo`
- `deps`: `collectors/deps.rs` using `go list -json ./...` + forbidden edge rules
- `mutation`: `collectors/mutation.rs` using proxy mutation flow (tooling override aware)

Every collector outputs:

- a canonical `SignalResult` variant
- a matching typed offender list
- policy-aware `pass` evaluation

## Detection and roots

- A root is considered Go when `go.mod` is present.
- `ayni analyze` runs Go collection only for configured and detected `[go].roots`.
- The default file profile for size checks is Go files (`*.go`) unless narrowed by policy.

## Tool catalog

Go tools are declared in `catalog.rs` using `CatalogEntry` + `Installer`:

- `Installer::Bundled` for `go`
- `Installer::GoInstall` for `gocyclo`

Each entry declares `for_signals`, so `ayni install` can derive required tools from enabled checks.

## Policy expectations

Go collectors read these `.ayni.toml` sections:

- `[checks]`
- `[go]` (`roots = [...]`)
- `[go.size]` (glob budgets as `glob = { warn, fail, exclude? }`)
- `[go.complexity]` (`fn_cyclomatic`)
- `[go.coverage]` (`line_percent`)
- `[go.deps.forbidden]` (forbidden dependency edges)
- optional `[go.tooling.test]`, `[go.tooling.coverage]`, `[go.tooling.mutation]` command overrides

If required policy fields are missing, collectors return explicit errors.

## Full TOML example

```toml
[languages]
enabled = ["go"]

[go]
roots = ["services/api", "services/worker"]

[go.tooling.test]
command = "go"
args = ["test", "./...", "-json"]

[go.tooling.coverage]
command = "go"
args = ["test", "./...", "-coverprofile=.ayni/go.cover.out"]

[go.tooling.mutation]
command = "go"
args = ["test", "./..."]

[go.size]
"**/*.go" = { warn = 300, fail = 600, exclude = ["vendor/**", ".git/**", ".ayni/**"] }

[go.complexity]
fn_cyclomatic = { warn = 10, fail = 20 }

[go.coverage]
line_percent = { warn = 70, fail = 50 }

[go.deps.forbidden]
"internal/domain/**" = ["internal/http/**"]
"internal/core/**" = ["internal/infra/**"]
```

## `ayni install --language go`

`--language go` scopes installation to Go catalog entries only.

```bash
ayni install --language go --repo-root <path>
```

The install flow:

1. Load policy and detect configured Go roots.
2. Evaluate each Go catalog entry against enabled checks.
3. Probe tool status (present/outdated/missing).
4. Optionally install missing tools when `--apply` is passed.

The flow is deterministic and idempotent.

## `ayni analyze --language go`

`--language go` limits analysis planning to Go roots and Go collectors.

```bash
ayni analyze --config ./.ayni.toml --language go --output stdout
```

Use `--output md` for markdown summary output:

```bash
ayni analyze --config ./.ayni.toml --language go --output md
```

## Output guarantees

The adapter emits only core-defined signal kinds and typed payloads. It must never emit ad-hoc row shapes, so downstream reporting stays stable across all languages.
