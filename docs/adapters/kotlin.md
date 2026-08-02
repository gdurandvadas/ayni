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

## Focused verification

`verify` writes requested-scope evidence only to `.ayni/verify/last/signals.json`.
Every command accepts an optional `--language kotlin`; unscoped verification is
always valid. The accepted selectors are:

| Signal | `--file` | `--package` | `--name` |
| --- | --- | --- | --- |
| `test` | no | yes | yes |
| `coverage` | no | no | no |
| `size` | yes | no | no |
| `complexity` | yes | no | no |
| `deps` | no | no | no |
| `mutation` | no | no | no |

For `test`, a Gradle test class/package and optional method become a Gradle
`--tests` pattern. Kotlin source-file selection is unsupported for tests because
Gradle filters operate on test class names. `--name` is test-only, and `--file`
cannot be combined with `--package`; unsupported or ambiguous selectors are
rejected before Gradle runs.

Verification commands carry their originating contract and target, for example:
`ayni verify test --config './.ayni.toml' --language kotlin --root '.' --package
'com.example.ApiTest' --name 'createsUser'`. Use only the selectors marked
above; copy the exact command in an artifact finding rather than synthesizing one.

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

Maximum size and complexity boundaries are inclusive (`warn` and `fail` trigger
at equality); coverage is an exclusive minimum boundary (equality passes that
threshold). Line and branch coverage are independently enforced: a configured
metric with missing or unparseable evidence fails the coverage row.

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
