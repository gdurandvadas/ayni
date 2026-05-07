# Signal Contract

This document defines the canonical Ayni signal vocabulary. It is the product-level contract used by adapters, the CLI, and AI consumers.

Repository policy lives in `.ayni.toml`; for checks, languages, thresholds, and **excluding paths** (for example skipping `target/**` in the size signal), see **[Configuration reference](config.md)**.

## Common row shape

Every signal row includes:

- `kind`: one of `test`, `coverage`, `size`, `complexity`, `deps`, `mutation`
- `language`: one of `rust`, `go`, `node`, `python` (expandable)
- `scope`: measurement target (`workspace`, `path`, optional `package`, optional `file`)
- `pass`: whether the row is within policy budget
- `result`: typed payload for the signal kind
- `budget`: typed threshold payload for the signal kind
- `offenders`: typed list of violations
- `delta_vs_previous`: optional change vs previous local run
- `delta_vs_baseline`: optional change vs baseline run

## Signal kinds

### `test`

Purpose: report test execution outcome.

Required `result` fields:

- `total_tests` (`u64`)
- `passed` (`u64`)
- `failed` (`u64`)
- `duration_ms` (`Option<u64>`)
- `runner` (`string`)

Required `offender` fields:

- `message` (`string`)
- `file` (`Option<string>`)
- `line` (`Option<u64>`)
- `test_name` (`Option<string>`)

Pass semantics: `failed == 0`.

### `coverage`

Purpose: report coverage quality.

Required `result` fields:

- `percent` (`Option<f64>`) — headline coverage percentage (0–100), comparable across adapters; set when the tool yields a primary metric (often matches line coverage)
- `line_percent` (`Option<f64>`)
- `branch_percent` (`Option<f64>`)
- `status` (`string`)
- `engine` (`string`)

Consumers that need a single number SHOULD prefer `percent`, then `line_percent`, then `branch_percent`.

Required `budget` fields:

- `line_percent_warn` (`Option<f64>`)
- `line_percent_fail` (`Option<f64>`)

Required `offender` fields:

- `file` (`string`)
- `line` (`Option<u64>`)
- `value` (`f64`)
- `level` (`warn|fail`)

Pass semantics: no `fail` level offenders and no runtime error status.

### `size`

Purpose: enforce file/module size budgets.

Required `result` fields:

- `max_lines` (`u64`)
- `total_files` (`u64`)
- `warn_count` (`u64`)
- `fail_count` (`u64`)

Required `offender` fields:

- `file` (`string`)
- `value` (`u64`)
- `warn` (`u64`)
- `fail` (`u64`)
- `level` (`warn|fail`)

Pass semantics: `fail_count == 0`.

### `complexity`

Purpose: cap function complexity.

Required `result` fields:

- `engine` (`string`)
- `method` (`string`)
- `measured_functions` (`u64`)
- `max_fn_cyclomatic` (`f64`)
- `max_fn_cognitive` (`Option<f64>`)
- `warn_count` (`u64`)
- `fail_count` (`u64`)

Required `offender` fields:

- `file` (`string`)
- `line` (`u64`)
- `function` (`string`)
- `cyclomatic` (`f64`)
- `cognitive` (`Option<f64>`)
- `level` (`warn|fail`)

Pass semantics: `fail_count == 0`.

### `deps`

Purpose: enforce architectural dependency constraints.

Required `result` fields:

- `crate_count` (`u64`)
- `edge_count` (`u64`)
- `violation_count` (`u64`)

Required `offender` fields:

- `from` (`string`)
- `to` (`string`)
- `rule` (`string`)
- `level` (`warn|fail`)

Pass semantics: `violation_count == 0`.

### `mutation`

Purpose: measure test suite fault-detection strength.

Required `result` fields:

- `engine` (`string`)
- `killed` (`u64`)
- `survived` (`u64`)
- `timeout` (`u64`)
- `score` (`Option<f64>`)

Required `offender` fields:

- `file` (`Option<string>`)
- `line` (`Option<u64>`)
- `mutation_kind` (`string`)
- `message` (`string`)
- `level` (`warn|fail`)

Pass semantics: no fail-level survivors above configured budget.

## Compatibility rules

- New adapters must only emit existing `kind` values.
- New fields may be added to kind-specific payloads, but existing fields and semantics must remain stable.
- Unknown/adapter-specific detail belongs in explicitly named extension sub-objects, never free-form top-level keys.
