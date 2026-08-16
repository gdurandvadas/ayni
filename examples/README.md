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

The `mono/` fixtures include `.ayni.toml`. Use them with their own locked
managed environment when the fixture has the required native lock inputs, or
prepare the documented host prerequisites before using the explicit escape
hatch. For example, after installing the Go toolchain and `gocyclo`:

```sh
ayni check --host --config examples/go/mono/.ayni.toml
```

CI prepares every host tool explicitly; Ayni does not install tools as a side
effect of `check`. Pull requests expose one workflow per adapter:
`ayni-go`, `ayni-node`, `ayni-python`, and `ayni-kotlin` validate the matching
`mono/` fixture, while `ayni-rust` validates the Ayni repository itself as the
Rust target. Python's pinned host-only CI tools live with its fixture in
`examples/python/mono/requirements-ci.txt`.

The `single/` fixtures intentionally omit Ayni configuration so they can be
used as raw language examples. Use `ayni agents sync --repo-root <path>` only
when a fixture needs the managed guidance block.
