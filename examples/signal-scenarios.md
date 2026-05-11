# Intentional signal scenarios

## Single repos (`examples/<lang>/single`)
- No `.ayni.toml` and no `.ayni/` folder: validates `install` bootstrap.
- Greeting service endpoint exists.
- Intentional complexity hotspot in `complex*` function to trigger complexity/readability signals.

## Monorepos (`examples/<lang>/mono`)
- Two libs + one service.
- `math` lib has exactly 10 exported functions.
- Tests cover 8/10 math functions (target ~80%).
- Service depends on both libs.
- Service includes an extra third-party dependency (`reqwest` / `lodash` / `requests` / `logrus` / `okhttp`) as intentional dependency-policy scenario.
