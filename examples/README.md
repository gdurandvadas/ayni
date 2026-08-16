# Example fixtures for adapter and signal testing

Each language has:

- `single/`: one service repository **without** Ayni files preinstalled. Contains intentional signal failures.
- `mono/`: monorepo with `math` lib, `greeting` lib, and `greeting-service` app. Contains dependency/policy scenarios.

Layout:

- `examples/<language>/single`
- `examples/<language>/mono`

The `math` library exports 10 functions and includes tests for 8/10 to make coverage intentionally incomplete.

Kotlin examples use Gradle Kotlin DSL. `single/` is intentionally missing Ayni
files; `mono/` includes `.ayni.toml` and Gradle quality plugins.

The `mono/` fixtures include `.ayni.toml` and can be exercised with the
explicit host workflow, for example:

```sh
ayni check --host --config examples/go/mono/.ayni.toml
```

The `single/` fixtures intentionally omit Ayni configuration so they can be
used as raw language examples. Use `ayni agents sync --repo-root <path>` only
when a fixture needs the managed guidance block.
