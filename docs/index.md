---
layout: home

hero:
  name: Ayni
  text: Reproducible quality signals for every repository
  tagline: Define what healthy means once, run it in a locked environment, and give humans and agents the same focused feedback.
  actions:
    - theme: brand
      text: Get started
      link: /getting-started/installation
    - theme: alt
      text: How Ayni works
      link: /getting-started/how-ayni-works
    - theme: alt
      text: View on GitHub
      link: https://github.com/gdurandvadas/ayni

features:
  - title: One quality contract
    details: Version tests, coverage, size, complexity, dependency, and mutation policy in .ayni.toml.
    link: /product/config
  - title: Managed execution
    details: Resolve exact tools and dependencies into a committed lock and a local OCI image.
    link: /product/environments
  - title: Focused feedback
    details: Verify one signal or run only the quality work affected by an explicit change.
    link: /product/runtime
  - title: Polyglot by design
    details: Use the same quality model across Rust, Node, Go, Python, and Kotlin roots.
    link: /getting-started/quickstart
---

## The core model

```text
Quality contract       Managed environment       Signal execution
.ayni.toml          +  .ayni.lock / OCI image  → check / verify / impact
"What is healthy?"    "Where does it run?"       "What did we measure?"
```

Ayni keeps these responsibilities separate and reviewable:

1. **Define policy.** `.ayni.toml` selects languages, signals, thresholds, structural rules, and optional runtime capabilities.
2. **Lock execution.** `.ayni.lock` records exact tools, native dependency inputs, preparation state, and an immutable base image.
3. **Measure code.** `check`, `verify`, and `impact run` launch managed environments automatically and produce normalized evidence.

Installing the CLI is separate from provisioning a repository environment. Ayni never silently creates a lock or rebuilds an image during a quality run.

## Start in five commands

After [installing Ayni](/getting-started/installation) and adding `.ayni.toml`:

```sh
ayni contract validate
ayni env show
ayni env lock
ayni env build
ayni check
```

Commit `.ayni.toml`, `.ayni.lock`, and your native dependency/tool locks. Keep `.ayni/` and the generated OCI image local.

[Follow the complete quickstart →](/getting-started/quickstart)

## A quality loop for humans and agents

Use a focused command while developing:

```sh
ayni verify test
ayni impact run --base origin/main
```

Then run the complete repository contract before integration:

```sh
ayni check
```

These commands use the managed environment directly. `ayni env run` and `ayni env shell` remain available for arbitrary development commands; they are not wrappers required by Ayni's quality commands.

## Supported language adapters

| Language | Managed project shape | Adapter guide |
| --- | --- | --- |
| Rust | Cargo projects and workspaces | [Rust](/adapters/rust) |
| Node | npm and pnpm projects and workspaces | [Node](/adapters/node) |
| Go | Go modules and workspaces | [Go](/adapters/go) |
| Python | uv projects and workspaces | [Python](/adapters/python) |
| Kotlin | Supported Gradle projects and workspaces | [Kotlin](/adapters/kotlin) |

Unsupported project variants fail explicitly instead of silently falling back to host tools. The `--host` option is an intentional escape hatch.
