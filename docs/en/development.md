**English** | [日本語](../ja/development.md) | [简体中文](../zh-CN/development.md)

# Development

> Commands for everyday code changes and test runs, and where things live. For the architecture see [architecture.md](architecture.md); for the original testing approach see [test-strategy.md](../zh-CN/history/test-strategy.md).

## 1. Tools

| Tool | Version | Notes |
|---|---|---|
| Rust | 1.99.1 (pinned by `backend/rust-toolchain.toml` and `ai-service/rust-toolchain.toml`) | `rustup` installs it automatically. The `bundled` feature of `rusqlite` (and `ring` in ai-service) needs a C compiler (Linux: `build-essential`; Windows: MinGW gcc or MSVC) |
| Node | 24 | Frontend; the unit tests use Node's built-in test runner with `--experimental-strip-types` |
| Python | 3.8+ | Only for the integration scripts in `tests/integration/` (with the made-up AI provider `ai_stub.py`), the two scripts in `scripts/`, the icon tool in `tools/icons/` (**standard library only**) and the screenshot tool in `tools/screenshots/` (standard library plus `openpyxl`) |
| Docker | Compose v2 | Whole-stack checks, images, the screenshot tool |
| Chrome or Chromium | any recent | Only for the screenshot tool and the icon tool (sections 9.1 and 9.2) |

## 2. Backend

```bash
cd backend
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test --all-targets          # 382 tests
cargo run -- serve --data ../data --no-browser     # listens on 127.0.0.1:8765
```

- Tests run against the **legacy snapshot** committed in `tests/baseline/`; Python is not needed for them.
- All tests use a fresh database in a temporary directory or a copy of `tests/baseline/fixtures/sample.db`, and **never touch `data/`**. Besides the unit tests in `src/`, the test files are: `api_basic.rs`, `api_import.rs`, `api_misc.rs` and `api_golden.rs` (routes, guards, the legacy responses), `db_golden.rs` (the database against the legacy snapshot), `parser_golden.rs`, `parser_hardening.rs`, `parser_four_columns.rs`, `parser_six_columns.rs` and `id_vectors.rs` (the parser), and one file per version: `v05_banks_records_settings.rs` (schema v2, banks, records, settings), `v06_users_and_sign_in.rs` (schema v3, accounts, sessions, the login throttle), `v065_bank_access_and_practice.rs` (schema v4, bank access, bank managers, ownership, practice), `v07_bank_access_and_layouts.rs` (the chosen banks and **Include banks imported later**, the conversion of pre-release v4 databases, the refusal of the old layouts and the retirement of their questions), `v07_wal_and_logs.rs` (WAL mode, the checkpoint, `reset-password` while the server runs, the System log), `v07_log_floods.rs` (floods of rejected requests and failed sign-ins are summed up), `v08_ai_link.rs` (schema v5, the AI+ feature grant, the internal token, the auth step and the internal routes; one test runs the real binary) and `v09_ai_study.rs` (schema v6 and its backup, the session `key` and `startedAt` of a saved record and its `recordId`, and the three study routes of the internal link: session binding, the AI+ check, scope, caps, the random sample of wrong answers, one identical "not found").
- Tests sign in through helpers; no switch turns sign-in off, also not for tests.
- **The old layouts in tests.** Production refuses three- and four-column workbooks (`services::check_layout`). The golden suites replay the old version's imports, which are three-column files, so the API state they build sets `old_layouts: OldLayouts::AllowForTests` (in `backend/tests/api_common/mod.rs`, and in `v05_banks_records_settings.rs` and `v065_bank_access_and_practice.rs`, which import the committed three-column fixture `tests/baseline/fixtures/demo.xlsx` through the API). It is a field of the API state that only test code sets: `AppState::new` and the `import` command always use `OldLayouts::Refuse`, and no environment variable, flag or setting can change it. `v07_bank_access_and_layouts.rs` tests the production rule. The store methods themselves accept any question, so the database golden tests run unchanged.
- Common environment variables: `XEXAM_DATA`, `XEXAM_PORT`, `XEXAM_ALLOWED_HOSTS`, `XEXAM_LOG` (log level, e.g. `debug`), `XEXAM_SQLITE_JOURNAL` (`delete` keeps the rollback journal instead of WAL), `XEXAM_COOKIE_SECURE`, `XEXAM_LINK_DIR` and `XEXAM_INTERNAL_TOKEN` (the link with ai-service, section 3). For the full list see `xexam-backend --help` and [deployment.md](deployment.md).
- `serve` creates `<data dir>/link/internal.token` (or `XEXAM_LINK_DIR/internal.token`) when it is missing; the other commands never touch it.
- The first start on an empty data directory creates the administrator `admin` / `admin`; the page asks for a new password at the first sign-in. If you forget it: `cargo run -- reset-password admin --data ../data` (it works while the server runs).
- Command-line import of a six-column workbook (into the legacy bank): `cargo run -- import ../some.xlsx --data ../data`. A three- or four-column workbook is refused.
- Pointing the backend at a database of an older schema migrates it and writes `backups/` next to it. Use a copy. The v0.9 study routes (`api/internal_study.rs`, the projections in `database/store/study.rs`) are token routes: the backend answers them only to the AI service and writes nothing and logs nothing. The backend writes no Chinese literal outside test code for them either (`server-messages.test.ts` scans `backend/src`).
- The database runs in WAL mode: while the backend runs, `xexam.db-wal` and `xexam.db-shm` sit next to `xexam.db`. Copy all three, or stop the backend first (everything is then in `xexam.db`; a `-wal` file left next to it is empty). `sqlite3` can read the database while the backend runs.

