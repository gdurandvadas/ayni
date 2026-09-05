# Tool baseline compatibility fixtures

Milestone 2 adds real execution evidence for the baseline gaps. These small
fixtures are adapter tests, not additional supported managed environments.
Run them from a temporary copy: mutation tools and Gradle write local state.
The ordinary Rust test suite consumes the captured reports without downloading
or invoking these tools.

| Fixture | Verified versions | Expected evidence |
| --- | --- | --- |
| `adapters/python/tests/fixtures/tooling/mutmut` | CPython 3.12; pytest 9.0.3; mutmut 2.5.1 | Two killed mutants, no survivors; JUnit XML accepted by the Python collector. |
| `adapters/kotlin/tests/fixtures/tooling/gradle` | JDK 21; Gradle 9.5.0 (example wrapper); Kotlin 2.0.20; JaCoCo 0.8.12; PIT Gradle plugin 1.19.0 | One covered line (100%); two killed mutants, no survivors; both XML reports accepted by the Kotlin collectors. |

The mutmut baseline deliberately stays on the 2.x command/report contract:
Ayni calls `run`, then `junitxml --suspicious-policy=failure
--untested-policy=failure`. The [2.5.1 documentation](https://pypi.org/project/mutmut/2.5.1/)
describes that JUnit interface. Upgrading to another report contract requires a
collector change and new execution evidence. These fixtures establish support
on the listed runtimes, not every possible Python/JDK version.

The Kotlin fixture selects `CounterTest` explicitly for PIT. A successful Gradle
process alone is insufficient: assert that mutants were evaluated and killed.
JaCoCo's version is a `toolVersion` on its bundled Gradle plugin, not the version
of an external plugin declaration. JaCoCo 0.8.12's supported Java versions are
recorded in its [release history](https://www.jacoco.org/jacoco/trunk/doc/changes.html).

## Python reproduction

From the repository root, with uv and Python 3.12 available:

```sh
fixture=$(mktemp -d)
cp -R adapters/python/tests/fixtures/tooling/mutmut/. "$fixture/"
uv venv --python 3.12 "$fixture/.venv"
uv pip install --python "$fixture/.venv/bin/python" 'mutmut==2.5.1' 'pytest==9.0.3'
PATH="$fixture/.venv/bin:$PATH" cargo run -p ayni-cli -- verify mutation \
  --host --language python --config "$fixture/.ayni.toml"
```

Expect `killed=2 survived=0 timeout=0`. The temporary project uses the host
collector and the isolated virtual environment; this is not an Ayni-owned uv
reconciliation or managed-image test. `junit.xml` captures the corresponding
mutmut report format for offline parser regression tests.

## Kotlin reproduction

From the repository root, with JDK 21 available:

```sh
fixture=$(mktemp -d)
cp -R adapters/kotlin/tests/fixtures/tooling/gradle/. "$fixture/"
cp examples/kotlin/mono/gradlew "$fixture/gradlew"
mkdir -p "$fixture/gradle"
cp -R examples/kotlin/mono/gradle/wrapper "$fixture/gradle/wrapper"
cargo run -p ayni-cli -- verify coverage --host --language kotlin \
  --config "$fixture/.ayni.toml"
cargo run -p ayni-cli -- verify mutation --host --language kotlin \
  --config "$fixture/.ayni.toml"
```

The wrapper downloads dependencies when absent. Gradle executes only the
fixture's native build configuration in the temporary project. Expect 100% line
coverage and `killed=2 survived=0 timeout=0`. `jacoco.xml` and `mutations.xml` are
captured native outputs; the JaCoCo session identifier and timestamps were
removed to avoid recording host-specific data.

## Other baselines and upgrades

Node and the non-mutation Python baselines match the existing native mono
example declarations. Kotlin's Kover and Detekt baselines also match its mono
example. Conformance tests fail if these fixture declarations drift from the
adapter inventory. Existing collector fixtures exercise their report parsers.
The repository's checkout complexity gate exercises rust-code-analysis-cli
0.0.25; Rust and Go environment tests assert isolated provisioning requirements.
This milestone does not claim fresh managed-image execution for all five mono
examples; that remains part of the final reconciliation rollout.

When changing a baseline, update the owning adapter inventory, its native
fixture declarations, and any captured reports affected by the tool's output.
Run the relevant checkout `verify`, then the repository completion gates.
Do not silently replace a baseline with a latest-version selector or install
signal tools globally.
