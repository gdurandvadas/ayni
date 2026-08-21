# Kotlin Adapter

## Installation

Kotlin supports Gradle projects only. The repository root is detected when it
contains `build.gradle.kts`, `build.gradle`, `settings.gradle.kts`, or
`settings.gradle`; configure analysis roots in `[kotlin].roots`. The Gradle
runner precedence is `./gradlew`, `gradlew.bat`, then `gradle` on `PATH`.

Managed Kotlin support requires a repository-owned POSIX Gradle wrapper
(`gradlew`, wrapper JAR, properties, exact official distribution URL), a
repository JDK requirement, build/settings files, and committed Gradle
dependency locks. Ayni discovers `.java-version`, `.tool-versions`, and common
Gradle JVM toolchain declarations separately from the wrapper version. Conflicts
fail rather than selecting an arbitrary JDK.

`env build` stages only digest-checked Gradle metadata and uses a generated init
script to resolve locked configurations into `GRADLE_USER_HOME`; it does not
copy source or edit build files. Managed Gradle commands use the locked JDK,
set `JAVA_HOME`, and add `--offline --no-daemon`. Coverage, complexity, and
mutation require exact repository plugin declarations for Kover/JaCoCo,
Detekt, and PIT respectively. The Gradle runner, JDK, and plugins remain
user-owned prerequisites for `--host` execution. Composite builds, dynamic
plugin versions, Android SDK management, missing dependency locks, and private
repositories requiring undeclared credentials are not supported by the first
managed slice.

## Signal Coverage

| Signal | Required tool or method | Version contract |
| --- | --- | --- |
| `test` | Gradle `test` task and JUnit XML | managed: exact wrapper and JDK; host: no version enforced |
| `coverage` | Gradle `koverXmlReport` or `jacocoTestReport` | managed: exact wrapper/JDK and exact Kover/JaCoCo declaration; host: no version enforced |
| `size` | built-in Kotlin source scan | no external tool |
| `complexity` | Gradle `detekt` task | managed: exact wrapper/JDK and exact Detekt plugin; host: no version enforced |
| `deps` | Gradle `dependencies` project edges | managed: exact wrapper and JDK; host: no version enforced |
| `mutation` | Gradle `pitest` task (opt-in) | managed: exact wrapper/JDK and exact PIT plugin; host: no version enforced |

## Focused verification

Shared artifact, completion, validation, and exact-command reuse semantics are
defined in [Completion and focused verification](../product/runtime.md#completion-and-focused-verification).
The accepted Kotlin selectors are:

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
`ayni verify test --host --config './.ayni.toml' --language kotlin --root '.' --package
'com.example.ApiTest' --name 'createsUser'`.

## Impact planning

Kotlin impact mapping currently treats every changed input below the configured
root as relevant, along with governing ancestor Gradle build/settings, lock,
wrapper executable/JAR, version-catalog, dependency-lock, and JDK runtime inputs. Because Gradle project topology is not yet
used for narrowing, every enabled signal broadens to the configured Kotlin root
and records a `missing_topology` uncertainty. This is intentionally conservative
and does not replace the final unscoped `ayni check`.

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

Shared boundary rules are defined under [Threshold semantics](../product/config.md#threshold-semantics).
Kotlin line and branch coverage are independently enforced: a configured metric
with missing or unparseable evidence fails the coverage row.

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
