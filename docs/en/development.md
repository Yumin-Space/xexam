**English** | [日本語](../ja/development.md) | [简体中文](../zh-CN/development.md)

# Development

> Commands for everyday code changes and test runs, and where things live. For the architecture see [architecture.md](architecture.md); for the original testing approach see [test-strategy.md](../zh-CN/history/test-strategy.md).

## 1. Tools

| Tool | Version | Notes |
|---|---|---|
| Rust | 1.99.1 (pinned by `backend/rust-toolchain.toml`) | `rustup` installs it automatically. The `bundled` feature of `rusqlite` needs a C compiler (Linux: `build-essential`; Windows: MinGW gcc or MSVC) |
| Node | 24 | Frontend; the unit tests use Node's built-in test runner with `--experimental-strip-types` |
| Python | 3.8+ | Only for the integration scripts in `tests/integration/` and the two scripts in `scripts/`. **Standard library only** |
| Docker | Compose v2 | Whole-stack checks, images |

## 2. Backend

```bash
cd backend
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test --all-targets          # 302 tests
cargo run -- serve --data ../data --no-browser     # listens on 127.0.0.1:8765
```

- Tests run against the **legacy snapshot** committed in `tests/baseline/`; Python is not needed for them.
- All tests use a fresh database in a temporary directory or a copy of `tests/baseline/fixtures/sample.db`, and **never touch `data/`**. Besides the unit tests in `src/`, the test files are: `api_basic.rs`, `api_import.rs`, `api_misc.rs` and `api_golden.rs` (routes, guards, the legacy responses), `db_golden.rs` (the database against the legacy snapshot), `parser_golden.rs`, `parser_hardening.rs`, `parser_four_columns.rs`, `parser_six_columns.rs` and `id_vectors.rs` (the parser), and one file per version: `v05_banks_records_settings.rs` (schema v2, banks, records, settings), `v06_users_and_sign_in.rs` (schema v3, accounts, sessions, the login throttle), `v065_bank_access_and_practice.rs` (schema v4, bank access, bank managers, ownership, practice), `v07_wal_and_logs.rs` (WAL mode, the checkpoint, `reset-password` while the server runs, the System log) and `v07_log_floods.rs` (floods of rejected requests and failed sign-ins are summed up).
- Tests sign in through helpers; no switch turns sign-in off, also not for tests.
- Common environment variables: `XEXAM_DATA`, `XEXAM_PORT`, `XEXAM_ALLOWED_HOSTS`, `XEXAM_LOG` (log level, e.g. `debug`), `XEXAM_SQLITE_JOURNAL` (`delete` keeps the rollback journal instead of WAL), `XEXAM_COOKIE_SECURE`. For the full list see `xexam-backend --help` and [deployment.md](deployment.md).
- The first start on an empty data directory creates the administrator `admin` / `admin`; the page asks for a new password at the first sign-in. If you forget it: `cargo run -- reset-password admin --data ../data` (it works while the server runs).
- Command-line import (into the legacy bank): `cargo run -- import ../some.xlsx --data ../data`.
- Pointing the backend at a database of an older schema migrates it and writes `backups/` next to it. Use a copy.
- The database runs in WAL mode: while the backend runs, `xexam.db-wal` and `xexam.db-shm` sit next to `xexam.db`. Copy all three, or stop the backend first (everything is then in `xexam.db`; a `-wal` file left next to it is empty). `sqlite3` can read the database while the backend runs.

## 3. Frontend

```bash
cd frontend
npm ci
npm run typecheck      # tsc for the app, then for the test files (tsconfig.test.json)
npm test               # 119 unit tests: grading and random pick, practice checking, saved results, language rules, records filter,
                       # lock notes and waits, import warnings, the System log buffer, bank access, saved exam, settings cache,
                       # UI catalogs, server-message translation, changelog parsing
npm run build          # type check + vite build → dist/
npm run dev            # http://127.0.0.1:5173, /api is proxied to http://127.0.0.1:8765
```