## 3. AI service (v0.8; study features v0.9)

The AI service is its own crate in `ai-service/` (package and binary `xexam-ai-service`, library `xexam_ai_service`, no workspace, `unsafe` forbidden), with its own `Cargo.lock` and `rust-toolchain.toml` (Rust 1.99.1, like the backend).

```bash
cd ai-service
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test --all-targets          # 383 tests (and 1 doc test, `cargo test --doc`: the hint prompt must not accept a solution)
cargo tree -e normal --target all | grep -E 'aws-lc|openssl-sys|native-tls'   # must print nothing (the TLS guard of CI)
```

- **Layout** (section 2 of [architecture.md](architecture.md) has the full list): `src/api/` (the dispatch, the user and the administrator routes), `src/chat.rs` (the task that writes an answer, the per-user and global places, cancelling), `src/app.rs` (the shared state and the heartbeat to the backend), `src/provider/` (the OpenAI-compatible and Anthropic adapters, the event-stream parser, the error mapping), `src/store/` (the AI database), `src/identity.rs` and `src/link.rs` (the identity statement and the calls to the backend), `src/secret.rs` and `src/settings.rs` (key encryption; providers, the URL policy, the fingerprint), `src/limits.rs`, `src/prompt.rs`, `src/retention.rs`, `src/logs.rs`, `src/codes.rs` (every Chinese text of the service), `src/config.rs`. Since v0.9 also `src/api/study.rs` (the four study routes), `src/study/` (the prompts `hint.rs`, `explain.rs`, `summary.rs` and their rendering, the hint guard in `guard.rs` and `guard/{lists,norm,rules}.rs`, `title.rs`, the strict wire types of the backend's data routes in `wire.rs`, the record-identity cache in `cache.rs`) and, in `src/store/`, the study chat functions of `data.rs` and the usage by purpose of `stats.rs`.
- **Tests** (`ai-service/tests/`, against the library, with an in-process fake provider and a fake backend that also records the heartbeats; none touches the network): `streaming.rs` (events, Stop, Retry, time limits, detaching, the gate and the limits while calls run), `settings_and_keys.rs` (validation, key re-entry, the header clash, encryption, the key file and `XEXAM_AI_SECRET_KEY`), `identity_access.rs` (the statement, the access levels, scoping, the token rules), `admin_data.rs` (the administrator's views and clears, the audit lines and the outbox, usage, reconcile), `logs_no_secrets.rs` (no key, token, prompt or answer in any log line) and `proxy_env.rs` (`HTTP_PROXY` in the environment is ignored); for the second round of v0.8 `r2_core_heartbeat.rs` (the report, its timing and the 60-second rule), `r2_core_schema.rs` (schema 2, the schema-1 backup and upgrade, the columns added to a schema-2 file that lacks some), `r2_core_suspend.rs` (Pause / Resume, audit lines, stopping answers), `r2_chat_lang.rs` (the reply language), `r2_chat_free.rs` (free mode as a state of the administrator: `POST /api/ai/free`, `freeMode`, no per-chat flag), `r2_free_history.rs` (the whole history in both modes, the system prompt of each mode, the mode line after a switch, `free` on every message), `r2_cache.rs` (prompt caching, cached tokens in the usage row, the shortening of old history messages), `r2_chat_thinking.rs` (the `thinking` event; the reasoning text must not appear in the stream, the database or the log) and `r2_chat_suspend.rs` (the status and the re-check); for v0.9 `v09_store.rs` (schema 3: a fresh file, the upgrade from schema 1 and 2 with its backup, a refused backup, the same `sqlite_master` on both paths, reopening writes nothing, `EXPLAIN QUERY PLAN` of the saved lookups, study chats and the ask transaction, usage by purpose, retention batches), `study_guard.rs` (the hint guard: letters, ordinals, hotspot statements, the key text, the lists, `<think>` tags, the same verdict for every chunking of a stream, the limits, the persistence rule; modules in `tests/study_guard/`), `study_prompts.rs` (the prompts, the `compile_fail` doc test) and `study_api.rs` (the study routes end to end against a fake backend: streaming, the identity of chats, saved lookups, bounds, summary refusals, access, administrator views, logs without content, the regenerate note and the cache bypass of Cloudflare AI Gateway, also for an AI Chat retry; modules in `tests/study_api/`); plus the unit tests in `src/`. Some `study_api` tests rely on short sleeps (200 ms and 120 ms) and are stable but timing based. The `TestApp` helper shuts the service down and releases the database file when it is dropped, so a test leaves nothing in the temporary folder (on Windows an open file would keep its directory).
- **Running it locally** next to the backend. The two must share the internal token: either the token file the backend writes into its link directory (by default `link` inside its data directory), or the same `XEXAM_INTERNAL_TOKEN` in both shells.

  ```bash
  # 1. the backend (writes ../data/link/internal.token)
  cd backend && cargo run -- serve --data ../data --no-browser
  # 2. ai-service: its data in ai-service/data-ai (ignored by Git), the token from the backend's link directory
  cd ai-service && XEXAM_LINK_DIR=../data/link cargo run
  # 3. optional, a made-up provider: in AI settings use the address http://127.0.0.1:9000/v1, a key like sk-demo-1234 and the model demo-chat-small
  python -I tests/integration/ai_stub.py --port 9000
  # 4. the frontend: /api/ai/ goes to ai-service, after the vite plugin asked the backend (as Nginx does)
  cd frontend && npm run dev
  ```

  ai-service listens on `127.0.0.1:8766` (`XEXAM_AI_HOST`, `XEXAM_AI_PORT`) and calls the backend at `http://127.0.0.1:8765` (`XEXAM_AI_BACKEND_URL`, plain http only). `xexam-ai-service --help` lists every variable. As soon as it has the token it reports to the backend (the heartbeat, every 25 seconds; section 5 of [architecture.md](architecture.md)); until the backend has seen a report, User management shows no AI+ feature switches and AI settings says that ai-service is waiting for its first report, so a few seconds after starting both is normal. Without a token it answers every AI route with 503 `ai.backendDown` and logs `AI link: waiting for the internal token at …` until the file appears. `ai_stub.py` (standard library only) also offers an Anthropic route (`/anthropic`), gateway routes (`/gw/openai`, `/gw/anthropic`, with a gateway key like `gw-demo-…`) and special models for the error paths, the thinking state (`demo-reasoning`, which sends made-up reasoning first) and the non-chat marks (`demo-embedding`, which is listed but refuses a chat); it answers in the language of the "Always reply in …" line of the system prompt. Its docstring lists them.
- **Study features** (v0.9): the stub provider of the tests and of `tests/integration/ai_stub.py` answers by the system prompt: `asked for a HINT` gives a clean self-made hint (a user turn with `[leak]` ends in a sentence that names the answer, for the guard tests), `asked for an EXPLANATION` gives `**Answer: <letters>**` and text, `answered wrong` gives four headings and `Items: <n>`. To change what the guard withholds, edit the lists in `guard/lists.rs` and add the sentence to `tests/study_guard/`; the hint prompt takes no `Solution` and must keep it that way (the doc test pins it). A new data field of the backend's study routes needs the same change in `backend/src/api/internal_study.rs`, `src/study/wire.rs` (it refuses unknown keys) and the contract tests on both sides.
- **Rules**: every Chinese text of the service is in `src/codes.rs` (the frontend translates by code, so add an `ai.err.*` key in `frontend/src/i18n/catalog/ai.ts` with a new code). Log lines are English, name users by `{:?}` of the username, and never contain a key, the internal token, an identity statement, a header value, a base URL, a prompt, an answer, a chat title or a memory note; `logs_no_secrets.rs` checks this. Types that hold a key or the token have no derived `Debug`. Request bodies are validated by hand (unknown keys refused); keep the JSON keys in the documented order (`serde_json` with `preserve_order`).
- **The backend's side** of the link is `backend/src/api/internal.rs` (the token, the identity statement, `/internal/ai/*` including the heartbeat route and the in-memory record of the last report), `backend/src/api/internal_study.rs` (v0.9: `question`, `record` and `record-info`) and `api::auth::introspect` (the auth step); its tests are in `backend/tests/v08_ai_link.rs` and `v09_ai_study.rs`.

## 4. Frontend

```bash
cd frontend
npm ci
npm run typecheck      # tsc for the app, then for the test files (tsconfig.test.json)
npm test               # unit tests: the API client (Nginx's 429 and its retries, time limits, the AI stream and error
                       # codes), grading and random pick, practice checking, saved results, language rules, records filter,
                       # lock notes and waits, import warnings, the System log buffer, bank status, bank access, saved exam,
                       # settings cache, the event-stream parser, the AI chat state, the answers' Markdown, memory requests,
                       # the AI settings rules (including the connection causes and the model kinds), the AI status, the badge states, the User management AI+ rules, the router (`#/ai/<id>`, `#/ai/new`), UI catalogs (the exact words of the owner's wording decisions), server-message translation, changelog parsing, and for v0.9 the AI+ study features (who sees which control, the state of an answer, the engine with fake timers and transport, the popover rules, the request builders, the focus target, session keys)
npm run build          # type check + vite build → dist/
npm run dev            # http://127.0.0.1:5173, /api is proxied to http://127.0.0.1:8765, /api/ai/ to http://127.0.0.1:8766
```

- While developing, open the page via `localhost` or `127.0.0.1`. The Vite proxy's `changeOrigin` must be `false`: the backend compares `Host` with `Origin`, and rewriting `Host` makes write requests fail with 403.
- When the backend is not at the default address: `XEXAM_BACKEND=http://127.0.0.1:9000 npm run dev`; ai-service: `XEXAM_AI_BACKEND=http://127.0.0.1:9001`.
- **AI requests in development** (v0.8): the plugin `xexamAiAuth()` in `vite.config.ts` does what Nginx does in production, in `npm run dev` and `npm run preview`. For every request whose path starts with `/api/ai/` it first asks the backend (`GET /internal/ai/auth` with the cookie and `X-Original-Host`, `X-Original-Origin`, `X-Original-Method`), then removes `Cookie`, `Authorization` and `Cf-Access-Jwt-Assertion` and sets the identity statement the backend returned (a header the browser sent is always replaced). The backend's 401 / 403 are passed through; anything else (the backend down) becomes 503 `ai.backendDown`. `/api/ai` without the slash goes to the backend, as with Nginx.
- The About page shows the changelogs `docs/en/changelog.md`, `docs/ja/changelog.md` and `docs/zh-CN/changelog.md`. The `xexam-changelog` plugin in `vite.config.ts` reads them from the repository root (the folder above `frontend/`) when the frontend is built (and in `npm run dev` when the About page is first opened), so they must be there. The build stops with an error when one of them has no changelog entry (`### vX.Y — date`).
- To run one test file: `node --experimental-strip-types --test src/i18n/server-messages.test.ts`.
- Import paths use the `.ts` extension (required by Node's built-in test runner).
- TypeScript is pinned to `~5.9.3`.

### 4.1 Interface texts

Every text the user sees comes from a catalog. Nothing is written into a page directly.

- **Where**: `src/i18n/catalog/<area>.ts`, one file per area: `common`, `home`, `banks`, `records`, `settings`, `quiz`, `result`, `about`, `auth`, `users`, `logs`, `ai`, `aiplus` (v0.9, the study features). Each file is `export default defineCatalog({ en: {…}, zh: {…}, ja: {…} })`.
- **Keys** start with the area: `banks.delete.title`, `records.deleteAll.ok`. `common.ts` holds the shared prefixes (`app.`, `nav.`, `common.`, `err.`, `kind.`, `mode.`, `act.`, `role.`, `forbidden.`, …).
- **Three languages, always.** TypeScript refuses a `zh` or `ja` block that misses an English key. A key ending in `_one` is the English singular; `tn()` uses it when the count is 1, and it is optional in Chinese and Japanese.
- **Placeholders** are `{name}`: `t('records.deleteOne.body', { at, pct })`. For counts use `tn(key, n)`, which fills `{n}`.
- **Checks**: `src/i18n/catalog.test.ts` makes sure all three languages have exactly the same keys and the same placeholders, no text is empty, every key carries its area prefix and exists in one catalog only, and every catalog file is registered in `src/i18n/index.ts`.
- **A new catalog file**: import it in `src/i18n/index.ts`, add it to `ALL` and to the `MsgKey` type.
- Write English first, then Chinese and Japanese. Use the terms the UI already uses (for example "question bank", "retired", "wrong-answer notebook", "bank manager").
- The question / answer language switch is the exception: its three buttons always say **English**, **中文** and **日本語** (`src/components/lang-switch.ts`), whatever the interface language; only its accessible name and tooltips come from the catalog.

### 4.2 Backend messages in the UI

Backend error messages are Chinese and stable (they are part of the tested API). The frontend translates them:

- `src/i18n/server-messages.ts`: `EXACT` maps whole messages, `PATTERNS` maps the backend's format strings (their `{}` parts become captured values). Chinese UI shows the message unchanged; unknown text is shown as it is. A few low-level protocol errors are English words (`not found`, `bad json`, …); they get a sentence in every language, Chinese included.
- `src/i18n/errors.ts`: `errorText(e)` turns any error into text in the current language. Pages use it for every error they show. A page's own error is a `LocalError(key, params)`, so it changes language with the UI.
- **Drift test**: `src/i18n/server-messages.test.ts` reads the Rust sources in `backend/src` with a small lexer. Every string literal with Chinese or Japanese characters (outside comments, test code, assertions, log and console output) must have an English and a Japanese translation, keep the values of its placeholders and contain no Chinese. It also fails on translations whose backend text no longer exists.
- **When you add or change a backend message**: add the translation in `server-messages.ts` in the same change. If the string is not shown to the user (for example a word the parser looks for), add it to `NOT_MESSAGES` in the test, with the reason.
- **Log lines stay English** and must never contain a password, a session token, a cookie value or question text (`backend/tests/v07_wal_and_logs.rs` checks the log after a whole session). Lines that anyone can cause without signing in go through a `BurstLog` (`backend/src/logs.rs`), not straight to `tracing`.
- **AI errors** are not translated by text: the AI service gives each error a stable `code`, and `errorText()` uses the key `ai.err.<code without "ai.">` of `src/i18n/catalog/ai.ts`, filling `{n}` from the error's `limit`. A new code of the AI service needs that key in all three languages.
- **The AI+ study controls** (v0.9) are built by one component, `src/components/ai-plus-controls.ts`; a page asks it for a control and never builds an AI element itself, so the rules (nothing without the AI+ feature, greyed with the tag and the reason when AI is not usable, no control on the exam page) stay in one place and in `src/domain/ai-plus.ts`. The popover (`ai-pop.ts`) and the summary window (`ai-summary.ts`) are installed once from `main.ts`, together with `reanchorAiPop()` after every page redraw and the sign-out hook that forgets their memory. The decisions are pure and have unit tests; the logic of one answer is the reducer in `ai-study.ts`, and `ai-run.ts` is the engine with an injected transport and timers. The answers are Markdown through the same safe renderer as AI Chat (`ai-render.ts`, text nodes only); the request error text of the study features is `studyErrorText` in `src/i18n/errors.ts`.

### 4.3 Adding a page or route

1. **Route**: in `src/router.ts`, add a variant to `Route`, a case to `parseHash` and, if its hash is not `#/<name>`, to `routeHash`. Say in `sectionOf` which menu entry it highlights (`null` = none). A page only administrators may open goes into `adminOnly` (others are sent home) or shows the **Not allowed** page from `main.ts`.
2. **Page**: create `src/pages/<name>.ts` with a `render<Name>()` function that returns an `HTMLElement`. Build the DOM with `h()` from `src/utils/dom.ts`. Put the styles in `src/styles/<name>.css`, import that file at the top of the page, and use the colour tokens from `main.css`, so the dark theme works.
3. **Wire it up**: add a case to `page()` in `src/main.ts`, with the title for the browser tab.
4. **Menu entry** (only for a main section): add it to `NavSection` in `src/router.ts`, and to `NAV` (with `need` when only some users may see it), `HREF` and, for the icon, `ICONS` in `src/components/shell.ts`. The entries of `NAV` are listed above the divider of the menu; the About entry below it is built separately.
5. **Texts**: a new catalog `src/i18n/catalog/<name>.ts` (section 4.1), in all three languages.
6. **State**: pages never import each other. Shared state lives in `src/state/store.ts`; changes go through functions in `src/state/actions.ts` (`navigate(route)`, `render()`, …). State that only this page needs stays in variables of the page module. `onRouteChange()` runs code when the user leaves or enters a page.
7. **API**: a new endpoint gets a line in the route table `ROUTES` of `backend/src/api/auth.rs` (who may use it), a typed wrapper in `src/api/` and its types in `src/types/api.ts`, and is documented in [api.md](api.md) and [openapi.yaml](openapi.yaml). An AI route goes into `ROUTES` of `ai-service/src/api/mod.rs` instead, with its access level.

### 4.4 Phone-first rules (v1.0)

- **One phone query.** `PHONE` is `(max-width: 600px), (orientation: landscape) and (max-height: 500px)`. Its TypeScript constant is `PHONE_MQ` in `src/utils/media.ts`; every stylesheet repeats the same text in its own `@media` block, because CSS cannot import a constant. Change both together. The other named queries (`SHORT_LANDSCAPE_MQ`, `BAR_MQ`, `TOUCH_MQ`) are in the same file.
- **Icons with labels.** A control that shows only an icon is built with `iconButton()` (`src/components/icon-button.ts`, styles in `src/styles/icon-button.css`); its shape comes from `src/components/mini-icons.ts` (`src/domain/mini-icon-shapes.test.ts` checks the shapes). The text stays its accessible name, and on touch it shows as a tap-and-hold tip (`src/domain/hold-tip.ts`).
- **Touch targets** are at least 44 × 44 px; a 36 px icon may stay visually smaller with an invisible enlarged area.
- **Focus.** A title or other non-interactive element that a script focuses gets no focus ring (`[tabindex="-1"]:focus`); real controls keep the ring.
- **Width.** Check a page at the width matrix in section 5.1 (360, 390, 412 portrait; 740 and 844 landscape; 768 tablet; 1280 desktop) in zh, en and ja, light and dark, before you call the page done. Never add a fixed width that breaks at 360 px.

## 5. Full stack

```bash
# Build and start the three containers (same xexam_* volumes! Your own data will be used; to experiment, use another project name: -p some-other-name)
docker compose -f docker/docker-compose.build.yml up -d --build

# The check scripts all use temporary project names and temporary volumes, and never touch the xexam_* volumes
docker build -f nginx/Dockerfile -t xexam-frontend:test .
python -I tests/integration/nginx_check.py xexam-frontend:test 18080   # 141 checks, with a fake backend and a fake ai-service
                                                                      # (rate limits, the auth step, header removal, streaming)
python -I tests/integration/stack_check.py 18090                      # 314 checks: the full stack (builds the three images) with the
                                                                      # made-up AI provider: sign-in, import, attempts, AI settings,
                                                                      # streamed answers, the AI+ feature and its pause, the
                                                                      # heartbeat and docker stop, reply language, free mode (per user),
                                                                      # the AI+ study features (hints, explanations, summaries, the chats and
                                                                      # their titles, saved answers, the busy flow, nothing logged),
                                                                      # persistence, WAL, the System log,
                                                                      # reset-password, docker stop, what each volume holds
python -I tests/integration/migrate_check.py backend/target/debug/xexam-backend   # 78 checks (80 on Linux): progress.json migration, WAL and the DELETE
                                                                      # opt-out, schema 3 → 6, 4 → 6 and 5 → 6, the internal token (run cargo build first)
```

Each script prints its checks and `All N checks passed` at the end. The last argument of `nginx_check.py` and `stack_check.py` is the host port they use; pick one that is free on your machine. `stack_check.py` reads an optional `XEXAM_STACK_COMPOSE`: another compose file of the same shape (for example a copy of `docker/docker-compose.build.yml` with other image tags and container names), so it can run next to an installation that uses the usual ones; it finds the containers through their compose services. `nginx_check.py` takes about two and a half minutes, because one check waits 95 seconds for a slow **Test** of AI settings. `stack_check.py` paces its study asks (about 3.2 seconds per session cookie), because Nginx lets one session start an AI answer every 3 seconds after a burst of 10 (zone `aisend`, 20 a minute); that adds about a minute and a half. The scripts use made-up banks that they write themselves; none touches your data.

The compose files and `.env` are in `docker/`; `docker/docker-compose.build.yml` uses the repository root (`..`) as its build context. The frontend image is built from the repository root. `nginx/Dockerfile` copies the three changelogs into its Node build stage (`COPY docs/en/changelog.md /docs/en/`, and the same for `ja` and `zh-CN`), because the About page is built from them; for the same reason `.dockerignore`, which keeps `docs/` and the READMEs out of the build context, lets exactly these three files back in. Only the built static files go into the final image.

### 5.1 The phone walk (v1.0)

`tools/phone-walk/walk.py` starts its own throw-away stack (the three images under unique names, with their own volumes, `TZ=UTC`, the made-up AI stub and free ports on 127.0.0.1), drives Chrome with phone emulation over the DevTools protocol, and writes a `report.json` and screenshots into its `--out` folder. It removes only what it created. It needs Docker, Chrome or Chromium, and Python 3.8+ with `openpyxl`, like the screenshot tool (`tools/phone-walk/README.md`).

```sh
python tools/phone-walk/walk.py                      # default run: report and screenshots in a new temporary folder
python tools/phone-walk/walk.py --full               # every check on the whole width matrix (final gate only)
python tools/phone-walk/walk.py --pages home,quiz,result,ai,menu   # only these pages
python tools/phone-walk/walk.py --only W-QZ-*,W-CH-1 # subsets, by check ID (reviewers)
python tools/phone-walk/walk.py --langs zh --widths 390x844,360x740 --themes light
python tools/phone-walk/walk.py --keep               # leave the stack running; prints its URL and the made-up accounts
python tools/phone-walk/walk.py --keep --lan         # the same, for a phone on the home network (warns first)
python tools/phone-walk/walk.py --out <dir> --chrome <path>
```

- **Width matrix**: 360×740, 390×844 and 412×915 portrait; 740×360 and 844×390 landscape; 768×1024 tablet; 1280×800 desktop. Light and dark themes; zh first, then en and ja.
- **Default run**: contrast (light and dark, zh, 390×844 and 1280×800); text fit (the rows that name it, in zh, en and ja, light); every other check in zh, light, at the sizes its row names. `--full` runs everything on the whole matrix and is for the final gate only.
- **Output**: `report.json` (one entry per check: ID, page, state, user, size, language, theme, pass, detail, screenshot) and screenshots named `<check>-<page>-<user>-<lang>-<theme>-<w>x<h>.png`. The exit code is 1 when a P1 check fails. Give each run its own `--out` folder when several run at once.
- **Data**: made-up accounts and banks only (`seed_extra.py`, `seed.py`); it never connects to another instance and never prints a key. `--lan` is for a short trial on your own phone: every device on the network can reach that stack until Ctrl+C.

## 6. Comparing against the legacy version (regression)

The old Python implementation has been removed from the repository, but its behaviour is pinned in `tests/baseline/` (all of it fictional, synthetic data):

| Snapshot | Content | Used by |
|---|---|---|
| `golden/NNN_*.json` (172 files) | Real responses of the legacy version for every endpoint × every state (status code, stable response headers, normalized body) | `backend/tests/api_golden.rs` replays them in order against the new routes and compares each item |
| `fixtures/*.xlsx` + `parser_golden/*.json` (49) | Synthetic workbooks (three- and four-column, the layouts of the old version) and the legacy parser's output for each | `backend/tests/parser_golden.rs` (the parser must still read them byte for byte the same; that is how an import recognises and refuses them) |
| `golden/id_vectors.json` (85 assertions) | Assertions about question ids; each entry carries the bytes that were hashed | `backend/tests/id_vectors.rs` |
| `fixtures/sample.db` + `golden/store_sample_outputs.json` | A sample database written by the legacy implementation, and the output of each of its interfaces | `backend/tests/db_golden.rs` (also checks its migration to the current schema) |
| `golden/allowed_differences_rust.json` (23 entries), `allowed_probe_differences_rust.json` (51 entries) | The places where the Rust implementation intentionally differed from the legacy version when it replaced it, with a one-sentence reason for each (in Chinese); a record, no test reads them | [api.md](api.md) section 7 |

- `api_golden.rs` sorts every capture into one of five outcomes and prints the table with `cargo test --test api_golden -- --nocapture`. Today: 84 matched, 58 matched with the engine fed from the capture, 16 intended differences (the 7 unsupported methods that now get 405, and the 9 captures of `GET /api/info`, compared without `bankCount` since v0.7.5), 14 skipped (static files, which Nginx serves, and one capture without a response) and 0 differing. Any differing capture fails the test.
- To **change** behaviour that a capture covers: change the code and teach `api_golden.rs` the intended difference with its reason (as for `bankCount`); do not edit the golden responses.
- The old capture / comparison / probe scripts depended on the old Python code and were removed with it; if you need them, recover them from the Git tag `python-v0.2` and the commit history.

## 7. Data safety rules

- **Do not** read, print, commit or modify `data/`, `data-ai/`, `progress.json`, or your own question-bank Excel files.
- Never put a real AI key into code, tests, docs or screenshots: tests use obviously fake keys (`sk-demo-…`, `gw-demo-…`), and the examples use `example.com` addresses, such as `https://ai-gateway.example.com/<route>` for a gateway.
- Always experiment on a copy or in a temporary directory; do not run "let's see what happens" commands against the real `xexam.db`. Remember that opening an older database with a newer version migrates it (and switches it to WAL).
- Do not run `docker compose down -v`.
- Examples in code, tests and docs use made-up content only, never real exam questions (the demo banks "General knowledge" and "Kitchen basics" of `tools/screenshots/demo_banks.py`, or the made-up six-column examples in `frontend/src/pages/banks.ts`).

## 8. CI

There is only one pipeline: [`.github/workflows/ci.yml`](../../.github/workflows/ci.yml). It runs on pushes to `main` and `dev`, on pull requests into `main`, and when started by hand.

| Job | What it does |
|---|---|
| `check` | `VERSION` has **exactly one** `CODE_VERSION=` and **one** `RELEASE_VERSION=` line, both like `0.7` or `0.6.5`, and `RELEASE_VERSION` is not newer than `CODE_VERSION`; the newest entry of each of the three changelogs (`docs/en/changelog.md`, `docs/ja/changelog.md`, `docs/zh-CN/changelog.md`) = `CODE_VERSION`, and the three have the same `### vX.Y — date` headings in the same order; no README holds a changelog entry, and each links to the changelog of its language; the release badge at the top of each README = `RELEASE_VERSION`; `backend/Cargo.toml`, `ai-service/Cargo.toml` and `frontend/package.json` carry `CODE_VERSION`; the layout (only `README.md` at the root, the other two READMEs in `README/`, the compose files and `.env.example` in `docker/`), `docker/docker-compose.yml` pulling the three images from Docker Hub, and the docs check (section 9). Its output `version` (`CODE_VERSION`) is what the later jobs use |
| `frontend` | Type check, unit tests, build |
| `backend` | fmt, clippy, 382 tests, and the `progress.json` migration check |
| `ai-service` | fmt, clippy, 383 tests of ai-service, and the TLS guard: `cargo tree` must not contain `aws-lc`, `openssl-sys` or `native-tls` |
| `stack` | After `frontend`, `backend` and `ai-service`: builds the `xexam-frontend` image and runs the 141 Nginx checks; starts the full compose stack (three images, the made-up AI provider) and runs the 314 end-to-end checks (45 minutes at most) |
| `arm64` | Builds the three images for arm64, starts them under QEMU, and checks that `/api/info` reports `aarch64` and `CODE_VERSION`, and that ai-service on its own passes its health check (`xexam-ai-service healthcheck`) and reports the same in `/healthz` (60 minutes at most) |
| `publish` | `main` only: pushes the three images (amd64 + arm64) with the tags `latest`, `v<CODE_VERSION>` and `sha-<commit>` to Docker Hub (`docker.io/yuminspace/`, public, what `docker/docker-compose.yml` pulls) and to ghcr.io (private mirror), and verifies the manifest and tags on both. Needs the repository secrets `DOCKERHUB_USERNAME` and `DOCKERHUB_TOKEN` (optionally the variable `DOCKERHUB_NAMESPACE`); without them it stops before pushing anything |
| `release` | `main` only: creates (or updates) the GitHub Release `v<CODE_VERSION>`, with the notes taken from `docs/en/changelog.md` (`scripts/release-notes.py`). A run of a version older than the release already recorded on `main` does not mark its Release as the latest. Its last step records the release (section 8.1) |

Pushes to `dev` and pull requests into `main` run only the check jobs (`check` to `arm64`) and **never publish**; only a push to `main` does. A run started by hand behaves like a push to the branch it runs on.

### 8.1 After a release: the bot commit

Only when the images are published and the GitHub Release exists does the last step of the `release` job move `RELEASE_VERSION`:

1. On the newest `origin/main` it runs `scripts/set-release-version.py <version>`, which sets `RELEASE_VERSION` in `VERSION` and the release badge at the top of the three READMEs (its alt text, URL and width).
2. It commits that on `main` as **`Record release v<version>: RELEASE_VERSION and the README badge [skip ci]`** through the GitHub API (GraphQL `createCommitOnBranch`), not with `git push`, so GitHub signs the commit and it shows as **Verified**. The request names the `main` commit it started from, so the API refuses when `main` moved in the meantime. A commit made with the workflow's own token starts no new run, and the message says `[skip ci]` as well. When nothing changes (a re-run), it commits nothing.
3. If `dev` has no commits of its own, it fast-forwards `dev` to the same commit. If `dev` has moved on (or moves during the release), it leaves `dev` alone and shows a **notice** in the run: `merge main into dev (git pull origin main on dev) before the next merge to main`. The release is complete either way.

`set-release-version.py` never moves `RELEASE_VERSION` back: a re-run of an older run's release job changes nothing and exits 0. It refuses a version newer than `CODE_VERSION`.

So after each release, `origin/main` (and usually `origin/dev`) has one commit more than your clone:

- **Before your next push**, update both branches: `git checkout main && git pull`, then `git checkout dev && git pull`.
- **When the run showed the notice**, merge main into dev before the next merge to main: on `dev`, `git pull origin main`, then push `dev`. Otherwise the next merge into `main` misses the bot commit or conflicts on `VERSION` and the badges.

## 9. Docs

- One folder per language: `docs/en` (English, the source), `docs/ja` (Japanese) and `docs/zh-CN` (Simplified Chinese). The same nine documents exist in each: `api`, `architecture`, `changelog` (since v0.9.1; before, it was in the READMEs), `database`, `deployment`, `development`, `migration`, `security` and, since v0.7.5, `usage` (how to use the app, moved out of the README), each folder with its own `screenshots/`.
- `docs/zh-CN/history/` holds older documents from the switch to Rust (audits, plans, risk assessment). They are Chinese only and are not updated any more.
- `docs/en/openapi.yaml` is shared by all languages; the Japanese and Chinese docs link to `../en/openapi.yaml`.
- The README is short: what Xexam does, the quick start, configuration, upgrading, the links to the docs and a link to the changelog (`changelog.md` of the same language). How to use the app, the Excel format and the import rules are in `usage.md`; deployment details, backups, security, architecture and development are in their own documents. Write a detail in one place and link to it.
- Every document starts with a language bar that links the same document in the other two folders. Links are relative to the file's own folder: the repository root is `../../`.
- Change the English document first, then the Japanese and Chinese ones in the same change. UI labels are written in bold exactly as the catalog of that language says them; the question language switch is always **English** / **中文** / **日本語**.
- The CI `check` job fails when one of the nine documents `api` … `usage` is missing in a language folder, when `docs/` contains anything other than the `en`, `ja` and `zh-CN` folders, or when a relative link or image path in a README or a doc points at a file that does not exist. Links to GitHub pages (`../../releases` and the like) are checked too: they must climb exactly to the repository page, so inside `README/` they start with `../../../`.

### 9.1 Screenshots

The README and docs images (`docs/<lang>/screenshots`, 19 desktop and 5 iPhone images per language, 24 in all) are made by `python tools/screenshots/retake.py` from the repository root. It builds the backend and ai-service images from `backend/Dockerfile` and `ai-service/Dockerfile` under unique tags (`xexam-shots-backend:<hex>`, `xexam-shots-ai-service:<hex>`) and runs them on a new Docker network with fresh volumes (`/data` for each, a shared link volume, read only for ai-service) and `TZ=UTC`, next to the made-up AI provider `tests/integration/ai_stub.py` (network name `ai-stub`). It builds the frontend with Vite and serves it with `vite preview` on a free port (the auth plugin checks every AI request with the backend, as Nginx does), then fills the instance with made-up demo banks, users and records, and sets up AI with the made-up provider and a made-up key, through the API; only after that does it give sam and alex the AI+ feature (the backend refuses that while AI+ is not available) and pause alex's AI+ feature, so the images show a bright, a dark and no badge, and an **Active** and a **Paused** row. After the restart it waits until `GET /api/users` says `aiAvailable: true`. It restarts the backend so the System log shows only a short stretch of sign-ins and user changes, then drives the real UI in headless Chrome for every language: the AI Chat scene turns the administrator's free mode on (so the green **Free mode** tag shows), starts a chat, sends a second question through the page (with the language of the run, so the answer is in it), and afterwards deletes the chat and turns free mode off again, and the study scenes of v0.9 open the AI hint window of a practice question, the AI analysis window of an exam result card and the AI summary window of a study record (`desktop-17-ai-hint`, `desktop-18-ai-result`, `desktop-19-ai-summary`, `iphone-5-ai-hint`), with the made-up provider's texts; the same run checks the layout (a failed check stops it): the practice action bar has at most two rows at 390 px, and Study records at 800 px (English and Japanese) is in the card layout without horizontal scroll. The AI settings scenes press **Test** and show the page in two images (the set-up and the users with AI+, with the AI+ feature column; then usage, retention and the safety limits), so the usage numbers differ slightly between the languages. Images are cut where no text or card is split and stripped of PNG metadata. Containers, volumes, the network, image tags, temporary folders and processes are removed at the end, also after an error.

Options: `--lang en|ja|zh-CN|all`, `--out DIR` (writes `DIR/<lang>/` so you can compare before touching `docs/`), `--only desktop-13-logs.png,...`, `--chrome PATH` (or `XEXAM_CHROME` / `CHROME_PATH`), and `--backend cargo` when Docker is not available (that run skips the System log image). It needs Python 3.8+ with `openpyxl`, Node.js, Docker and Chrome or Chromium; see `tools/screenshots/README.md`. With `--backend cargo` it builds `backend/` and `ai-service/` with cargo and runs the made-up provider as a local Python process. Two pages are shortened in the page itself so that their card ends inside the image: Question banks shows the format guide opened with only its first two parts, and About shows only the first points of the newest version. Two or three runs can go at once (a port another run took first is retried with a new one, and `npm ci` runs behind a lock file). Start it as `python tools/screenshots/retake.py`, not with `python -I`: `-I` hides the user site-packages where `openpyxl` is usually installed. Retake the images after a change of the UI or of the version, look at them, and only then write them into `docs/`.

### 9.2 Icons

The app icon (the header logo, the browser tab, the phone's home screen, the README) lives in `frontend/public/`: `icon.svg`, `icon-32.png`, `icon-192.png`, `icon-512.png` and `apple-touch-icon.png` (180 × 180, without transparent corners, because iOS rounds them itself). `python3 -I tools/icons/make_icons.py <source.svg>` writes all five from one source SVG: it draws the PNGs with headless Chrome (the DevTools client of `tools/screenshots/browser.py`) and re-encodes them in pure Python with only the IHDR, IDAT and IEND chunks, so they carry no metadata and the output depends only on the pixels. It prints the SHA-256 of every file and is idempotent.

## 10. Version, changelog and licence

- **`VERSION`** in the repository root holds two `KEY=VALUE` lines (lines starting with `#` are comments), each exactly once:
  - `CODE_VERSION` (`1.0` today): the version of the code on this branch. Bump it by hand, together with the changelog entry in all three changelogs (`docs/<lang>/changelog.md`), `backend/Cargo.toml`, `ai-service/Cargo.toml` and `frontend/package.json` (a version of two parts such as `0.9` is written `0.9.0` there; also their lock files) and `info.version` in `docs/en/openapi.yaml`. The images are built and tagged with it, and `/api/info` reports it (the backend reads it from `/VERSION` in the image).
  - `RELEASE_VERSION` (`0.9.1` until the release job of v1.0 has run): the last version that was actually published. **Never edit it by hand**: the release job writes it, together with the release badge at the top of the READMEs (section 8.1). A push or a merge alone never changes it.
- Every update adds an entry at the very top of the changelog in all three languages (`docs/en/changelog.md`, `docs/ja/changelog.md`, `docs/zh-CN/changelog.md`), with a version number equal to `CODE_VERSION`; the three keep the same entries. CI checks this. The READMEs hold no entries, only a link to the changelog. The GitHub Release notes are taken from the English one.
- The About page of the app shows these changelogs, built into the frontend (section 4). Keep the entries in simple Markdown: paragraphs, `- ` bullets, nested `  - ` bullets, `**bold**` and `` `code` ``; links to files in the repository are shown there as plain text, and they are written relative to the changelog's folder (`docs/<lang>/`). A changed changelog appears in the app only after the frontend is built again.
- Work happens on `dev`; `main` receives the merge, and a push to `main` publishes the images and the Release. Pull `main` and `dev` after each release (section 8.1).
- The project is under the MIT licence (`LICENSE` in the repository root).
