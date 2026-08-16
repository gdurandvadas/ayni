# Intentional signal scenarios

## Single repos (`examples/<lang>/single`)
- No `.ayni.toml`, `.ayni.lock`, or generated `.ayni/` state. These are raw
  language fixtures for discovery and explicit contract setup; Ayni has no
  implicit install/bootstrap command.
- `ayni agents sync --repo-root <path>` remains the only command that creates or
  refreshes managed agent guidance.
- Greeting service endpoint exists.
- Intentional complexity hotspot in `complex*` function to trigger complexity/readability signals.

## Monorepos (`examples/<lang>/mono`)
- Two libs + one service.
- `math` lib has exactly 10 exported functions.
- Tests cover 8/10 math functions (target ~80%).
- Service depends on both libs.
- Service includes an extra third-party dependency (`reqwest` / `lodash` / `requests` / `logrus` / `okhttp`) as intentional dependency-policy scenario.
