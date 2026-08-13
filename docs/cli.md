# Command-Line Help for `ayni`

This document contains the help content for the `ayni` command-line program.

**Command Overview:**

* [`ayni`↴](#ayni)
* [`ayni init`↴](#ayni-init)
* [`ayni env`↴](#ayni-env)
* [`ayni env show`↴](#ayni-env-show)
* [`ayni env doctor`↴](#ayni-env-doctor)
* [`ayni env lock`↴](#ayni-env-lock)
* [`ayni env build`↴](#ayni-env-build)
* [`ayni env shell`↴](#ayni-env-shell)
* [`ayni env run`↴](#ayni-env-run)
* [`ayni contract`↴](#ayni-contract)
* [`ayni contract show`↴](#ayni-contract-show)
* [`ayni contract validate`↴](#ayni-contract-validate)
* [`ayni verify`↴](#ayni-verify)
* [`ayni verify test`↴](#ayni-verify-test)
* [`ayni verify coverage`↴](#ayni-verify-coverage)
* [`ayni verify size`↴](#ayni-verify-size)
* [`ayni verify complexity`↴](#ayni-verify-complexity)
* [`ayni verify deps`↴](#ayni-verify-deps)
* [`ayni verify mutation`↴](#ayni-verify-mutation)
* [`ayni impact`↴](#ayni-impact)
* [`ayni impact show`↴](#ayni-impact-show)
* [`ayni impact run`↴](#ayni-impact-run)
* [`ayni check`↴](#ayni-check)
* [`ayni agents`↴](#ayni-agents)
* [`ayni agents sync`↴](#ayni-agents-sync)
* [`ayni results`↴](#ayni-results)
* [`ayni results show`↴](#ayni-results-show)
* [`ayni results compare`↴](#ayni-results-compare)

## `ayni`

Correct environments, focused feedback, one definitive quality gate

**Usage:** `ayni <COMMAND>`

###### **Subcommands:**

* `init` — Prepare a repository for Ayni
* `env` — Inspect and manage the repository code environment
* `contract` — Inspect and validate the repository quality contract
* `verify` — Run one quality signal with optional adapter-owned selectors
* `impact` — Explain or run the checks affected by an explicit change
* `check` — Run the complete repository quality contract
* `agents` — Manage Ayni's agent instructions
* `results` — Inspect and compare explicit local result files



## `ayni init`

Prepare a repository for Ayni

**Usage:** `ayni init [OPTIONS]`

###### **Options:**

* `--repo-root <REPO_ROOT>`

  Default value: `.`



## `ayni env`

Inspect and manage the repository code environment

**Usage:** `ayni env <COMMAND>`

###### **Subcommands:**

* `show` — Explain the resolved environment plan without modifying state
* `doctor` — Diagnose missing, conflicting, unsupported, or stale environment state
* `lock` — Resolve exact environment requirements into the committed lock
* `build` — Build the repository code-environment image from a current lock
* `shell` — Enter the managed environment with the checkout mounted
* `run` — Run an arbitrary command inside the managed environment



## `ayni env show`

Explain the resolved environment plan without modifying state

**Usage:** `ayni env show [OPTIONS]`

###### **Options:**

* `--repo-root <REPO_ROOT>`

  Default value: `.`



## `ayni env doctor`

Diagnose missing, conflicting, unsupported, or stale environment state

**Usage:** `ayni env doctor [OPTIONS]`

###### **Options:**

* `--repo-root <REPO_ROOT>`

  Default value: `.`



## `ayni env lock`

Resolve exact environment requirements into the committed lock

**Usage:** `ayni env lock [OPTIONS]`

###### **Options:**

* `--repo-root <REPO_ROOT>`

  Default value: `.`



## `ayni env build`

Build the repository code-environment image from a current lock

**Usage:** `ayni env build [OPTIONS]`

###### **Options:**

* `--repo-root <REPO_ROOT>`

  Default value: `.`



## `ayni env shell`

Enter the managed environment with the checkout mounted

**Usage:** `ayni env shell [OPTIONS]`

###### **Options:**

* `--repo-root <REPO_ROOT>`

  Default value: `.`



## `ayni env run`

Run an arbitrary command inside the managed environment

**Usage:** `ayni env run [OPTIONS] -- <COMMAND>...`

###### **Arguments:**

* `<COMMAND>`

###### **Options:**

* `--repo-root <REPO_ROOT>`

  Default value: `.`



## `ayni contract`

Inspect and validate the repository quality contract

**Usage:** `ayni contract <COMMAND>`

###### **Subcommands:**

* `show` — Render the effective quality contract
* `validate` — Validate the contract without discovery or tool execution



## `ayni contract show`

Render the effective quality contract

**Usage:** `ayni contract show [OPTIONS]`

###### **Options:**

* `--config <CONFIG>`

  Default value: `./.ayni.toml`
* `--output <OUTPUT>`

  Default value: `human`

  Possible values:
  - `human`:
    Human-readable terminal output
  - `json`:
    One deterministic JSON document on stdout




## `ayni contract validate`

Validate the contract without discovery or tool execution

**Usage:** `ayni contract validate [OPTIONS]`

###### **Options:**

* `--config <CONFIG>`

  Default value: `./.ayni.toml`
* `--output <OUTPUT>`

  Default value: `human`

  Possible values:
  - `human`:
    Human-readable terminal output
  - `json`:
    One deterministic JSON document on stdout




## `ayni verify`

Run one quality signal with optional adapter-owned selectors

**Usage:** `ayni verify <COMMAND>`

###### **Subcommands:**

* `test` — Run only the test signal
* `coverage` — Run only the coverage signal
* `size` — Run only the size signal
* `complexity` — Run only the complexity signal
* `deps` — Run only the dependency signal
* `mutation` — Run only the mutation signal



## `ayni verify test`

Run only the test signal

**Usage:** `ayni verify test [OPTIONS]`

###### **Options:**

* `--config <CONFIG>`

  Default value: `./.ayni.toml`
* `--language <LANGUAGE>`

  Possible values: `rust`, `go`, `node`, `python`, `kotlin`

* `--root <ROOT>` — Select exactly one normalized root configured for the selected language
* `--output <OUTPUT>`

  Default value: `human`

  Possible values:
  - `human`:
    Human-readable terminal output
  - `json`:
    One deterministic JSON document on stdout
  - `markdown`:
    Deterministic Markdown output

* `--host` — Run on the host instead of in the managed environment
* `--debug` — Print raw command diagnostics
* `--file <FILE>`
* `--package <PACKAGE>`
* `--name <NAME>`



## `ayni verify coverage`

Run only the coverage signal

**Usage:** `ayni verify coverage [OPTIONS]`

###### **Options:**

* `--config <CONFIG>`

  Default value: `./.ayni.toml`
* `--language <LANGUAGE>`

  Possible values: `rust`, `go`, `node`, `python`, `kotlin`

* `--root <ROOT>` — Select exactly one normalized root configured for the selected language
* `--output <OUTPUT>`

  Default value: `human`

  Possible values:
  - `human`:
    Human-readable terminal output
  - `json`:
    One deterministic JSON document on stdout
  - `markdown`:
    Deterministic Markdown output

* `--host` — Run on the host instead of in the managed environment
* `--debug` — Print raw command diagnostics



## `ayni verify size`

Run only the size signal

**Usage:** `ayni verify size [OPTIONS]`

###### **Options:**

* `--config <CONFIG>`

  Default value: `./.ayni.toml`
* `--language <LANGUAGE>`

  Possible values: `rust`, `go`, `node`, `python`, `kotlin`

* `--root <ROOT>` — Select exactly one normalized root configured for the selected language
* `--output <OUTPUT>`

  Default value: `human`

  Possible values:
  - `human`:
    Human-readable terminal output
  - `json`:
    One deterministic JSON document on stdout
  - `markdown`:
    Deterministic Markdown output

* `--host` — Run on the host instead of in the managed environment
* `--debug` — Print raw command diagnostics
* `--file <FILE>`



## `ayni verify complexity`

Run only the complexity signal

**Usage:** `ayni verify complexity [OPTIONS]`

###### **Options:**

* `--config <CONFIG>`

  Default value: `./.ayni.toml`
* `--language <LANGUAGE>`

  Possible values: `rust`, `go`, `node`, `python`, `kotlin`

* `--root <ROOT>` — Select exactly one normalized root configured for the selected language
* `--output <OUTPUT>`

  Default value: `human`

  Possible values:
  - `human`:
    Human-readable terminal output
  - `json`:
    One deterministic JSON document on stdout
  - `markdown`:
    Deterministic Markdown output

* `--host` — Run on the host instead of in the managed environment
* `--debug` — Print raw command diagnostics
* `--file <FILE>`
* `--package <PACKAGE>`



## `ayni verify deps`

Run only the dependency signal

**Usage:** `ayni verify deps [OPTIONS]`

###### **Options:**

* `--config <CONFIG>`

  Default value: `./.ayni.toml`
* `--language <LANGUAGE>`

  Possible values: `rust`, `go`, `node`, `python`, `kotlin`

* `--root <ROOT>` — Select exactly one normalized root configured for the selected language
* `--output <OUTPUT>`

  Default value: `human`

  Possible values:
  - `human`:
    Human-readable terminal output
  - `json`:
    One deterministic JSON document on stdout
  - `markdown`:
    Deterministic Markdown output

* `--host` — Run on the host instead of in the managed environment
* `--debug` — Print raw command diagnostics
* `--file <FILE>`
* `--package <PACKAGE>`



## `ayni verify mutation`

Run only the mutation signal

**Usage:** `ayni verify mutation [OPTIONS]`

###### **Options:**

* `--config <CONFIG>`

  Default value: `./.ayni.toml`
* `--language <LANGUAGE>`

  Possible values: `rust`, `go`, `node`, `python`, `kotlin`

* `--root <ROOT>` — Select exactly one normalized root configured for the selected language
* `--output <OUTPUT>`

  Default value: `human`

  Possible values:
  - `human`:
    Human-readable terminal output
  - `json`:
    One deterministic JSON document on stdout
  - `markdown`:
    Deterministic Markdown output

* `--host` — Run on the host instead of in the managed environment
* `--debug` — Print raw command diagnostics



## `ayni impact`

Explain or run the checks affected by an explicit change

**Usage:** `ayni impact <COMMAND>`

###### **Subcommands:**

* `show` — Explain the quality work affected by a change without running it
* `run` — Execute the quality work affected by a change



## `ayni impact show`

Explain the quality work affected by a change without running it

**Usage:** `ayni impact show [OPTIONS] --base <BASE>`

###### **Options:**

* `--config <CONFIG>`

  Default value: `./.ayni.toml`
* `--base <BASE>` — Explicit base revision used to calculate the change
* `--output <OUTPUT>`

  Default value: `human`

  Possible values:
  - `human`:
    Human-readable terminal output
  - `json`:
    One deterministic JSON document on stdout
  - `markdown`:
    Deterministic Markdown output

* `--host` — Run on the host instead of in the managed environment
* `--debug` — Print raw command diagnostics



## `ayni impact run`

Execute the quality work affected by a change

**Usage:** `ayni impact run [OPTIONS] --base <BASE>`

###### **Options:**

* `--config <CONFIG>`

  Default value: `./.ayni.toml`
* `--base <BASE>` — Explicit base revision used to calculate the change
* `--output <OUTPUT>`

  Default value: `human`

  Possible values:
  - `human`:
    Human-readable terminal output
  - `json`:
    One deterministic JSON document on stdout
  - `markdown`:
    Deterministic Markdown output

* `--host` — Run on the host instead of in the managed environment
* `--debug` — Print raw command diagnostics



## `ayni check`

Run the complete repository quality contract

**Usage:** `ayni check [OPTIONS]`

###### **Options:**

* `--config <CONFIG>`

  Default value: `./.ayni.toml`
* `--output <OUTPUT>`

  Default value: `human`

  Possible values:
  - `human`:
    Human-readable terminal output
  - `json`:
    One deterministic JSON document on stdout
  - `markdown`:
    Deterministic Markdown output

* `--host` — Run on the host instead of in the managed environment
* `--debug` — Print raw command diagnostics



## `ayni agents`

Manage Ayni's agent instructions

**Usage:** `ayni agents <COMMAND>`

###### **Subcommands:**

* `sync` — Create or refresh only Ayni's managed AGENTS.md block



## `ayni agents sync`

Create or refresh only Ayni's managed AGENTS.md block

**Usage:** `ayni agents sync [OPTIONS]`

###### **Options:**

* `--repo-root <REPO_ROOT>`

  Default value: `.`



## `ayni results`

Inspect and compare explicit local result files

**Usage:** `ayni results <COMMAND>`

###### **Subcommands:**

* `show` — Render one explicit local result file
* `compare` — Compare two explicit compatible result files



## `ayni results show`

Render one explicit local result file

**Usage:** `ayni results show [OPTIONS] --file <FILE>`

###### **Options:**

* `--file <FILE>` — Result file to render
* `--output <OUTPUT>`

  Default value: `human`

  Possible values:
  - `human`:
    Human-readable terminal output
  - `json`:
    One deterministic JSON document on stdout
  - `markdown`:
    Deterministic Markdown output




## `ayni results compare`

Compare two explicit compatible result files

**Usage:** `ayni results compare [OPTIONS] --baseline <BASELINE> --candidate <CANDIDATE>`

###### **Options:**

* `--baseline <BASELINE>` — Earlier result file
* `--candidate <CANDIDATE>` — Later result file
* `--output <OUTPUT>`

  Default value: `human`

  Possible values:
  - `human`:
    Human-readable terminal output
  - `json`:
    One deterministic JSON document on stdout




<hr/>

<small><i>
    This document was generated automatically by
    <a href="https://crates.io/crates/clap-markdown"><code>clap-markdown</code></a>.
</i></small>
