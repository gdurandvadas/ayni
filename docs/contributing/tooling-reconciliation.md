# Signal-tool inspection

`ayni tools reconcile` inspects the signal tools required by `.ayni.toml` across
Rust, npm/pnpm Node, Go modules, uv Python, and locked Gradle Kotlin. It reports
native declarations, resolved versions, adapter baselines, and setup problems.
It is read-only: fix native metadata with your package manager, then run
`ayni env lock` and `ayni env build`.

```sh
ayni tools reconcile
ayni tools reconcile --check --output json
```

Use `--repo-root` and `--config` for another repository. Preview returns 0 even
when diagnostics require action; `--check` returns 1 for missing or incompatible
metadata or a stale/invalid existing Ayni lock. Invalid policy or escaping roots
return 2. Inspection creates no files and runs no package-manager commands.

## Version selection

Project declarations and native locks choose Node, Python, and Kotlin tool
versions. Adapter baselines are compatibility information, not mandatory
upgrades. Rust and Go isolated tools use exact adapter baselines; runtime
components follow their toolchain. Custom command overrides exclude the default
tools for the overridden signal.

Each adapter owns its `ManagedToolSpec` inventory. The catalog owns signal
mappings; core checks completeness and selects only enabled default tools.
`ToolVersionAuthority` records version selection independently of the source
path. Environment planning and locking preserve this authority.

Node checks npm/pnpm declarations against locked versions. Python checks uv
resolutions against native requirements and reports ambiguous versions or
unsupported requirement syntax. Kotlin preserves the declared Kover or JaCoCo
provider and checks direct plugin versions, dependency locks, and verification
metadata. Plugin aliases require manual inspection. Rust and Go inspection
reports isolated tool requirements without claiming they are installed on the
host. See each [adapter guide](/adapters/rust) for supported project shapes.

## Result contract

JSON projection `0.2.0` contains normalized targets, governing roots, tool
requirements, current declarations/resolutions, diagnostics, and environment
lock state. Targets and diagnostics are deterministic. The removed `ownership`,
`inputs`, `edits`, `commands`, and `outputs` fields described unimplemented
behavior and are no longer serialized. Contract projection `0.6.0` likewise
removes `environment.signal_tool_ownership`. Remove the former
`[environment.signal_tools]` policy table; unknown settings fail validation.

A missing `.ayni.lock` is reported as `absent`, not tooling drift. Existing locks
are checked against policy and recorded native-input digests.
`recorded_inputs_match` means those inputs match; run `env doctor` to validate
environment readiness. `refresh_after_reconciliation` means native setup needs
repair even though the recorded inputs currently match.

`ToolingRequest` carries an absolute repository root, normalized target, enabled
signals, and the subset using default tools. The caller checks filesystem
containment. `LanguageAdapter::plan_tooling` validates capability language,
target identity, governing root, requirements, and diagnostic paths. There is no
mutation plan, staging protocol, or apply executor.
