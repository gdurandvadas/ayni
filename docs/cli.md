# Command-Line Help for `ayni`

This document contains the help content for the `ayni` command-line program.

**Command Overview:**

* [`ayni`↴](#ayni)
* [`ayni analyze`↴](#ayni-analyze)
* [`ayni verify`↴](#ayni-verify)
* [`ayni verify test`↴](#ayni-verify-test)
* [`ayni verify coverage`↴](#ayni-verify-coverage)
* [`ayni verify size`↴](#ayni-verify-size)
* [`ayni verify complexity`↴](#ayni-verify-complexity)
* [`ayni verify deps`↴](#ayni-verify-deps)
* [`ayni verify mutation`↴](#ayni-verify-mutation)
* [`ayni install`↴](#ayni-install)
* [`ayni agents`↴](#ayni-agents)
* [`ayni agents sync`↴](#ayni-agents-sync)
* [`ayni contract`↴](#ayni-contract)
* [`ayni contract display`↴](#ayni-contract-display)
* [`ayni artifact`↴](#ayni-artifact)
* [`ayni artifact compare`↴](#ayni-artifact-compare)
* [`ayni version`↴](#ayni-version)

## `ayni`

Open-source code quality signals for AI agents

**Usage:** `ayni <COMMAND>`

###### **Subcommands:**

* `analyze` — Analyze the local repository and print a quality report
* `verify` — Run focused, non-promotion verification
* `install` — Scaffold repository policy and show required tools; use `--apply` to install or `--check` to inspect readiness
* `agents` — Manage Ayni's agent instructions
* `contract` — Inspect the effective configured quality contract
* `artifact` — Compare two explicit complete signal artifacts without repository discovery
* `version` — Print the Ayni CLI version



## `ayni analyze`

Analyze the local repository and print a quality report

**Usage:** `ayni analyze [OPTIONS]`

###### **Options:**

* `--config <CONFIG>`

  Default value: `./.ayni.toml`
* `--output <OUTPUT>` — Report format: `stdout` (default, coloured console), `md` (markdown report), or `json` (machine-readable signal artifact on stdout)

  Possible values:
  - `stdout`:
    Coloured console report (default)
  - `md`:
    Markdown report printed to stdout
  - `json`:
    Machine-readable signal artifact (same shape as `.ayni/last/signals.json`) on stdout

* `--json` — Print the machine-readable signal artifact to stdout (equivalent to `--output json`)
* `--debug` — Print raw command diagnostics and disable the live dashboard



## `ayni verify`

Run focused, non-promotion verification

**Usage:** `ayni verify <COMMAND>`

###### **Subcommands:**

* `test` — Run only the test signal with adapter-owned selectors
* `coverage` — Run only the coverage signal with adapter-owned selectors
* `size` — Run only the size signal with adapter-owned selectors
* `complexity` — Run only the complexity signal with adapter-owned selectors
* `deps` — Run only the dependency signal with adapter-owned selectors
* `mutation` — Run only the mutation signal with adapter-owned selectors



## `ayni verify test`

Run only the test signal with adapter-owned selectors

**Usage:** `ayni verify test [OPTIONS]`

###### **Options:**

* `--config <CONFIG>`

  Default value: `./.ayni.toml`
* `--language <LANGUAGE>`

  Possible values: `rust`, `go`, `node`, `python`, `kotlin`

* `--output <OUTPUT>`

  Possible values:
  - `stdout`:
    Coloured console report (default)
  - `md`:
    Markdown report printed to stdout
  - `json`:
    Machine-readable signal artifact (same shape as `.ayni/last/signals.json`) on stdout

* `--json`
* `--debug` — Print raw command diagnostics
* `--file <FILE>`
* `--package <PACKAGE>`
* `--name <NAME>`



## `ayni verify coverage`

Run only the coverage signal with adapter-owned selectors

**Usage:** `ayni verify coverage [OPTIONS]`

###### **Options:**

* `--config <CONFIG>`

  Default value: `./.ayni.toml`
* `--language <LANGUAGE>`

  Possible values: `rust`, `go`, `node`, `python`, `kotlin`

* `--output <OUTPUT>`

  Possible values:
  - `stdout`:
    Coloured console report (default)
  - `md`:
    Markdown report printed to stdout
  - `json`:
    Machine-readable signal artifact (same shape as `.ayni/last/signals.json`) on stdout

* `--json`
* `--debug` — Print raw command diagnostics



## `ayni verify size`

Run only the size signal with adapter-owned selectors

**Usage:** `ayni verify size [OPTIONS]`

###### **Options:**

* `--config <CONFIG>`

  Default value: `./.ayni.toml`
* `--language <LANGUAGE>`

  Possible values: `rust`, `go`, `node`, `python`, `kotlin`

* `--output <OUTPUT>`

  Possible values:
  - `stdout`:
    Coloured console report (default)
  - `md`:
    Markdown report printed to stdout
  - `json`:
    Machine-readable signal artifact (same shape as `.ayni/last/signals.json`) on stdout

* `--json`
* `--debug` — Print raw command diagnostics
* `--file <FILE>`



## `ayni verify complexity`

Run only the complexity signal with adapter-owned selectors

**Usage:** `ayni verify complexity [OPTIONS]`

###### **Options:**

* `--config <CONFIG>`

  Default value: `./.ayni.toml`
* `--language <LANGUAGE>`

  Possible values: `rust`, `go`, `node`, `python`, `kotlin`

* `--output <OUTPUT>`

  Possible values:
  - `stdout`:
    Coloured console report (default)
  - `md`:
    Markdown report printed to stdout
  - `json`:
    Machine-readable signal artifact (same shape as `.ayni/last/signals.json`) on stdout

* `--json`
* `--debug` — Print raw command diagnostics
* `--file <FILE>`
* `--package <PACKAGE>`



## `ayni verify deps`

Run only the dependency signal with adapter-owned selectors

**Usage:** `ayni verify deps [OPTIONS]`

###### **Options:**

* `--config <CONFIG>`

  Default value: `./.ayni.toml`
* `--language <LANGUAGE>`

  Possible values: `rust`, `go`, `node`, `python`, `kotlin`

* `--output <OUTPUT>`

  Possible values:
  - `stdout`:
    Coloured console report (default)
  - `md`:
    Markdown report printed to stdout
  - `json`:
    Machine-readable signal artifact (same shape as `.ayni/last/signals.json`) on stdout

* `--json`
* `--debug` — Print raw command diagnostics
* `--file <FILE>`
* `--package <PACKAGE>`



## `ayni verify mutation`

Run only the mutation signal with adapter-owned selectors

**Usage:** `ayni verify mutation [OPTIONS]`

###### **Options:**

* `--config <CONFIG>`

  Default value: `./.ayni.toml`
* `--language <LANGUAGE>`

  Possible values: `rust`, `go`, `node`, `python`, `kotlin`

* `--output <OUTPUT>`

  Possible values:
  - `stdout`:
    Coloured console report (default)
  - `md`:
    Markdown report printed to stdout
  - `json`:
    Machine-readable signal artifact (same shape as `.ayni/last/signals.json`) on stdout

* `--json`
* `--debug` — Print raw command diagnostics



## `ayni install`

Scaffold repository policy and show required tools; use `--apply` to install or `--check` to inspect readiness

**Usage:** `ayni install [OPTIONS]`

###### **Options:**

* `--repo-root <REPO_ROOT>`

  Default value: `.`
* `--language <LANGUAGE>` — Limit setup to one or more languages; repeat `--language` for polyglot repositories

  Possible values: `rust`, `go`, `node`, `python`, `kotlin`

* `--apply` — Install missing or outdated tools from adapter catalogs (cargo, rustup, go, npm, …)
* `--check` — Check the existing policy and tooling without modifying the repository
* `--output <OUTPUT>` — Readiness output format; JSON is available only with `--check`

  Possible values: `json`




## `ayni agents`

Manage Ayni's agent instructions

**Usage:** `ayni agents <COMMAND>`

###### **Subcommands:**

* `sync` — Create or update Ayni's managed section in AGENTS.md



## `ayni agents sync`

Create or update Ayni's managed section in AGENTS.md

**Usage:** `ayni agents sync [OPTIONS]`

###### **Options:**

* `--repo-root <REPO_ROOT>`

  Default value: `.`



## `ayni contract`

Inspect the effective configured quality contract

**Usage:** `ayni contract <COMMAND>`

###### **Subcommands:**

* `display` — Display the validated policy without running analysis or discovery



## `ayni contract display`

Display the validated policy without running analysis or discovery

**Usage:** `ayni contract display [OPTIONS]`

###### **Options:**

* `--config <CONFIG>` — Path to the policy file to display

  Default value: `./.ayni.toml`
* `--output <OUTPUT>` — Render the deterministic contract projection as JSON

  Possible values: `json`




## `ayni artifact`

Compare two explicit complete signal artifacts without repository discovery

**Usage:** `ayni artifact <COMMAND>`

###### **Subcommands:**

* `compare` — Compare exactly two explicit schema-v3 artifact files



## `ayni artifact compare`

Compare exactly two explicit schema-v3 artifact files

**Usage:** `ayni artifact compare [OPTIONS] --baseline <BASELINE> --candidate <CANDIDATE>`

###### **Options:**

* `--baseline <BASELINE>` — Earlier artifact file
* `--candidate <CANDIDATE>` — Later artifact file
* `--output <OUTPUT>` — Comparison output format

  Default value: `stdout`

  Possible values:
  - `stdout`:
    Human-readable comparison report
  - `json`:
    One machine-readable comparison document




## `ayni version`

Print the Ayni CLI version

**Usage:** `ayni version`



<hr/>

<small><i>
    This document was generated automatically by
    <a href="https://crates.io/crates/clap-markdown"><code>clap-markdown</code></a>.
</i></small>

