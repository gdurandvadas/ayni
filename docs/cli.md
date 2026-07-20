# Command-Line Help for `ayni`

This document contains the help content for the `ayni` command-line program.

**Command Overview:**

* [`ayni`↴](#ayni)
* [`ayni analyze`↴](#ayni-analyze)
* [`ayni verify`↴](#ayni-verify)
* [`ayni verify test`↴](#ayni-verify-test)
* [`ayni install`↴](#ayni-install)
* [`ayni agents`↴](#ayni-agents)
* [`ayni agents sync`↴](#ayni-agents-sync)
* [`ayni version`↴](#ayni-version)

## `ayni`

Open-source code quality signals for AI agents

**Usage:** `ayni <COMMAND>`

###### **Subcommands:**

* `analyze` — Analyze the local repository and print a quality report
* `verify` — Run focused, non-promotion verification
* `install` — Scaffold repository policy and show required tools; use `--apply` to install them
* `agents` — Manage Ayni's agent instructions
* `version` — Print the Ayni CLI version



## `ayni analyze`

Analyze the local repository and print a quality report

**Usage:** `ayni analyze [OPTIONS]`

###### **Options:**

* `--config <CONFIG>`

  Default value: `./.ayni.toml`
* `--file <FILE>`
* `--package <PACKAGE>`
* `--language <LANGUAGE>`

  Possible values: `rust`, `go`, `node`, `python`, `kotlin`

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



## `ayni verify test`

Run only the test signal with adapter-owned selectors

**Usage:** `ayni verify test [OPTIONS]`

###### **Options:**

* `--config <CONFIG>`

  Default value: `./.ayni.toml`
* `--file <FILE>`
* `--package <PACKAGE>`
* `--name <NAME>`
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
* `--debug`



## `ayni install`

Scaffold repository policy and show required tools; use `--apply` to install them

**Usage:** `ayni install [OPTIONS]`

###### **Options:**

* `--repo-root <REPO_ROOT>`

  Default value: `.`
* `--language <LANGUAGE>` — Limit setup to one or more languages; repeat `--language` for polyglot repositories

  Possible values: `rust`, `go`, `node`, `python`, `kotlin`

* `--apply` — Install missing or outdated tools from adapter catalogs (cargo, rustup, go, npm, …)



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



## `ayni version`

Print the Ayni CLI version

**Usage:** `ayni version`



<hr/>

<small><i>
    This document was generated automatically by
    <a href="https://crates.io/crates/clap-markdown"><code>clap-markdown</code></a>.
</i></small>

