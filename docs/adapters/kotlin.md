# Kotlin Adapter

## Installation

Kotlin supports Gradle projects only. The repository root is detected when it
contains `build.gradle.kts`, `build.gradle`, `settings.gradle.kts`, or
`settings.gradle`; configure analysis roots in `[kotlin].roots`. The Gradle
runner precedence is `./gradlew`, `gradlew.bat`, then `gradle` on `PATH`.

The Gradle runner and JDK are user-owned prerequisites. Applied installation
can add missing plugins only to supported direct `plugins { }` blocks in
`build.gradle.kts` or `build.gradle`; unsupported build shapes report setup
errors. Existing JaCoCo coverage is retained; otherwise installation adds Kover.

## Signal Coverage

| Signal | Required tool or method | Version contract |
| --- | --- | --- |
| `test` | Gradle `test` task and JUnit XML | no version enforced |
| `coverage` | Gradle `koverXmlReport` or `jacocoTestReport` | Kover 0.9.8 when Ayni adds it; JaCoCo: no version enforced |
| `size` | built-in Kotlin source scan | no version enforced |
| `complexity` | Gradle `detekt` task | Detekt 1.23.8 when Ayni adds it |
| `deps` | Gradle `dependencies` project edges | no version enforced |
| `mutation` | Gradle `pitest` task (opt-in) | PIT plugin 1.19.0 when Ayni adds it |

Focused verification

`ayni verify test --language kotlin` supports a Gradle test class/package via
`--package` and an optional method selector via `--name`; these become a Gradle
`--tests` pattern. Kotlin source-file selection with `--file` is rejected
because Gradle test filters operate on test class names.

## Contract

Enabled checks come from `[checks]`. Configure roots in `[kotlin].roots`
(default `["."]`), size budgets in `[kotlin.size]`, complexity in
`[kotlin.complexity]`, coverage in `[kotlin.coverage]`, and forbidden edges in
`[kotlin.deps.forbidden]`. Command overrides are optional in
`[kotlin.tooling.test]`, `[kotlin.tooling.coverage]`, and
`[kotlin.tooling.mutation]`; each override requires `command` and may set `args`.

Size requires a budget entry and complexity requires `fn_cyclomatic`; either
missing value produces a clear collector error. Coverage thresholds and
dependency rules are optional: without `line_percent`, coverage has no policy
threshold, and without `kotlin.deps.forbidden`, no edges are forbidden.

## Configuration Example

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
