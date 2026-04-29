# Command-Line Help for `ayni`

This document contains the help content for the `ayni` command-line program.

**Command Overview:**

* [`ayni`↴](#ayni)
* [`ayni analyze`↴](#ayni-analyze)
* [`ayni install`↴](#ayni-install)
* [`ayni version`↴](#ayni-version)

## `ayni`

Local-first code quality signals for AI agents

**Usage:** `ayni <COMMAND>`

###### **Subcommands:**

* `analyze` — Analyze the local repository and print a quality report
* `install` — Scaffold repo guidance and show required tools; use `--apply` to install them
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

  Possible values: `rust`, `go`, `node`

* `--output <OUTPUT>` — Report format: `stdout` (default, coloured console) or `md` (markdown tables)

  Default value: `stdout`

  Possible values:
  - `stdout`:
    Coloured console report (default)
  - `md`:
    Markdown tables written to `.ayni/last/summary.llm.md` and printed to stdout




## `ayni install`

Scaffold repo guidance and show required tools; use `--apply` to install them

**Usage:** `ayni install [OPTIONS]`

###### **Options:**

* `--repo-root <REPO_ROOT>`

  Default value: `.`
* `--language <LANGUAGE>`

  Possible values: `rust`, `go`, `node`

* `--apply` — Install missing or outdated tools from adapter catalogs (cargo, rustup, go, npm, …)



## `ayni version`

Print the Ayni CLI version

**Usage:** `ayni version`



<hr/>

<small><i>
    This document was generated automatically by
    <a href="https://crates.io/crates/clap-markdown"><code>clap-markdown</code></a>.
</i></small>

