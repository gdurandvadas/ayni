# Impact-aware execution
# Impact-aware execution

Impact analysis gives coding agents a smaller validation loop without weakening
`ayni check` as the repository completion gate.

## Commands

Explain the checks affected by the current checkout relative to an explicit Git
base:

```sh
ayni impact show --base <revision>
```

Run that plan:

```sh
ayni impact run --base <revision>
```

Managed execution is the default for `impact run`; use `--host` only as the
explicit host escape hatch. `impact show` plans without running quality tools.
Both commands require an explicit base and resolve it to a commit. The candidate
is recorded as the current `HEAD` plus staged, unstaged, and untracked files.
The deterministic fingerprint includes tracked binary/mode changes and framed
untracked path, type, executable-bit, link-target, and content evidence. There
is no hidden or automatically selected baseline.

## Conservative planning

Ayni maps changed paths through configured language roots and adapter-owned
package topology. Rust Cargo workspaces and npm Node workspaces include the full
reverse-dependency closure, so changing a library also selects checks for
workspace packages that depend on it.

Each selected check records machine-readable inclusion reasons. When ownership,
topology, or tool scoping is uncertain, Ayni broadens from file or package scope
to the configured root and records the uncertainty. Contract changes broaden all
configured targets. Environment-lock changes broaden every affected runtime
target. Missing impact capability never silently drops required work.

Signal strategies remain conservative:

- tests include directly affected and reverse-dependent packages when supported;
- coverage and mutation broaden whenever narrower evidence would be unsafe;
- size and complexity use changed files when the adapter can measure them exactly;
- dependency checks follow manifest and internal-topology changes;
- deleted, renamed, ambiguous, or configuration-sensitive inputs broaden scope.

## Results are not completion evidence

`impact run` writes its separate result below:

```text
.ayni/impact/last/impact.json
```

The impact envelope contains the resolved base and candidate, changed inputs,
selected checks and reasons, uncertainties, typed signal rows, and execution
issues. It explicitly states that repository completion is still required.
Impact evidence never overwrites `.ayni/last/signals.json` or focused verification
evidence. Ayni refuses impact planning unless this generated slot is ignored by
Git, and recomputes the plan immediately before persistence so candidate drift
fails closed.

A successful impact run means only that its selected plan passed. Before calling
work complete, always run:

```sh
ayni check
```

## Output

Human output emphasizes selected scopes, reasons, broadening, and the final-gate
reminder. `--output json` emits one deterministic document on stdout; progress
and diagnostics use stderr. Markdown output is suitable for summaries while
retaining the same prominent non-completion marker.
