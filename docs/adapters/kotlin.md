# Kotlin Adapter

The Kotlin adapter implements the same `LanguageAdapter` and `SignalCollector`
contracts as the existing adapters.

It supports Gradle roots only. Root discovery is intentionally conservative:
the adapter detects the repository root when Gradle markers are present, while
analysis uses configured `[kotlin].roots` entries.

## Module layout

```text
adapters/kotlin/src/
├── adapter.rs
├── catalog.rs
├── discovery.rs
├── install.rs
└── collectors/
```

## Signal mapping

| Signal | Collector | Tooling |
| --- | --- | --- |
| `test` | `collectors/test.rs` | Gradle `test` + JUnit XML under `build/test-results/test` |
| `coverage` | `collectors/coverage.rs` | Gradle `koverXmlReport` + Kover/JaCoCo XML |
| `size` | `collectors/size.rs` | file walk + `[kotlin.size]` glob budgets |
| `complexity` | `collectors/complexity.rs` | Gradle `detekt` + Checkstyle XML |
| `deps` | `collectors/deps.rs` | Gradle `dependencies` project edges |
| `mutation` | `collectors/mutation.rs` | Gradle `pitest` + PIT XML |

## Execution

Kotlin resolves the Gradle runner from the configured root:

1. `./gradlew`
2. `gradlew.bat`
3. `gradle`

`install_cwd` and `exec_cwd` are the configured Kotlin root.

## Policy

Kotlin collectors read:

- `[kotlin]` (`roots = [...]`)
- `[kotlin.size]`
- `[kotlin.complexity]` (`fn_cyclomatic`)
- `[kotlin.coverage]` (`line_percent`)
- `[kotlin.deps.forbidden]`
- optional `[kotlin.tooling.test]`, `[kotlin.tooling.coverage]`, `[kotlin.tooling.mutation]`

Example:

```toml
[languages]
enabled = ["kotlin"]

[kotlin]
roots = ["."]

[kotlin.size]
"**/*.kt" = { warn = 400, fail = 800, exclude = ["build/**", ".gradle/**", ".git/**", ".ayni/**"] }
"**/*.kts" = { warn = 400, fail = 800, exclude = ["build/**", ".gradle/**", ".git/**", ".ayni/**"] }

[kotlin.complexity]
fn_cyclomatic = { warn = 10, fail = 20 }

[kotlin.coverage]
line_percent = { warn = 70, fail = 50 }

[kotlin.deps.forbidden]
"apps/api" = ["libs/ui"]
```

## Install

`ayni install --apply --language kotlin` updates direct Gradle `plugins { }`
blocks in `build.gradle.kts` or `build.gradle` when supported.

It adds:

- `org.jetbrains.kotlinx.kover` `0.9.8`
- `io.gitlab.arturbosch.detekt` `1.23.8`
- `info.solidsoft.pitest` `1.19.0`

Unsupported build shapes fail with a setup error instead of being rewritten.
