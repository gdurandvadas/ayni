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
Both commands require an explicit local Git base and resolve it directly to a
commit. The candidate is the final local working-tree state relative to that
base: commits through `HEAD`, tracked index/worktree changes, and non-ignored
untracked files. Index and worktree deltas are not reported as separate hosted
review concepts. The deterministic fingerprint includes tracked binary/mode
changes and framed untracked path, type, executable-bit, link-target, and
content evidence. There is no hidden baseline, implicit merge-base, remote
fetch, pull-request lookup, or GitHub/GitLab/Bitbucket integration.

## Conservative planning

Ayni maps changed paths through configured language roots and adapter-owned
package topology. Rust Cargo workspaces and npm Node workspaces include the full
reverse-dependency closure, so changing a library also selects checks for
workspace packages that depend on it.

Each selected check records machine-readable inclusion reasons. When ownership,
topology, or tool scoping is uncertain, Ayni broadens from file or package scope
to the configured root and records the uncertainty. Contract changes broaden all
configured targets. Environment-lock changes conservatively broaden every configured target because
the lock is one repository environment contract. Missing impact capability never silently drops required work.

Signal strategies remain conservative:

- tests include directly affected and reverse-dependent packages when supported;
- coverage and mutation broaden whenever narrower evidence would be unsafe;
- size and complexity use changed files when the adapter can measure them exactly;
- dependency checks follow manifest and internal-topology changes;
- deleted, renamed, copied, ambiguous, or configuration-sensitive inputs broaden scope.

## Local Git requirements

Impact uses the local `git` executable and ordinary worktree data only. The
configured repository may be a subdirectory of a larger worktree; paths and the
candidate fingerprint are scoped to that configured repository. Ignored
untracked files are excluded, while tracked files remain visible regardless of
ignore rules. Unresolved conflicts, unsafe or non-UTF-8 paths, and unsupported
filesystem entries fail closed. Ayni never contacts or interprets a hosting
provider.

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
