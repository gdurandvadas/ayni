# Example fixtures for adapter and signal testing

Each language has:

- `single/`: one service repository **without** Ayni files preinstalled. Contains intentional signal failures.
- `mono/`: monorepo with `math` lib, `greeting` lib, and `greeting-service` app. Contains dependency/policy scenarios.

Layout:

- `examples/<language>/single`
- `examples/<language>/mono`

The `math` library exports 10 functions and includes tests for 8/10 to make coverage intentionally incomplete.
