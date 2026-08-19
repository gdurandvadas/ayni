---
layout: home

hero:
  name: Ayni
  text: Reproducible quality evidence for AI-edited repositories
  tagline: Commit policy, run repository tools in a locked environment, and give humans and agents the same scoped evidence.
  actions:
    - theme: brand
      text: Get started
      link: /getting-started/quickstart
    - theme: alt
      text: Installation
      link: /getting-started/installation
    - theme: alt
      text: How Ayni works
      link: /getting-started/how-ayni-works
    - theme: alt
      text: View on GitHub
      link: https://github.com/gdurandvadas/ayni

features:
  - title: One quality contract
    details: Version a closed signal vocabulary in .ayni.toml, with truthful adapter-specific capability tiers.
    link: /product/config
  - title: Managed execution
    details: Resolve exact tools and dependencies into a committed lock and a local OCI image.
    link: /product/environments
  - title: Focused feedback
    details: Verify one signal or run only the quality work affected by an explicit change.
    link: /product/runtime
  - title: Truthful adapter tiers
    details: Use one artifact vocabulary while seeing exactly which capabilities are supported, experimental, or unavailable.
    link: /product/capabilities
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

## Start with a reviewable proposal

After [installing Ayni](/getting-started/installation):

```sh
ayni init --dry-run
ayni init --write
ayni env show
ayni env lock
ayni env build
ayni check
```

`init` proposes a minimal test-only policy from adapter-owned project discovery; it never guesses thresholds and `--write` refuses to overwrite existing policy.

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

These commands use the managed environment directly. Optional, read-write `env run` and `env shell` access is documented only under [advanced development access](/product/environments#advanced-development-access).

## Supported language adapters

| Language | Managed project shape | Adapter guide |
| --- | --- | --- |
| Rust | Cargo projects and workspaces | [Rust](/adapters/rust) |
| Node | npm and pnpm projects and workspaces | [Node](/adapters/node) |
| Go | Go modules and workspaces | [Go](/adapters/go) |
| Python | uv projects and workspaces | [Python](/adapters/python) |
| Kotlin | Supported Gradle projects and workspaces | [Kotlin](/adapters/kotlin) |

Unsupported project variants fail explicitly instead of silently falling back to host tools. Signal depth also varies: consult the [adapter capability matrix](/product/capabilities). The `--host` option is labeled as an evaluation path and is not provenance-equivalent to managed evidence.
