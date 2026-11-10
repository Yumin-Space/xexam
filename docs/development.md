**English** | [日本語](development.ja.md) | [简体中文](development.zh-CN.md)

# Development

> Commands for everyday code changes and test runs. For the architecture see [architecture.md](architecture.md); for the testing approach see [test-strategy.md](test-strategy.md).

## 1. Tools

| Tool | Version | Notes |
|---|---|---|
| Rust | 1.99.1 (pinned by `backend/rust-toolchain.toml`) | `rustup` installs it automatically. The `bundled` feature of `rusqlite` needs a C compiler (Linux: `build-essential`; Windows: MinGW gcc or MSVC) |
| Node | 24 | Frontend |
| Python | 3.8+ | Used only to run tests: the golden-response comparison and the parser comparison ("the legacy version is the source of truth"). **Standard library only** |
| Docker | Compose v2 | Whole-stack checks, images |

## 2. Backend

```bash
cd backend
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test --all-targets          # 151 tests
cargo run -- serve --data ../data --no-browser     # listens on 127.0.0.1:8765
```

- Tests run against the **legacy snapshot** committed in `tests/baseline/`; Python is not needed (only integration scripts such as `migrate_check.py` run, using the Python standard library).
- All tests use a fresh database in a temporary directory or a copy of `tests/baseline/fixtures/sample.db`, and **never touch `data/`**.
- Common environment variables: `XEXAM_DATA`, `XEXAM_PORT`, `XEXAM_ALLOWED_HOSTS`, `XEXAM_LOG` (log level, e.g. `debug`). For the full list see `xexam-backend --help` and [deployment.md](deployment.md).
- Command-line import: `cargo run -- import ../some.xlsx --data ../data`.

## 3. Frontend

```bash
cd frontend
npm ci
npm run typecheck      # tsc (including test files)
npm test               # 27 unit tests (grading, wrong-answer rules, sessionStorage, language rules)
npm run build          # type check + vite build → dist/
npm run dev            # http://127.0.0.1:5173, /api is proxied to http://127.0.0.1:8765
```

- While developing, open the page via `localhost` or `127.0.0.1`. The Vite proxy's `changeOrigin` must be `false`: the backend compares `Host` with `Origin`, and rewriting `Host` makes write requests fail with 403.
- When the backend is not at the default address: `XEXAM_BACKEND=http://127.0.0.1:9000 npm run dev`.
- Import paths use the `.ts` extension (required by Node's built-in test runner).
- TypeScript is pinned to `~5.9.3` (a peer dependency of `openapi-typescript`).

## 4. Full stack

```bash
# Build and start the two containers (same xexam_data volume! Your own data will be used; to experiment, use another project name: -p some-other-name)
docker compose -f docker-compose.build.yml up -d --build

# The check scripts all use temporary project names and temporary volumes, and never touch xexam_data
docker build -f nginx/Dockerfile -t xexam-frontend:test .
python -I tests/integration/nginx_check.py xexam-frontend:test   # 38 checks, using a fake backend
python -I tests/integration/stack_check.py                      # 34 checks, starts the full stack (builds both images)
python -I tests/integration/migrate_check.py backend/target/debug/xexam-backend         # 10 checks: migration of an old progress.json (run cargo build first)
```

## 5. Comparing against the legacy version (regression)

The old Python implementation has been removed from the repository, but its behavior is pinned in `tests/baseline/` (all of it fictional, synthetic data):

| Snapshot | Content | Used by |
|---|---|---|
| `golden/NNN_*.json` (172 files) | Real responses of the legacy version for every endpoint × every state (status code, stable response headers, normalized body) | `backend/tests/api_golden.rs` replays them in order against the new routes and compares each item |
| `fixtures/*.xlsx` + `parser_golden/*.json` (49) | Synthetic workbooks and the legacy parser's output for each | `backend/tests/parser_golden.rs` |
| `golden/id_vectors.json` (85 entries) | Assertions about question ids; each entry carries the bytes that were hashed | `backend/tests/id_vectors.rs` |
| `fixtures/sample.db` + `golden/store_sample_outputs.json` | A sample database written by the legacy implementation, and the output of each of its interfaces | `backend/tests/db_golden.rs` |
| `golden/allowed_differences_rust.json` (23 entries), `allowed_probe_differences_rust.json` (51 entries) | Places where the new implementation intentionally differs from the legacy version, with a one-sentence reason for each | Any difference in `api_golden.rs` that is not registered = failure |

- To **change** behavior (for example, to add an intentional difference): change the code, then register the difference in `allowed_differences_rust.json` with its reason; do not edit the golden responses directly.
- The old capture / comparison / probe scripts depended on the old Python code and were removed with it; if you need them, recover them from the Git tag `python-v0.2` and the commit history.

## 6. Data safety rules

- **Do not** read, print, commit or modify `data/`, `progress.json`, or your own question-bank Excel files.
- Always experiment on a copy or in a temporary directory; do not run "let's see what happens" commands against the real `xexam.db`.
- Do not run `docker compose down -v`.

## 7. CI

There is only one pipeline: [`.github/workflows/ci.yml`](../.github/workflows/ci.yml).

| Job | What it does |
|---|---|
| `check` | `VERSION` format; the top entry of the README changelog = `VERSION` |
| `frontend` | Type check, 27 unit tests, build |
| `backend` | fmt, clippy, 151 tests, and the `progress.json` migration check |
| `stack` | Builds the `xexam-frontend` image and runs 38 Nginx checks; starts the full compose stack and runs 34 end-to-end checks |
| `arm64` | Builds both images for arm64, starts them under QEMU, and checks that `/api/info` reports `aarch64` |
| `publish` | `main` only: pushes both images (amd64 + arm64) to ghcr.io and verifies the manifest and tags |
| `release` | `main` only: creates (or updates) the GitHub Release for `v<VERSION>` |

Pushes to `dev`, pull requests into `main` and other branches run only the first five jobs and **never publish**; only a push to `main` does.

## 8. Version number and changelog

- The single source of truth for the version number is `VERSION` in the repository root. `Cargo.toml` and `package.json` each hold a copy; keep them consistent.
- Every update must add an entry at the very top of the README's "Changelog" section, with a version number that matches `VERSION` (CI verifies this).