- While developing, open the page via `localhost` or `127.0.0.1`. The Vite proxy's `changeOrigin` must be `false`: the backend compares `Host` with `Origin`, and rewriting `Host` makes write requests fail with 403.
- When the backend is not at the default address: `XEXAM_BACKEND=http://127.0.0.1:9000 npm run dev`.
- The About page shows the changelogs of `README.md`, `README/README.ja.md` and `README/README.zh-CN.md`. The `xexam-changelog` plugin in `vite.config.ts` reads them from the folder above `frontend/` (the repository root) when the frontend is built (and in `npm run dev` when the About page is first opened), so they must be there. The build stops with an error when one of them has no changelog entry (`### vX.Y — date`).
- To run one test file: `node --experimental-strip-types --test src/i18n/server-messages.test.ts`.
- Import paths use the `.ts` extension (required by Node's built-in test runner).
- TypeScript is pinned to `~5.9.3`.

### 3.1 Interface texts

Every text the user sees comes from a catalog. Nothing is written into a page directly.

- **Where**: `src/i18n/catalog/<area>.ts`, one file per area: `common`, `home`, `banks`, `records`, `settings`, `quiz`, `result`, `about`, `auth`, `users`, `logs`. Each file is `export default defineCatalog({ en: {…}, zh: {…}, ja: {…} })`.
- **Keys** start with the area: `banks.delete.title`, `records.deleteAll.ok`. `common.ts` holds the shared prefixes (`app.`, `nav.`, `common.`, `err.`, `kind.`, `mode.`, `act.`, `role.`, `forbidden.`, …).
- **Three languages, always.** TypeScript refuses a `zh` or `ja` block that misses an English key. A key ending in `_one` is the English singular; `tn()` uses it when the count is 1, and it is optional in Chinese and Japanese.
- **Placeholders** are `{name}`: `t('records.deleteOne.body', { at, pct })`. For counts use `tn(key, n)`, which fills `{n}`.
- **Checks**: `src/i18n/catalog.test.ts` makes sure all three languages have exactly the same keys and the same placeholders, no text is empty, every key carries its area prefix and exists in one catalog only, and every catalog file is registered in `src/i18n/index.ts`.
- **A new catalog file**: import it in `src/i18n/index.ts`, add it to `ALL` and to the `MsgKey` type.
- Write English first, then Chinese and Japanese. Use the terms the UI already uses (for example "question bank", "retired", "wrong-answer notebook", "bank manager").
- The question / answer language switch is the exception: its three buttons always say **English**, **中文** and **日本語** (`src/components/lang-switch.ts`), whatever the interface language; only its accessible name and tooltips come from the catalog.

### 3.2 Backend messages in the UI

Backend error messages are Chinese and stable (they are part of the tested API). The frontend translates them:

- `src/i18n/server-messages.ts`: `EXACT` maps whole messages, `PATTERNS` maps the backend's format strings (their `{}` parts become captured values). Chinese UI shows the message unchanged; unknown text is shown as it is. A few low-level protocol errors are English words (`not found`, `bad json`, …); they get a sentence in every language, Chinese included.
- `src/i18n/errors.ts`: `errorText(e)` turns any error into text in the current language. Pages use it for every error they show. A page's own error is a `LocalError(key, params)`, so it changes language with the UI.
- **Drift test**: `src/i18n/server-messages.test.ts` reads the Rust sources in `backend/src` with a small lexer. Every string literal with Chinese or Japanese characters (outside comments, test code, assertions, log and console output) must have an English and a Japanese translation, keep the values of its placeholders and contain no Chinese. It also fails on translations whose backend text no longer exists.
- **When you add or change a backend message**: add the translation in `server-messages.ts` in the same change. If the string is not shown to the user (for example a word the parser looks for), add it to `NOT_MESSAGES` in the test, with the reason.
- **Log lines stay English** and must never contain a password, a session token, a cookie value or question text (`backend/tests/v07_wal_and_logs.rs` checks the log after a whole session). Lines that anyone can cause without signing in go through a `BurstLog` (`backend/src/logs.rs`), not straight to `tracing`.

### 3.3 Adding a page or route

1. **Route**: in `src/router.ts`, add a variant to `Route`, a case to `parseHash` and, if its hash is not `#/<name>`, to `routeHash`. Say in `sectionOf` which menu entry it highlights (`null` = none). A page only administrators may open goes into `adminOnly` (others are sent home) or shows the **Not allowed** page from `main.ts`.
2. **Page**: create `src/pages/<name>.ts` with a `render<Name>()` function that returns an `HTMLElement`. Build the DOM with `h()` from `src/utils/dom.ts`. Put the styles in `src/styles/<name>.css`, import that file at the top of the page, and use the colour tokens from `main.css`, so the dark theme works.
3. **Wire it up**: add a case to `page()` in `src/main.ts`, with the title for the browser tab.
4. **Menu entry** (only for a main section): add it to `NavSection` in `src/router.ts`, and to `NAV` (with `need` when only some users may see it), `HREF` and, for the icon, `ICONS` in `src/components/shell.ts`. The entries of `NAV` are listed above the divider of the menu; the About entry below it is built separately.
5. **Texts**: a new catalog `src/i18n/catalog/<name>.ts` (section 3.1), in all three languages.
6. **State**: pages never import each other. Shared state lives in `src/state/store.ts`; changes go through functions in `src/state/actions.ts` (`navigate(route)`, `render()`, …). State that only this page needs stays in variables of the page module. `onRouteChange()` runs code when the user leaves or enters a page.
7. **API**: a new endpoint gets a line in the route table `ROUTES` of `backend/src/api/auth.rs` (who may use it), a typed wrapper in `src/api/` and its types in `src/types/api.ts`, and is documented in [api.md](api.md) and [openapi.yaml](openapi.yaml).

## 4. Full stack

```bash
# Build and start the two containers (same xexam_data volume! Your own data will be used; to experiment, use another project name: -p some-other-name)
docker compose -f docker/docker-compose.build.yml up -d --build

# The check scripts all use temporary project names and temporary volumes, and never touch xexam_data
docker build -f nginx/Dockerfile -t xexam-frontend:test .
python -I tests/integration/nginx_check.py xexam-frontend:test 18080   # 38 checks, using a fake backend
python -I tests/integration/stack_check.py 18090                      # starts the full stack (builds both images): sign-in, import,
                                                                      # attempts, persistence, WAL, the System log, reset-password, docker stop
python -I tests/integration/migrate_check.py backend/target/debug/xexam-backend   # progress.json migration, WAL and the DELETE opt-out (run cargo build first)
```

Each script prints its checks and `All N checks passed` at the end. The last argument of `nginx_check.py` and `stack_check.py` is the host port they use; pick one that is free on your machine.

The compose files and `.env` are in `docker/`; `docker/docker-compose.build.yml` uses the repository root (`..`) as its build context. The frontend image is built from the repository root. `nginx/Dockerfile` copies the three READMEs into its Node build stage (`COPY README.md /` and `COPY README/README.ja.md README/README.zh-CN.md /README/`), because the About page's changelog is built from them; for the same reason `.dockerignore` keeps `README.md` and `README/` in the build context. Only the built static files go into the final image.

## 5. Comparing against the legacy version (regression)

The old Python implementation has been removed from the repository, but its behaviour is pinned in `tests/baseline/` (all of it fictional, synthetic data):

| Snapshot | Content | Used by |
|---|---|---|
| `golden/NNN_*.json` (172 files) | Real responses of the legacy version for every endpoint × every state (status code, stable response headers, normalized body) | `backend/tests/api_golden.rs` replays them in order against the new routes and compares each item |
| `fixtures/*.xlsx` + `parser_golden/*.json` (49) | Synthetic workbooks and the legacy parser's output for each | `backend/tests/parser_golden.rs` (the 3- and 4-column layouts must stay byte for byte the same) |
| `golden/id_vectors.json` (85 assertions) | Assertions about question ids; each entry carries the bytes that were hashed | `backend/tests/id_vectors.rs` |
| `fixtures/sample.db` + `golden/store_sample_outputs.json` | A sample database written by the legacy implementation, and the output of each of its interfaces | `backend/tests/db_golden.rs` (also checks its migration to the current schema) |
| `golden/allowed_differences_rust.json` (23 entries), `allowed_probe_differences_rust.json` (51 entries) | Places where the new implementation intentionally differs from the legacy version, with a one-sentence reason for each | Any difference in `api_golden.rs` that is not registered = failure |

- To **change** behaviour (for example, to add an intentional difference): change the code, then register the difference in `allowed_differences_rust.json` with its reason; do not edit the golden responses directly.
- The old capture / comparison / probe scripts depended on the old Python code and were removed with it; if you need them, recover them from the Git tag `python-v0.2` and the commit history.

## 6. Data safety rules

- **Do not** read, print, commit or modify `data/`, `progress.json`, or your own question-bank Excel files.
- Always experiment on a copy or in a temporary directory; do not run "let's see what happens" commands against the real `xexam.db`. Remember that opening an older database with a newer version migrates it (and switches it to WAL).
- Do not run `docker compose down -v`.
- Examples in code, tests and docs use made-up content only, never real exam questions (the demo banks "General knowledge" and "Kitchen basics", or the made-up six-column example in `frontend/src/pages/banks.ts`).

## 7. CI

There is only one pipeline: [`.github/workflows/ci.yml`](../../.github/workflows/ci.yml). It runs on pushes to `main` and `dev`, on pull requests into `main`, and when started by hand.

| Job | What it does |
|---|---|
| `check` | `VERSION` has **exactly one** `CODE_VERSION=` and **one** `RELEASE_VERSION=` line, both like `0.7` or `0.6.5`, and `RELEASE_VERSION` is not newer than `CODE_VERSION`; the newest changelog entry of each of the three READMEs = `CODE_VERSION`; the release badge at the top of each README = `RELEASE_VERSION`; `backend/Cargo.toml` and `frontend/package.json` carry `CODE_VERSION`; the layout (only `README.md` at the root, the other two READMEs in `README/`, the compose files and `.env.example` in `docker/`) and the docs check (section 8). Its output `version` (`CODE_VERSION`) is what the later jobs use |
| `frontend` | Type check, 119 unit tests, build |
| `backend` | fmt, clippy, 302 tests, and the `progress.json` migration check |
| `stack` | Builds the `xexam-frontend` image and runs the 38 Nginx checks; starts the full compose stack and runs the end-to-end checks |
| `arm64` | Builds both images for arm64, starts them under QEMU, and checks that `/api/info` reports `aarch64` and `CODE_VERSION` |
| `publish` | `main` only: pushes both images (amd64 + arm64) with the tags `latest`, `v<CODE_VERSION>` and `sha-<commit>` to Docker Hub (`docker.io/yuminspace/`, public, what `docker/docker-compose.yml` pulls) and to ghcr.io (private mirror), and verifies the manifest and tags on both. Needs the repository secrets `DOCKERHUB_USERNAME` and `DOCKERHUB_TOKEN` (optionally the variable `DOCKERHUB_NAMESPACE`); without them it stops before pushing anything |
| `release` | `main` only: creates (or updates) the GitHub Release `v<CODE_VERSION>`, with the notes taken from the changelog in `README.md` (`scripts/release-notes.py`). A run of a version older than the release already recorded on `main` does not mark its Release as the latest. Its last step records the release (section 7.1) |

Pushes to `dev` and pull requests into `main` run only the first five jobs and **never publish**; only a push to `main` does. A run started by hand behaves like a push to the branch it runs on.

### 7.1 After a release: the bot commit

Only when the images are published and the GitHub Release exists does the last step of the `release` job move `RELEASE_VERSION`:

1. On the newest `origin/main` it runs `scripts/set-release-version.py <version>`, which sets `RELEASE_VERSION` in `VERSION` and the release badge at the top of the three READMEs (its alt text, URL and width).
2. It commits that as **`Record release v<version>: RELEASE_VERSION and the README badge [skip ci]`** and pushes it to `main`. A push with the workflow's own token starts no new run, and the message says `[skip ci]` as well. When nothing changes (a re-run), it commits nothing.
3. If `dev` has no commits of its own, it fast-forwards `dev` to the same commit. If `dev` has moved on (or moves during the release), it leaves `dev` alone and shows a **notice** in the run: `merge main into dev (git pull origin main on dev) before the next merge to main`. The release is complete either way.

`set-release-version.py` never moves `RELEASE_VERSION` back: a re-run of an older run's release job changes nothing and exits 0. It refuses a version newer than `CODE_VERSION`.

So after each release, `origin/main` (and usually `origin/dev`) has one commit more than your clone:

- **Before your next push**, update both branches: `git checkout main && git pull`, then `git checkout dev && git pull`.
- **When the run showed the notice**, merge main into dev before the next merge to main: on `dev`, `git pull origin main`, then push `dev`. Otherwise the next merge into `main` misses the bot commit or conflicts on `VERSION` and the badges.

## 8. Docs

- One folder per language: `docs/en` (English, the source), `docs/ja` (Japanese) and `docs/zh-CN` (Simplified Chinese). The same seven documents exist in each: `api`, `architecture`, `database`, `deployment`, `development`, `migration`, `security`.
- `docs/zh-CN/history/` holds older documents from the switch to Rust (audits, plans, risk assessment). They are Chinese only and are not updated any more.
- `docs/en/openapi.yaml` is shared by all languages; the Japanese and Chinese docs link to `../en/openapi.yaml`.
- Every document starts with a language bar that links the same document in the other two folders. Links are relative to the file's own folder: the repository root is `../../`.
- Change the English document first, then the Japanese and Chinese ones in the same change. UI labels are written in bold exactly as the catalog of that language says them; the question language switch is always **English** / **中文** / **日本語**.
- The CI `check` job fails when one of the seven documents is missing in a language folder, when `docs/` contains anything other than the `en`, `ja` and `zh-CN` folders, or when a relative link or image path in a README or a doc points at a file that does not exist. Links to GitHub pages (`../../releases` and the like) are checked too: they must climb exactly to the repository page, so inside `README/` they start with `../../../`.

## 9. Version, changelog and licence

- **`VERSION`** in the repository root holds two `KEY=VALUE` lines (lines starting with `#` are comments), each exactly once:
  - `CODE_VERSION` (`0.7` today): the version of the code on this branch. Bump it by hand, together with the changelog entry in all three READMEs, `backend/Cargo.toml` and `frontend/package.json` (`0.7.0`; also their lock files). The images are built and tagged with it, and `/api/info` reports it (the backend reads it from `/VERSION` in the image).
  - `RELEASE_VERSION` (`0.6` until the release job of v0.7 has run): the last version that was actually published. **Never edit it by hand**: the release job writes it, together with the release badge at the top of the READMEs (section 7.1). A push or a merge alone never changes it.
- Every update adds an entry at the very top of the changelog in all three READMEs (`README.md`, `README/README.ja.md`, `README/README.zh-CN.md`), with a version number equal to `CODE_VERSION`. CI checks this. The GitHub Release notes are taken from the English one.
- The About page of the app shows these changelogs, built into the frontend (section 3). Keep the entries in simple Markdown: paragraphs, `- ` bullets, nested `  - ` bullets, `**bold**` and `` `code` ``; links to files in the repository are shown there as plain text. A changed changelog appears in the app only after the frontend is built again.
- Work happens on `dev`; `main` receives the merge, and a push to `main` publishes the images and the Release. Pull `main` and `dev` after each release (section 7.1).
- The project is under the MIT licence (`LICENSE` in the repository root).
