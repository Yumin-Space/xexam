**English** | [日本語](../ja/migration.md) | [简体中文](../zh-CN/migration.md)

# Migration and rollback

> This document covers every upgrade path, newest first:
>
> - **v2.7 → v2.8** (section 000000000000000): nothing to do besides `docker compose pull` and `docker compose up -d`. No database change (main schema 12, AI schema 11), no new variable, no `docker-compose.yml` or Nginx change. Rolling back to v2.7 needs no restore.
> - **v2.6 → v2.7** (section 00000000000000): nothing to do besides `docker compose pull` and `docker compose up -d`. No database change (main schema 12, AI schema 11), no new variable, no `docker-compose.yml` or Nginx change. Rolling back to v2.6 needs no restore.
> - **v2.5 → v2.6** (section 0000000000000): nothing to do besides `docker compose pull` and `docker compose up -d`. The main database moves to schema 12 on start after the copy `backups/xexam-schema11-<date-time>.db` (one new column, `users.display_name`); the AI database stays at schema 11 and only gains a table, columns and a model slot. Every existing user sets a username at the next sign-in, and users who never chose a question language now start in English. No new variable and no `docker-compose.yml` edit. Rolling back to v2.5 needs no restore.
> - **v2.4 → v2.5** (section 000000000000): nothing to do besides `docker compose pull` and `docker compose up -d`. No database change (main schema 11, AI schema 11), no new variable and no `docker-compose.yml` edit; Home becomes a dashboard and the old Home is the page Exam & practice. Rolling back to v2.4 needs no restore.
> - **v2.3 → v2.3.1** (section 00000000000): nothing to do besides `docker compose pull` and `docker compose up -d`. The AI service's container is renamed from `ai-service` to `xexam-ai-service` (the Compose service keeps its name); no variable, no `docker-compose.yml` edit and no database change. Rolling back to v2.3 needs no restore.
> - **v2.2 → v2.3** (section 0000000000): nothing to do. A change to the screens only (AI Chat layout and the top bar): no `docker-compose.yml` change, no new variable, and neither database changes. Rolling back to v2.2 needs no restore.
> - **v2.1 → v2.2** (section 000000000): no `docker-compose.yml` change is required and no new variable (the compose files only show the internal ports). The AI database stays at schema 11; a one-time step moves the pacing default to 13 / 230 000 after the copy `backups/xexam-ai-schema11-<date-time>.db`. Rolling back to v2.1 needs no restore.
> - **v2.0 → v2.1** (section 00000000): no `docker-compose.yml` change and no new variable. The AI database moves to schema 11 on start after the copy `backups/xexam-ai-schema10-<date-time>.db`; the main database does not change. Rolling back to v2.0 needs that copy restored (section 00000000.4).
> - **v1.9 → v2.0** (section 0000000): no `docker-compose.yml` change and no new variable; a database clean-up with nothing visible in the app. The main database moves to schema 11 and the AI database to schema 10 on start, each after its own backup copy (`backups/xexam-schema10-<date-time>.db`, `backups/xexam-ai-schema9-<date-time>.db`); keep about twice the size of the main database free. Rolling back to v1.9 needs the AI copy restored, the main database does not (section 0000000.4).
> - **v1.8 → v1.9** (section 000000; also from v1.6 or v1.7, which share the AI schema 8): no required `docker-compose.yml` change for one host; the AI database moves to schema 9 on start after the copy `backups/xexam-ai-schema8-<date-time>.db`; the main database does not change. New optional variables and three per-machine compose files (section 12 of [deployment.md](deployment.md)). Rolling back needs that copy restored (section 000000.4).
> - **v1.5 → v1.6** (section 00000): no new `docker-compose.yml` variable; the main database moves to schema 10 automatically after its own backup copy is written (additive: one column, `users.can_download_banks`). The AI database stays at schema 8 (one column, `model_slots.effort`, is added by itself, with no backup copy). Rolling back to v1.5 needs no restore.
> - **v1.3 → v1.4** (section 0000): the main database moves to schema 9 and the AI database to schema 8, each automatically after its own backup copy is written (additive: last-activity columns, and the table of the external location lookup). **The Docker volumes are renamed** (`xexam_data` → `xexam_backend_data`, `xexam_link` → `xexam_ai_auth`): copy `xexam_data` into `xexam_backend_data` before the first v1.4 start, or Xexam starts with an empty database. Rolling back to v1.3 uses `xexam_data` again; the AI database needs its copy back (section 0000.4).
> - **v1.2 → v1.3** (section 000): a new `docker-compose.yml` is recommended (a `tunnel` network and the optional `cloudflared` service, for real client addresses); the main database moves to schema 8 and the AI database to schema 7, each automatically after its own backup copy is written. The main database rolls back without a restore; the AI database needs its copy back (section 000.4).
> - **v0.9.1 → v1.0** (section 0): no new `docker-compose.yml`; the AI database moves to schema 4 after its backup copy `backups/xexam-ai-schema3-<date-time>.db` is written; the main database does not change. Rolling back to v0.9.1 needs that copy restored (section 0.4).
> - **v0.9 → v0.9.1** (no section of its own): nothing to do besides `docker compose pull` and `docker compose up -d`; no database change, no new setting and no new `docker-compose.yml`. Rolling back to v0.9 is changing `:latest` to `:v0.9` for the three images and running `docker compose up -d`; the databases stay as they are.
> - **v0.8 → v0.9** (section 0b): no new `docker-compose.yml`; the main database moves to schema v6 and the AI database to schema 3, each automatically after its own backup copy is written. The main database rolls back without a restore; the AI database needs its copy back. (This section is numbered 0b so that the numbers of the others, which other documents link to, stay as they are.)
> - **v0.7.5 or v0.7 → v0.8** (section 1): a new `docker-compose.yml` with the AI service, `ai-service` (a third image, `xexam-ai-service`, and two volumes); the database moves to schema v5 automatically, after a backup copy is written; AI stays off until an administrator sets it up.
> - **v0.6 → v0.8** (section 2): the database moves to schema v5 and to WAL mode, automatically, after a backup copy is written; then the questions of the old three- and four-column Excel layouts are retired, and you update the banks with six-column workbooks.
> - **v0.5 or v0.5.1 → v0.8** (section 3): sign-in arrives together with the later schemas, in one step.
> - **v0.4 → v0.5 and later** (section 4): question banks, exam detail and settings (schema v2), today together with everything after it.
> - **Python v0.2.x → Rust + TypeScript + Nginx (v0.3)** (sections 5–11): the database file and its schema did not change then; the switch meant "swapping the program", not "moving data". Basis for that decision: [migration-plan.md](../zh-CN/history/migration-plan.md). Risks: [risk-assessment.md](../zh-CN/history/risk-assessment.md).
> - **v0.1 → later** (section 12): the database file name.
>
> Every upgrade that changes the schema writes **one** copy of the database first, named after the schema it had: `backups/xexam-schema<N>-<YYYYmmdd-HHMMSS>.db` next to `xexam.db` (in Docker: `/data/backups/` in the volume `xexam_backend_data`; `xexam_data` before v1.4). Rolling back means **restoring that copy** and running the older images, with one exception: v0.8 opens the schema-6 main database of v0.9 as it is (section 0b.4). The AI database has its own copies, `xexam-ai-schema<N>-<YYYYMMDD-HHMMSS>.db` in `/data/backups/` of the volume `xexam_ai_data`; the AI service of v0.8 cannot open the schema-3 database of v0.9, so rolling back to v0.8 means restoring the schema-2 copy there. The background is in section 6 of [database.md](database.md). A rollback of the main database never touches the AI database, and the other way round.
>
> **Going straight to v1.0 from an older version** applies every step of the sections below in one go with the current images: one backup named after the schema the database had, then the migration to the current schema (6 for the main database, 4 for the AI database). The sections below are written for the version they lead to, so where one says schema 5, v1.0 ends at schema 6, the log says `Upgraded the database to schema 6`, `/api/info` shows `1.0` and ai-service creates its database at schema 4.
>
> **Every upgrade from v0.6 or older retires all your questions.** Since v0.7 only the six-column Excel layout can be imported, and every start retires the questions that were imported from the three- and four-column layouts of earlier versions; every question of v0.6 and older is in one of those. The banks, the study records and the statistics stay. Prepare a six-column version of each workbook before you upgrade (section 12 of [usage.md](usage.md)), and update each bank with it afterwards (section 2.4).

## 000000000000000. Upgrading from v2.7 to v2.8

This section is numbered 000000000000000 because it is the newest one. The older sections keep their numbers.

### 000000000000000.1 What changes

- **Nothing in the data or the servers.** Neither database changes (the main database stays at schema 12, the AI database at schema 11), there is no new variable, and neither `docker-compose.yml` nor the Nginx configuration needs an edit.
- **Dark theme**: graded options are tinted cards (correct: olive with a green line; wrong: red-brown with an orange-red line) instead of solid blocks; the light theme is unchanged.
- **User management**: a user's bank access opens from a database icon right after the study-records book, instead of the line under the user's details.
- **Exam with a deleted bank**: sending the same result again returns the same list of questions that were not counted, so the note is not lost.
- **Sign-in lock**: reviewed, no change in behaviour.
- **Tests and docs**: two flaky tests are deterministic, and the API reference, the database diagrams and the architecture page are brought up to date.

### 000000000000000.2 Upgrade

`docker compose pull`, then `docker compose up -d`. The data volumes stay as they are.

### 000000000000000.3 Check

The app menu shows **Xexam v2.8**. In User management every row has a database icon right of the study-records book (greyed for administrators).

### 000000000000000.4 Going back to v2.7

Change `:latest` to `:v2.7` for the three images and run `docker compose up -d`. No restore is needed: nothing in the data changed.

## 00000000000000. Upgrading from v2.6 to v2.7

This section is numbered 00000000000000 because it is the newest one. The older sections keep their numbers.

### 00000000000000.1 What changes

- **Nothing in the data or the servers.** Neither database changes (the main database stays at schema 12, the AI database at schema 11), there is no new variable, and neither `docker-compose.yml` nor the Nginx configuration needs an edit.
- **Two new read-only routes for administrators**: `GET /api/users/records?user=` and `GET /api/users/record?user=&id=` (Study records of any user, from User management). Each look is written to the activity log as `users.viewRecords`. Scripts are not affected.
- **The AI study assessment is checked and retried**: ai-service asks for the whole answer, cleans it and checks it, and asks again up to 3 calls in all; the retries do not count against the per-user AI caps but show in AI usage. A new error code `ai.assess.unusable` ends a request whose 3 calls gave no usable text.
- **Older garbled capsules are hidden**: a stored assessment text that fails today's check is not shown on Home and does not use up the day, so the user can write a new one.
- **Thinking room**: a call that sends a thinking strength asks the provider for **Tokens per answer** plus room for thinking (at most 32 768); nothing to set.
- **Screens**: the answer sheet is a left drawer on phones, Study records and User management have numbered pages, and the dark theme is all black.

### 00000000000000.2 Upgrade

`docker compose pull`, then `docker compose up -d`. The data volumes stay as they are.

### 00000000000000.3 Check

The app menu shows **Xexam v2.7**. In User management every row has a book icon left of the pencil; on a phone an exam or a practice has the answer sheet button at the top left.

### 00000000000000.4 Going back to v2.6

Change `:latest` to `:v2.6` for the three images and run `docker compose up -d`. No restore is needed: nothing in the data changed.

## 0000000000000. Upgrading from v2.5 to v2.6

This section is numbered 0000000000000 because it is the newest one. The older sections keep their numbers.

### 0000000000000.1 What changes

- **The main database** moves from schema 11 to **schema 12**, once, on the first start of the v2.6 backend, after the copy `backups/xexam-schema11-<date-time>.db`. The step adds one nullable column, `users.display_name` (the username others see), and the unique index `idx_users_display_name`; no row is changed and no login is rewritten. Keep free disk space of about the size of the database for the copy. Details: section 6.19 of [database.md](database.md).
- **The AI database** stays at **schema 11**. On every start, without a copy, ai-service adds the table `assess_latest` (the latest study assessment of each user), the column `usage.assess`, the limits `assess_per_day` (3) and `assess_admins_exempt` (on) and the model slot `assess`, which starts with the model and thinking strength of the summary slot (section 8.2k of [database.md](database.md)). The AI study assessment is now its own feature, on for everyone after the upgrade; at most 3 successful assessments per user and day, administrators not limited.
- **Every existing user is asked for a username** at the next sign-in (one field; together with the new password when one is due). Until it is set, the account can do nothing else. Tell your users before you upgrade. The administrator who installed Xexam is asked too.
- **Sign-in is by e-mail.** The sign-in page accepts only an e-mail address, plus the default administrator `admin`. If your accounts use plain names as logins (not e-mail addresses), give each one an e-mail login in **User management** → **Edit** before your users sign in; until then such a user can only be reached through the API (the server still accepts every login) or through `admin`. An unchanged older login can stay in **Edit**; a new or changed login must look like an e-mail address. A login can no longer equal another account's username.
- **The default question language is English.** Users who never picked a question language (the stored setting is missing) now start in English instead of Japanese; a stored choice is kept. Each user can set it in **Settings** → **Default question language**.
- **Hand-written mode is on by default** in every browser, so each browser loads the font (about 1.1 MB) once on its first visit. Users switch it off with the **Aa** button or in **Settings** → **Text**.
- **Nginx**: the frontend image's configuration has one more body in `map $ai_deny $ai_forbidden_body` (`name`, `请先设置用户名。`, code `auth.name`). If you replaced the Nginx configuration of the image with your own, add that line; otherwise a user without a username gets the generic body on AI routes. The route `GET /api/ai/assess/saved` is gone (`GET /api/ai/assess/latest` replaces it); no Nginx change is needed for it.
- **Scripts that call the API**: `GET /api/dashboard` answers `wrongTop` instead of `popular`; the login error texts say 邮箱 (`邮箱或密码不对。`, `这个邮箱已经有人用了。`); a session without a username gets 403 `请先设置用户名。` until `POST /api/auth/name` is called. See "Changes in v2.6" in section 7 of [api.md](api.md).

### 0000000000000.2 Upgrade

`docker compose pull`, then `docker compose up -d`. The data volumes stay as they are. The backend log says `Upgraded the database to schema 12. A copy of the previous version was saved first: /data/backups/xexam-schema11-….db …`.

### 0000000000000.3 Check

The app menu shows **Xexam v2.6**. Sign in: you are asked for a username, then Home shows the leaderboard and **Most missed**, and the user row at the bottom of the menu opens **Profile**.

### 0000000000000.4 Going back to v2.5

Change `:latest` to `:v2.5` for the three images and run `docker compose up -d`. **No restore is needed**: v2.5 opens the schema-12 main database as it is (it treats a newer schema as current and never reads `users.display_name`), and the AI database is still schema 11 (v2.5 ignores the new table, columns and slot). Usernames stay stored for a later v2.6; accounts made under v2.5 meanwhile are asked for one then. Before going back, switch the **AI study assessment** feature on again in AI settings if you had switched it off (section 8.2k of [database.md](database.md)). To get back the exact state from before the upgrade instead, stop the backend and restore `backups/xexam-schema11-<date-time>.db` as in section 5 of [database.md](database.md); everything written since is lost.

## 000000000000. Upgrading from v2.4 to v2.5

This section is numbered 000000000000 because it is the newest one. The older sections keep their numbers.

### 000000000000.1 What changes

- **Nothing in the data or the servers.** Neither database changes (the main database stays at schema 11, the AI database at schema 11), there is no new variable, and `docker-compose.yml` needs no edit. The new routes are read-only or reuse the existing AI tables: `GET /api/dashboard`, `POST /internal/ai/assess`, `POST /api/ai/assess/ask` and `GET /api/ai/assess/saved`. An AI study assessment is stored as a `summary` chat with the reference `assess@<date>`, which v2.4 simply shows as a summary chat.
- **Behaviour you will notice** (section 2.1 and section 3 of [usage.md](usage.md)): Home is a dashboard with a greeting, page tiles, the leaderboard, the most practised questions and, for AI+ users, the AI study assessment; the old Home (banks, sheets, **Start exam**, **Start practice**) is the page **Exam & practice**, second in the menu, at `#/study`. A bookmark of `#/` now opens the dashboard.
- **Nginx**: the frontend image's configuration also streams `/api/ai/assess/ask` without buffering. If you replaced the Nginx configuration of the image with your own, add `assess/ask` to the streamed AI routes (the regular expression that lists `send`, `retry`, `regenerate`, `edit`, `study/ask` and `summary/ask`).

### 000000000000.2 Upgrade

`docker compose pull`, then `docker compose up -d`. The data volumes stay as they are.

### 000000000000.3 Check

The app menu shows **Xexam v2.5** and lists **Home** and **Exam & practice** as its first two entries; Home shows the leaderboard.

### 000000000000.4 Going back to v2.4

Change `:latest` to `:v2.4` for the three images and run `docker compose up -d`. No restore is needed: nothing in the data changed, and the saved assessment chats show as ordinary summary chats.

## 00000000000. Upgrading from v2.3 to v2.3.1

This section is numbered 00000000000 because it is the newest one. The older sections keep their numbers.

### 00000000000.1 What changes

- **Only a name.** The container of the AI service is now called `xexam-ai-service` (it was `ai-service`), like `xexam-backend` and `xexam-frontend`. The Compose service is still `ai-service`, so `docker compose logs ai-service`, `docker compose exec ai-service …`, `http://ai-service:8766` and `XEXAM_AI_UPSTREAM` do not change. Neither database changes, there is no new variable, and `docker-compose.yml` needs no edit besides the `container_name` in the new file.
- **Scripts**: anything that calls `docker logs ai-service`, `docker restart ai-service` or `docker exec ai-service …` by container name must use `xexam-ai-service` now.

### 00000000000.2 Upgrade

`docker compose pull`, then `docker compose up -d`. Compose replaces the old `ai-service` container with the one under the new name; the data volumes stay as they are.

### 00000000000.3 Check

The app menu shows **Xexam v2.3.1**, and `docker compose ps` lists `xexam-frontend`, `xexam-backend` and `xexam-ai-service`, all `healthy`.

### 00000000000.4 Going back to v2.3

Change `:latest` to `:v2.3` for the three images (and the container name in your compose file back to `ai-service` if you changed it) and run `docker compose up -d`. No restore is needed: nothing in the data changed.

## 0000000000. Upgrading from v2.2 to v2.3

This section is numbered 0000000000 because it is the newest one. The older sections keep their numbers.

### 0000000000.1 What changes

- **Nothing in the data or the servers.** Neither database changes (the main database stays at schema 11, the AI database at schema 11), there is no new variable, and `docker-compose.yml` does not change.
- **Behaviour you will notice** (section 16 of [usage.md](usage.md)): AI Chat opens a new chat every time and has a sidebar (a drawer on phones) instead of the Chats and Memory tabs; the top bar shows the logo icon again on tablets and desktops.

### 0000000000.2 Upgrade

`docker compose pull`, then `docker compose up -d`. Nothing else has to be done.

### 0000000000.3 Check

The app menu shows **Xexam v2.3**. Open AI Chat: it starts with an empty new chat.

### 0000000000.4 Going back to v2.2

Change `:latest` to `:v2.2` for the three images and run `docker compose up -d`. No restore is needed: nothing in the data changed.

## 000000000. Upgrading from v2.1 to v2.2

This section is numbered 000000000 because it is the newest one. The older sections keep their numbers.

### 000000000.1 What changes

- **The AI database** stays at schema 11. On the first start of the new ai-service, a pacing setting still on the v2.1 default (13 requests / 240 000 tokens a minute) moves once to 13 / 230 000, after the copy `backups/xexam-ai-schema11-<date-time>.db`; the `meta` flag `pace_default_230k` marks it. Values you entered yourself are kept. Details: section 8.2i of [database.md](database.md).
- **The main database** does not change.
- **Behaviour you will notice** (section 17.9 of [usage.md](usage.md)): every thinking-strength dropdown starts with **Default** (no strength parameter is sent), and rows you never touched are Default too, so they no longer send the model's default level; AI Chat has a separate strength button; the phone top bar shows the person icon and a tip instead of the account name; the quiz screen has a theme button.

### 000000000.2 Upgrade

`docker compose pull`, then `docker compose up -d`. If you copied the compose files before, the new ones show `XEXAM_BACKEND_PORT` and `XEXAM_AI_PORT` in `.env`; nothing has to change when you keep the defaults.

### 000000000.3 Check

The log of ai-service says it moved the pacing default (only when the values were still the old default). **AI safety → Server** shows 13 requests and 230 000 tokens a minute unless you set your own.

### 000000000.4 Going back to v2.1

Change `:latest` to `:v2.1` for the three images and run `docker compose up -d`. No restore is needed: the schema is the same, and the pacing values stay as they are.

## 00000000. Upgrading from v2.0 to v2.1

This section is numbered 00000000 because it is the newest one. The older sections keep their numbers.

### 00000000.1 What changes

- **The AI database** moves from schema 10 to **schema 11**, once, on the first start of the new ai-service, after the copy `backups/xexam-ai-schema10-<date-time>.db`. It adds `connections.effort_family` (inferred for each existing connection; check it in the connection form) and `chats.pick_effort`, and clears stored slot strengths that do not fit their connection's family. Details: section 8.2h of [database.md](database.md).
- **The main database** does not change.
- **Behaviour you will notice** (section 17.9 of [usage.md](usage.md)): the thinking strength is set per connection and every call sends a level (the model's default unless you chose one); the **Default** button is gone; model rows are greyed while AI+ is off; AI Chat has a **Strength** step; the top bar has a light / dark button.

### 00000000.2 Upgrade

`docker compose pull`, then `docker compose up -d`. No compose change. Afterwards open **AI settings → Connections and models** and check the **Thinking-strength parameter** of each connection.

### 00000000.3 Check

The log of ai-service says it upgraded the database to schema 11 and prints one line per connection (id and family). **AI settings** shows the family on every connection card.

### 00000000.4 Going back to v2.0

Stop the stack, restore `backups/xexam-ai-schema10-<date-time>.db` into the AI volume (section 8.4 of [database.md](database.md)), change `:latest` to `:v2.0` for the three images and run `docker compose up -d`. Settings, chats and usage written after the upgrade are lost. The main database needs no restore.

## 0000000. Upgrading from v1.9 to v2.0

This section is numbered 0000000 because it is the newest one. The older sections keep their numbers. v2.0 is a database clean-up: nothing changes in how Xexam looks or behaves.

### 0000000.1 What changes

- **The main database** moves from schema 10 to **schema 11**, once, on the first start of the new backend, after the copy `backups/xexam-schema10-<date-time>.db`. Nine tables are rebuilt with foreign keys, rows that point to nothing are removed or detached by fixed rules (counted and logged; the copy holds them), the unused legacy tables `stats` and `settings` are dropped, and three indexes are added (one is a unique index on `users.uid`, skipped with a warning if two users share a stamp). The whole step is one transaction that rolls back on any surprise. Details: section 6.17 of [database.md](database.md).
- **The AI database** moves from schema 9 to **schema 10**, once, on the first start of the new ai-service, after the copy `backups/xexam-ai-schema9-<date-time>.db`. The old table `secrets`, sixteen unused columns of `settings`, `limits` and `chats` are dropped (counts are logged, never values). Details: section 8.2g of [database.md](database.md).
- **Nothing else**: no new setting, no API change, no compose change; the screens and answers are the same.
- **Disk and time**: keep about twice the size of the main database free on the volume of `xexam_backend_data` (the backup plus the rebuilt tables) and about the size of the AI database on the volume of `xexam_ai_data`. With a large history of study records the first start takes longer; the server listens only after both upgrades are done.

### 0000000.2 Upgrade

1. **Back up both data volumes** (stop first, as in the upgrade section of [README](../../README.md#upgrading)) and check the free disk space.
2. `docker compose pull`, then `docker compose up -d`.

### 0000000.3 Checks after the upgrade

- `/api/info` and the **About** page show `2.0`.
- The backend log has `Upgraded the database to schema 11` with the copy it wrote, and only when something was cleaned: `Removed <n> orphaned rows of <relation> (copy in the backup).`, `Detached <n> questions from a missing bank; they are kept in the legacy bank.` and `Removed the unused legacy table <stats|settings> (<n> rows, …)`. A line that starts with `Warning: <n> user identity stamps are shared by more than one user` means duplicates: see 6.17.
- The ai-service log has `Upgraded the AI database to schema 10` and `Removed the unused legacy table secrets (<n> rows) and <k> unused columns (copy in the backup).`
- If an upgrade fails, the log says `cannot open database: the upgrade to schema 11 stopped (<reason>). Nothing was changed; the copy <path> was written.` (or `cannot upgrade the AI database` for the AI side). The file is unchanged, the copy stays, and the next start tries again.

### 0000000.4 Rollback to v1.9

- **Main database**: nothing to restore. A v1.9 backend opens the schema-11 file as it is; an empty `stats` table that it re-creates is dropped again by v2.0 later (section 6.18 of [database.md](database.md)).
- **AI database**: a v1.9 ai-service refuses schema 10 (`written by a newer ai-service`). Stop the stack, restore `backups/xexam-ai-schema9-<date-time>.db` into the AI volume as in section 8.4 of [database.md](database.md) (remove `xexam-ai.db-wal` and `xexam-ai.db-shm`), change the three images to `:v1.9` and run `docker compose up -d`. Settings, chats and usage written after the upgrade are not in the copy and are lost.
- **Further back than v1.9**: restore the main copy `xexam-schema10-<date-time>.db` as well and follow the rollback of the version you return to (the sections below).
- The backups made before the upgrade still hold the old provider keys (the table `secrets` of the AI copy): keep them private, or delete them when you no longer need them (section 11 of [security.md](security.md)).

## 000000. Upgrading from v1.8 to v1.9

This section is numbered 000000 because it is the newest one. The older sections keep their numbers. The steps are the same from v1.6 and v1.7: they all have AI schema 8.

### 000000.1 What changes

- **The AI database** moves from schema 8 to **schema 9**, once, on the first start of the new ai-service, after the copy `backups/xexam-ai-schema8-<date-time>.db`. The step adds `connections.enabled` (every connection stays **on**) and the three pacing columns of `limits` (`pace_on` 1, `pace_rpm` 15, `pace_tpm` 250 000), rebuilds `model_slots` (the `study` row is copied to both `hint` and `explain`: both start with the old study model and thinking strength) and rewrites stored Gemma thinking strengths to the Google family with the same request. Details: section 8.2f of [database.md](database.md).
- **The main database** does not change.
- **Behaviour you will notice** (section 17.10 of [usage.md](usage.md)): feature switches are now one per model row and apply to administrators too; the pacing of AI generation and import is on by default (13 requests and 230 000 tokens a minute, used exactly as entered; since v2.1 there is no hidden 10 % margin, and an AI database upgraded to schema 11 moves values still on the old default 15 / 250 000 to 13 / 230 000 and keeps the others as entered); **AI safety** has seven sub-tabs; the **Gemma** thinking button is gone; requests to AI providers carry `xexam-ai-service/<version>` again as `User-Agent` unless `XEXAM_AI_USER_AGENT_NAME=none` (the v1.8 behaviour).
- **Compose**: nothing is required for one host. The new variables (`XEXAM_FRONTEND_PEERS`, `XEXAM_INTERNAL_TOKEN_REQUIRED`, `XEXAM_BACKEND_UPSTREAM`, `XEXAM_AI_UPSTREAM`, `XEXAM_NGINX_RESOLVER`) and `docker-compose.frontend.yml`, `docker-compose.backend.yml`, `docker-compose.ai-service.yml` matter only when you run the services on different machines.

### 000000.2 Upgrade

1. **Back up both data volumes** (stop first, as in the upgrade section of [README](../../README.md#upgrading)).
2. `docker compose pull`, then `docker compose up -d`.

### 000000.3 Checks after the upgrade

- `/api/info` and the **About** page show `1.9`; the ai-service log has `Upgraded the AI database to schema 9` with the copy it wrote.
- **AI settings** shows a switch on every connection (on) and a switch on every model row except **Default**; **Hint** and **Analysis** are separate rows with the same model as the old **Study** row.

### 000000.4 Rollback to v1.8

- **Main database**: nothing to restore.
- **AI database**: a v1.8 ai-service refuses schema 9 (`written by a newer ai-service`). Stop the stack, restore `backups/xexam-ai-schema8-<date-time>.db` into the AI volume as in section 8.4 of [database.md](database.md) (remove `xexam-ai.db-wal` and `xexam-ai.db-shm`), change the three images to `:v1.8` and run `docker compose up -d`. Settings, chats and usage written after the upgrade are not in the copy and are lost.

## 00000. Upgrading from v1.5 to v1.6

This section is numbered 00000 because it is the newest one. The older sections keep their numbers. (v1.5 itself changed only the CI and has no section: upgrading to it needed nothing.)

### 00000.1 What changes

- **The main database** moves from schema 9 to **schema 10**, once, on the first start of the new backend, after the copy `backups/xexam-schema9-<date-time>.db`. The step is additive: `users` gets `can_download_banks` (0 for every user, so **nobody but administrators can download banks until an administrator ticks Download banks for them**). The log row is `system.migrate` with `from` 9 and `to` 10.
- **The AI database** stays at **schema 8**. On start ai-service adds the nullable column `model_slots.effort` (the thinking strength of each slot; NULL = **Default**, nothing is sent), so every slot behaves as before until an administrator chooses a strength. No backup copy is written because the schema number does not change.
- **Bank download** (Excel or Word) appears on **Question banks** for administrators at once; plain users get it with the new per-user right (section 11.1 of [usage.md](usage.md)).
- **Thinking strength** rows appear in **AI settings**; nothing is sent until one is chosen (section 17.9 of [usage.md](usage.md)). AI generation and import now ask for structured output where the provider supports it and use a larger plan budget; nothing needs to be set.
- **Nginx** now passes `X-Geo-*` to ai-service, from trusted proxies only, so the **System log** rows written by ai-service carry the Cloudflare place too (nothing to configure). If you use the external lookup API, the first switch to it (or the next save of its settings) after the upgrade looks up places for old records without one (section 14.2 of [usage.md](usage.md)).

### 00000.2 Upgrade

1. **Back up both data volumes** (stop first, as in the upgrade section of [README](../../README.md#upgrading)).
2. `docker compose pull`, then `docker compose up -d`. No change to `docker-compose.yml` or `.env` is needed.

### 00000.3 Checks after the upgrade

- `/api/info` and the **About** page show `1.6`; the backend log has `Upgraded the database to schema 10` with the copy it wrote. ai-service logs no schema upgrade (its schema is still 8).
- As an administrator, **Question banks** shows a **Download** button on every bank; the **Question banks of ...** dialog in **User management** has **Download banks** (unticked) next to **Manage banks**.
- **AI settings** shows **Thinking strength** with **Default** pressed in every model row.

### 00000.4 Rollback to v1.5

- **Main database**: nothing to restore. A v1.5 backend opens the schema-10 file as it is and ignores `can_download_banks` (its own inserts get the default 0); the copy `xexam-schema9-<date-time>.db` is optional. Download rights set under v1.6 are not used by v1.5 and are still there when you upgrade again.
- **AI database**: nothing to restore. A v1.5 ai-service opens the file (schema 8) and never reads `model_slots.effort`; the strengths you chose are used again by v1.6.
- Change the three images to `:v1.5` and run `docker compose up -d`. v1.6 does not delete the pre-migration copy; remove it by hand when you no longer need it.

### 00000.5 What has been tested

- The schema 9 to 10 step on files with data and on empty files (one backup, one `system.migrate` row, the same tables as a new file), the permission (off by default, kept across role changes, always on for administrators, 403 without it, read on every request), the Excel round trip (download, import again, same questions), the Word file (structure, language order, marked answers), the thinking-strength parameter of every family and its retry without the parameter after a 400 / 422, the structured-output fallback, the `answer` field, and the sign-in place backfill (one lookup per address, pause after a rate limit, no overwriting), with local stubs only.
- Not covered by the release checks: a Word document opened in every Word version (the file uses built-in styles only), a provider on the real internet beyond a few manual calls, and the phone layouts on a real device.

## 0000. Upgrading from v1.3 to v1.4

This section is numbered 0000 because it is the newest one. The older sections keep their numbers.

**Upgrade step first: the volumes are renamed** (`xexam_data` → `xexam_backend_data`, `xexam_link` → `xexam_ai_auth`). After the update Xexam starts with an empty database until the old data is copied over:

1. `docker compose down`
2. `docker run --rm -v xexam_data:/from:ro -v xexam_backend_data:/to alpine sh -c 'find /to -mindepth 1 -delete && cp -a /from/. /to/'`
3. `docker compose up -d`

In Portainer, copy the contents of `xexam_data` into `xexam_backend_data` the same way. The old volumes `xexam_data` and `xexam_link` stay untouched until you remove them by hand (`docker volume rm`); `xexam_link` is not needed, a new token is created.

### 0000.1 What changes

- **The main database** moves from schema 8 to **schema 9**, once, on the first start of the new backend, after the copy `backups/xexam-schema8-<date-time>.db`. The step is additive: users get `last_active_ip`, `last_active_country`, `last_active_region`, `last_active_city` and `last_active_ua`; nothing is backfilled, so **Last active IP / Active location / Last active device** fill in as users are active again. The log row is `system.migrate` with `from` 8 and `to` 9.
- **The AI database** moves from schema 7 to **schema 8**, once, on the first start of the new AI service, after the copy `backups/xexam-ai-schema7-<date-time>.db` in the AI volume. It adds the empty table `geo_settings` (the external location lookup).
- **The location source** is **Cloudflare request headers** after the upgrade: nothing is looked up and nothing is sent anywhere until an administrator sets up and switches on the external lookup API (section 14.2 of [usage.md](usage.md)).
- **User management** shows one more set of lines and the **Location source** setting; the **System log** rows show **IP · place · device**. The rest of v1.4 (the AI Chat model name rule and the trusted private-network proxy default) is described in the [changelog](changelog.md).

### 0000.2 Upgrade

1. **Back up both data volumes** (stop first, as in the upgrade section of [README](../../README.md#upgrading)).
2. Take the new `docker-compose.yml` (it carries the `XEXAM_TRUSTED_PROXIES` default) and keep your `.env`, then `docker compose pull`, `docker compose down`, the copy command `docker run --rm -v xexam_data:/from:ro -v xexam_backend_data:/to alpine sh -c 'find /to -mindepth 1 -delete && cp -a /from/. /to/'` (the steps at the top of this section; **before** the first v1.4 start), and `docker compose up -d`. Do not use `down -v`.
3. Optional: to show places without Cloudflare, see section 3.5 of [deployment.md](deployment.md).

### 0000.3 Checks after the upgrade

- `/api/info` and the **About** page show `1.4`; the log has `Upgraded the database to schema 9` (backend) and `Upgraded the AI database to schema 8` (ai-service), each with the copy it wrote.
- Open the sign-in info eye in **User management**: users who are active again show **Last active IP**, **Active location** and **Last active device**; the **Location source** setting is on **Cloudflare request headers**.

### 0000.4 Rollback to v1.3

- **Main database**: v1.3 uses the volume `xexam_data` again, which still holds the state from before the upgrade (nothing was written there under v1.4); what was written under v1.4 is in `xexam_backend_data` (copy it back into `xexam_data` if you want to keep it: a v1.3 backend opens the schema-9 file as it is, the new columns are ignored). Restoring `xexam-schema8-<date-time>.db` is optional.
- **AI database**: stop the stack, put `xexam-ai-schema7-<date-time>.db` back as the AI database (`/data/xexam-ai.db` in the volume `xexam_ai_data`), change the three images to `:v1.3` and start. What was written under v1.4 in the AI database (chats, usage, the external lookup settings and key) is lost. v1.3 refuses a schema-8 file, so nothing is damaged silently.
- v1.4 does not delete the pre-migration copies; remove old ones by hand when you no longer need them.

### 0000.5 What has been tested

- Backend 583 tests, frontend 819, AI service 1113, all green, including: schema 8 to 9 and 7 to 9 on files with data (the old and the new file have the same tables, one backup and one `system.migrate` row), AI schema 1 to 7 to 8 (same tables on every path), the source rule (Cloudflare mode ignores the lookup; external mode ignores Cloudflare's place headers), no lookup while the eye is closed, **Clear records** wiping the last-active columns and the memory, the reserved-address filter, and the key never appearing in an answer, a log line or the database file, with a local stub provider only (success, timeout, HTTP 500, not JSON, missing fields, redirect refused, oversized).
- Not covered by the release checks: a provider on the real internet beyond a few manual **Test** calls with the public example address `8.8.8.8`, and the phone layouts on a real device.

## 000. Upgrading from v1.2 to v1.3

This section is numbered 000 because it is the newest one. The older sections keep their numbers.

### 000.1 What changes

- **The main database** moves from schema 7 to **schema 8**, once, on the first start of the new backend, after the copy `backups/xexam-schema7-<date-time>.db`. The step is additive: sessions get a start time, a remember flag, an address and a device; users get `last_active_at` and the four sign-in info columns; the tables `activity_log` and `system_settings` are new. Every session is given the upgrade time as its start, so an administrator is signed out at the latest 7 days after the upgrade.
- **The AI database** moves from schema 6 to **schema 7**, once, on the first start of the new AI service, after the copy `backups/xexam-ai-schema6-<date-time>.db` in the AI volume. It also adds three nullable columns to the messages for the thinking of AI Chat answers (`reasoning`, `reasoning_ms`, `reasoning_truncated`; older answers have none and show no thinking block). Your provider settings and keys become **connection 1** (named after the host of your address; the keys are copied, not moved), the model of the settings becomes the **default** model, and AI+ keeps its on / off state, so every feature keeps using the same model as before. **AI hint** and **AI analysis** become one permission (**AI hint + analysis**): a user who had only one of the two keeps both. Chats become trees (each message points to the one before it, the last message is the shown branch). The old settings columns and the `secrets` table stay in the file unused.
- **Limits**: **import questions per file** stays as it was unless it was the old ceiling of 50, which becomes 100; **memory length** (1000 tokens) and **memories per user** (50) move to 200 and 30 only when they still have the old default values. Values you changed are never lowered. New limits start at their defaults (administrators exempt from request caps: on; sharing: on, 180 days; at most 8 generation parts; summary items 40; daily token budget: off).
- **Sign-in**: sessions follow the new rules (section 5.1 of [security.md](security.md)). An administrator who was idle for more than 24 hours has to sign in again; users keep 30 days of use. Nobody is signed out by the upgrade itself.
- **Real client address and location**: the new `docker-compose.yml` makes Nginx trust the bundled `cloudflared` container (default `XEXAM_TRUSTED_PROXIES=172.31.254.2`). With the old compose file nothing is trusted: everything works, but the IP column shows the address Nginx sees. If `cloudflared` runs on the host or on another machine, see section 3.5 of [deployment.md](deployment.md). The **sign-in IP and location eye** is closed (hidden) after the upgrade.
- **Removed**: the AI settings routes `POST /api/ai/admin/verify` and `POST /api/ai/admin/models` (replaced by the connection and model routes) and the old settings screen. Open browser tabs of v1.2 show an error until they are reloaded.

### 000.2 Upgrade

1. **Back up both data volumes** (stop first, as in the upgrade section of [README](../../README.md#upgrading)).
2. **Get the new `docker-compose.yml`** (recommended; it adds the `tunnel` network, the pinned `public` subnet, the Nginx variables and the `cloudflared` profile), keep your `.env`, then `docker compose pull` and `docker compose up -d`. Do not use `down -v`.
3. To use the bundled tunnel: put `TUNNEL_TOKEN` in `.env`, set the public hostname of the tunnel to `http://frontend:8080` in the Cloudflare dashboard, and start with `docker compose --profile tunnel up -d`. Stop any `cloudflared` that ran elsewhere.
4. Optional, for regions and cities: in Cloudflare turn on **Add visitor location headers** (Rules > Managed Transforms); the country needs nothing.

### 000.3 Checks after the upgrade

- `/api/info` and the **About** page show `1.3`; the log has `Upgraded the database to schema 8` (backend) and `Upgraded the AI database to schema 7` (ai-service), each with the copy it wrote.
- **AI settings** shows one connection with its models, the default model and the same features as before; AI Chat shows the old chats with their messages.
- Sign in as an administrator: **Users** shows **Last active**; the **Activity** tab of the System log has a `system.migrate` entry.
- Behind the tunnel, open the sign-in info eye in **Users** and sign in again: the entry shows your public address and, with the location headers, your country; without the tunnel it shows the connecting address.

### 000.4 Rollback to v1.2

- **Main database**: nothing to restore. A v1.2 backend opens the schema-8 file as it is (the new columns have defaults, the new tables are ignored). Restoring `xexam-schema7-<date-time>.db` is optional and loses what was written since.
- **AI database**: stop the stack, put `xexam-ai-schema6-<date-time>.db` back as the AI database (`/data/xexam-ai.db` in the volume `xexam_ai_data`), change the three images to `:v1.2.1` and start. What was written under v1.3 in the AI database (chats and their branches, connections 2 and up, shared answers) is lost. v1.2 refuses a schema-7 file, so nothing is damaged silently; there is no way down from schema 7, because a branched chat cannot be flattened.
- v1.3 does not delete the pre-migration copies; remove old ones by hand when you no longer need them.

### 000.5 What has been tested

- Backend 548 tests, frontend 784, AI service 1044, all green, including: schema 7 to 8 and 6 to 7 on files with data (the old and the new file have the same tables), a v1.2-style session on a schema-8 file, the sign-in info eye (hidden: nothing new is recorded and saved values are not returned; open: shown, from trusted headers only; the explicit **Clear records** empties users, sessions and the log), the real-address matrix of Nginx, and the keys-never-leave rule over both services.
- Not covered by the release checks: a real provider, a real Cloudflare tunnel and the location headers; the phone layouts of the new pages were not walked on a real device.

## 00. Upgrading from v1.0 to v1.1

This section is numbered 00 because it is the newest one. The older sections keep their numbers.

### 00.1 What changes

- **The main database** moves from schema 6 to **schema 7**, once, on the first start of the new backend. It gets four columns: the **time limit** of an exam (`attempts.time_limit`, NULL for untimed exams and practice), how an exam ended (`attempts.ended`: submitted by hand, or by the countdown), and two JSON fields for AI-generated banks (`banks.ai_job`, `banks.ai_meta`). Every record saved before v1.1 stays as it is and is still scored the v1.0 way.
- **The AI database** moves from schema 4 to **schema 6**, once, on the first start of the new AI service (the schema 5 and schema 6 steps run in one transaction). It gets the bookkeeping of generation and import jobs, the day on which a generation or import was charged to the quota, the per-user **feature permissions** (`users.features_off`, all six features allowed by default), and the new limits: **generation calls per job (15)**, **import calls per job (25)**, the **import size (20 MB)** and the **import question limit (50)**. Your AI chats, memories, keys, settings and usage rows stay.
- **Features**: after the upgrade every user with the AI+ feature may use all six features (chat, hint, analysis, summary, AI generation and AI import), subject to the other rules (pause, the request limits, the daily generation quota). Use **AI settings › AI+ users** to switch any of them off for one user.
- **Lengths in tokens**: the answer length (`replyMaxTokens`, default 3000), the memory length (`memoryTokens`, default 1000) and the length of one generation or import call (`genCallTokens`, default 20000) are counted in tokens from v1.1 on. A value still at its old default moves to the new default; any other value you set is multiplied by 1.5 once, so the text you allow stays about the same.
- **Exams**: every exam now records whether it was submitted by hand or ended by the countdown, and a question left unanswered counts as **not completed**, not wrong. The rule applies to every exam, timed or not. Practice is unchanged.
- **Banks**: the bank management page has checkboxes, a toolbar for **Delete selected** and **AI generate**, and a separate **AI import** card. Nothing in your existing banks changes. AI-generated banks are shared like any new bank (`AI_BANKS_AUTO_SHARE`, default on).
- **Growth**: an AI-generated bank of 20 questions takes about 90 KB, and so does its copy in the bank download of everyone who can see it. At 5 generations a day that is about 170 MB a year; at 20 users with 3 generations a day it is about 2 GB a year. v1.1 does not cap the number of kept AI banks.

### 00.2 Upgrade

1. **Back up both data volumes** (stop first, as in the upgrade section of [README](../../README.md#upgrading)). The backend and the AI service each write their own copy before they change their database, but a copy of both volumes is the simplest way back.
2. **Pull the three images and start** (do not use `down -v`): `docker compose pull`, then `docker compose up -d`. The compose file, the volumes and `.env` stay as they are.
3. **Read the logs**: `docker compose logs backend` should say that it wrote `backups/xexam-schema6-<YYYYMMDD-HHMMSS>.db` and upgraded the database to schema 7; `docker compose logs ai-service` should say that it wrote `backups/xexam-ai-schema4-<YYYYMMDD-HHMMSS>.db` and upgraded the AI database to schema 6. Keep free space of about the size of both databases before you upgrade.

### 00.3 Checks after the upgrade

- `http://localhost:8765/api/info` shows `"version": "1.1"`.
- `docker compose ps` lists three healthy containers.
- Open **AI settings** (administrators): the tabs are **AI+ function**, **AI+ users**, **AI usage**, **AI banks** (new) and **AI safety**; the generation and import call limits read 15 and 25.
- Open **Bank management** and an exam: a timed exam can be started from Home, and a practice still works without a limit.

### 00.4 Rollback to v1.0

Both databases have to go back, because v1.0 refuses schema 7 and schema 6. Stop the containers, restore the copy `backups/xexam-schema6-<date-time>.db` as the main database and `backups/xexam-ai-schema4-<date-time>.db` as the AI database (the AI volume also keeps the key file `secret.key`, which does not change), and start the v1.0 images. Everything written after the upgrade is lost with the copies: new exams, AI-generated and imported banks, AI jobs and their usage rows.

### 00.5 What has been tested

The upgrade paths of v1.1 are tested with made-up rows in `backend/tests/v11_time_limit.rs` (the main database from schema 6) and `ai-service/tests/v11_store.rs` (the AI database from schema 4): one backup copy is written first, the old records stay as they were, and the new columns and indexes exist.

---

## 0. Upgrading from v0.9.1 to v1.0

This section is numbered 0 because it is the newest one. The older sections keep their numbers (0b for v0.9, 1 to 12 as before).

### 0.1 What changes

- **The AI database** moves from schema 3 to **schema 4**, once, on the first start of the new AI service. It gets three columns for the per-user request limits (**5 per minute, 20 per hour, 100 per day**) and one for the **answer length** (**1000 characters**). Your AI chats, memories, keys, settings and usage rows stay.
- **Memory length**: a **Characters per memory** limit still at the old default of 300 becomes 200 during that upgrade. Any other value you chose stays. If you want 300 back, set it again in AI settings › AI safety.
- **Per-message length**: the **Characters per message** setting is gone. A user message is always limited to 16000 characters (a fixed limit). Messages that were stored earlier are not cut.
- **Answer length**: answers are now limited to **Characters per answer**, which defaults to 1000 (it was not a setting before). Long answers, above all an AI summary over many wrong answers, may now stop at the limit; raise it in AI settings › AI safety if you need longer ones.
- **Request limits**: from the upgrade on, every AI request of a user counts toward the three per-user limits, administrators included. The day count starts again at 0:00 in the time zone of the AI service (its `TZ` setting; example `TZ=UTC`).
- **The main database does not change** (it stays at schema 6). Study records, banks and users are untouched.
- **Appearance and behaviour**: the phone layout, the AI+ panel that replaces the AI windows of a practice and its results, the separate **Check answer** and **Next** buttons, the centred footer line and the four tabs of AI settings. Nothing needs to be set up for them.

### 0.2 Upgrade

1. **Back up both data volumes** (stop first, as in the upgrade section of [README](../../README.md#upgrading)). The AI service writes its own copy before it changes its database, but a copy of both volumes is still the simplest way back.
2. **Pull the three images and start** (do not use `down -v`): `docker compose pull`, then `docker compose up -d`. The compose file, the volumes and `.env` stay as they are.
3. **Read the AI service's log**: `docker compose logs ai-service` should say that it wrote the copy `backups/xexam-ai-schema3-<YYYYMMDD-HHMMSS>.db` in the AI volume and upgraded the AI database to schema 4. The backup is about as large as the AI database; keep that much free space before the upgrade.

### 0.3 Checks after the upgrade

- `http://localhost:8765/api/info` shows the running version (`"version": "1.0"` in the release).
- `docker compose ps` lists three healthy containers.
- Open **AI settings** (administrators): the page has the four tabs **AI+ function**, **AI+ users**, **AI usage** and **AI safety**; under **AI safety** the request limits read 5, 20 and 100, and the answer length reads 1000.
- Decide the limits for your users: the defaults are a cost and abuse protection, not a usage quota. Check that the time zone of the AI service (`TZ`) gives the day reset you want.

### 0.4 Rollback to v0.9.1

The main database needs no restore (v0.9.1 opens its schema-6 database as it is). The AI database does: v0.9.1 refuses schema 4. To go back, stop the containers, restore the AI volume's copy `backups/xexam-ai-schema3-<date-time>.db` as the AI database, and start the v0.9.1 images. Everything in AI that was written after the upgrade is lost with it: chats, memories, saved answers, usage rows and changes to AI settings and limits. The study records of the same time stay.

### 0.5 What has been tested

Tests that pin this upgrade: the AI service upgrades schema 1, 2 and 3 to 4 with the same `sqlite_master` on every path, writes one backup copy first, keeps made-up rows and the old limit values, and sets a memory limit at the old default of 300 to 200 once. Tests are listed in the changelog entry of v1.0 (`docs/en/changelog.md`).

---

## 0b. Upgrading from v0.8 to v0.9

This section is numbered 0b so that the numbers of sections 1 to 12, which other documents link to, stay as they are.

### 0b.1 What changes

v0.9 adds the AI help for studying (AI hint and AI analysis in practice, AI analysis on result pages, AI summary of a record; section 5.1, 6.1 and 7.1 of [usage.md](usage.md)). It needs **no new `docker-compose.yml`**: the same three images with the new tag, the same volumes and networks, no new setting.

| | v0.8 | v0.9 |
|---|---|---|
| Images, containers, volumes, networks | `xexam-frontend`, `xexam-backend`, `xexam-ai-service`; `xexam_data`, `xexam_ai_data`, `xexam_link` | the same |
| Main database (`xexam.db`) | schema 5 | **schema 6** |
| AI database (`xexam-ai.db`) | schema 2 | **schema 3** |
| Nginx (inside the frontend image) | streams `send` and `retry` | also streams the new `study/ask` and `summary/ask`, and logs a request's path without its query string |

Two databases migrate, each by its own service, each on its first start and each after its own backup copy:

1. **The backend** writes a copy of the database to `/data/backups/xexam-schema5-<YYYYmmdd-HHMMSS>.db`, an ordinary schema-5 database that v0.8 can open, then moves the database to schema v6 in one transaction: two new columns on `attempts`, `session_key` (the 32-character key of the practice or exam session) and `started_at` (when it started, Unix seconds), both empty (`NULL`) for every record you already have. There is no index and no constraint, and nothing else changes: banks, records, accounts, settings and sessions stay, and nobody is signed out. The log says `Upgraded the database to schema 6. A copy of the previous version was saved first: /data/backups/xexam-schema5-….db (…)`. If the copy cannot be written, nothing changes and the backend does not start. From now on a saved practice or exam gets its key and start time (a browser that sends none, or an invalid one, saves them empty).
2. **ai-service** writes a copy of its database to `/data/backups/xexam-ai-schema2-<YYYYMMDD-HHMMSS>.db` in the volume `xexam_ai_data`, then moves it to schema 3 in one transaction by adding columns and indexes, never removing anything: on `chats` the `kind` (`chat`, `practice`, `exam` or `summary`; every existing chat becomes `chat`), `ref` (the session key or record this chat belongs to) and `record_id` (the record, for display only); on `messages` the `kind` (`chat`, `hint`, `explain` or `summary`; every existing message becomes `chat`), `question_id`, `question_no`, `source`, `lang` and the three summary counts `sample_used`, `sample_usable` and `sample_wrong`; on `usage` the `purpose` (`hint`, `explain` or `summary`; empty for every row you already have, which the statistics read as AI Chat); and three indexes that find a session's chat, the saved answers of a question and answers cut off by a restart. Settings, encrypted keys, limits, users, chats, messages and memories stay as they are. The log says `Upgraded the AI database to schema 3. A copy of the previous version was saved first: …`. If the copy cannot be written, ai-service does not start and changes nothing; the main app keeps working and **AI settings** says that ai-service is not running.

What to know about these copies:

- **Free disk space**: each copy is a full copy of its database (SQLite `VACUUM INTO`), so each service needs about as much free space in its volume as its database takes now. A full disk stops that service's start with the database unchanged.
- **They are never deleted automatically**, and they hold everything that existed when they were written, including chats and study records that you or an administrator delete later. Delete old copies yourself (`docker run --rm -v xexam_data:/data alpine rm /data/backups/<name>`) when you no longer need them, and keep them as private as the databases ([database.md](database.md)).
- A database that has been migrated opens again without a new copy and without writing anything.

What else you will notice:

- With the **AI+ feature**: the AI buttons and icons in practice, **AI analysis** on result pages and on a record's detail page, **AI summary** on Study records and Home, and the new chats of practices, exams and summaries in AI Chat. Users without the feature see nothing new. Users whose AI is paused or unavailable see the controls greyed out. Nothing has to be set up: the AI+ feature, the provider settings and the users' switches are as they were.
- **Records made before v0.9** have no session key. Their first AI analysis on the detail page starts a chat of its own for the record (usage.md, section 16.1); AI summaries work for them like for any record, except for records so old that they kept no per-question details.
- AI usage statistics get a **By feature** table (AI Chat, AI hint, AI analysis, AI summary).
- **The chat limits count the new chats**: every practice, exam and summarised record is a chat, and a request is two messages, so a very long practice uses up the default 400 messages per chat after about 200 requests (usage.md, section 17).

### 0b.2 Upgrade

```bash
# 1. Back up both volumes (stop first; the backup files hold your whole databases, do not commit them)
docker compose stop
docker run --rm -v xexam_data:/data -v "$PWD:/backup" alpine tar czf /backup/xexam-data-v08.tgz -C /data .
docker run --rm -v xexam_ai_data:/data -v "$PWD:/backup" alpine tar czf /backup/xexam-ai-data-v08.tgz -C /data .

# 2. Pull the three images and start (do not use down -v)
docker compose pull
docker compose up -d

# 3. Look for the two upgrade messages
docker compose ps
docker compose logs backend
docker compose logs ai-service
```

Your `docker-compose.yml` and `.env` stay as they are. **Source build**: `git pull && docker compose -f docker/docker-compose.build.yml up -d --build` from the repository root. To stay on v0.8 on purpose, keep the `:v0.8` tag on all three images.

### 0b.3 Checks after the upgrade

- [ ] `docker compose ps` lists `xexam-frontend`, `xexam-backend` and `ai-service`, all `healthy`.
- [ ] `version` in `/api/info` is `0.9`.
- [ ] The backend log has `Upgraded the database to schema 6. A copy of the previous version was saved first: /data/backups/xexam-schema5-…`, and `docker run --rm -v xexam_data:/data alpine ls -l /data/backups` lists that copy.
- [ ] The ai-service log has `Upgraded the AI database to schema 3. A copy of the previous version was saved first: …`, and `docker run --rm -v xexam_ai_data:/data alpine ls -l /data/backups` lists `xexam-ai-schema2-…`.
- [ ] You can sign in as before, the study records open, and **AI Chat** still lists your chats. **AI settings** says **ai-service is connected (version 0.9; …)** and the AI+ feature is as you left it.
- [ ] To try the new features, sign in with an account that has the **AI+ feature**, start a practice and press **AI hint** (usage.md, section 5.1).
- [ ] A second restart of the containers writes no new copy and no upgrade line.

### 0b.4 Rollback to v0.8

The two databases roll back differently.

- **The main database needs no restore.** v0.8 opens the schema-6 database as it is: it ignores the two new columns, and a record saved by v0.8 simply has an empty key and start time. This was tested: a v0.8 backend opened the migrated database, saved a record and left `meta.schema` at 6; v0.9 then started on it again without a new copy.
- **The AI database does need its copy.** The v0.8 ai-service refuses a database of schema 3 and exits with `The AI database … has schema 3, written by a newer ai-service; this version (schema 2) cannot open it. Run the newer version, or restore a backup.`, leaving the file unchanged. Until it is restored, v0.8 runs without AI. Restoring the copy **loses every change to the AI database since the upgrade**: all chats of that time (including all the chats of practices, exams and summaries), memories, setting changes, usage numbers and users' pauses. The key file `secret.key` is not in the database and stays, so the keys in the restored database still open.

```bash
# 1. Stop the new version (do not use down -v)
docker compose stop

# 2. Find the AI copy (the name has the schema it had: xexam-ai-schema2-…)
docker run --rm -v xexam_ai_data:/data alpine ls -l /data /data/backups

# 3. Put it back as xexam-ai.db (use the file name from step 2); leftover -wal / -shm files are removed first
docker run --rm -v xexam_ai_data:/data alpine sh -c 'rm -f /data/xexam-ai.db-wal /data/xexam-ai.db-shm && cp /data/backups/xexam-ai-schema2-20261112-140501.db /data/xexam-ai.db'

# 4. Pin the three images of docker-compose.yml to v0.8 (on Docker Hub, no login), then start:
#      image: docker.io/yuminspace/xexam-frontend:v0.8
#      image: docker.io/yuminspace/xexam-backend:v0.8
#      image: docker.io/yuminspace/xexam-ai-service:v0.8
docker compose up -d
```

For a source build, check out the release instead: `git fetch --tags && git checkout v0.8 && docker compose -f docker/docker-compose.build.yml up -d --build`.

If you would rather go without AI for a while, skip steps 2 and 3: the main app of v0.8 runs by itself, and you stop ai-service (`docker compose stop ai-service`) so that it does not keep restarting; leave its volume alone. To bring the main database back to exactly its state before the upgrade (not needed), restore `/data/backups/xexam-schema5-….db` as in section 1.4, which loses everything done in the main app since. To go back further than v0.8, use the rollback of the older section (1.4, 2.6, …) with the copy written then; those copies are still in `backups/`.

To upgrade again later, put the v0.9 images back. A main database that is still at schema 6 is not migrated again and gets no new copy; an AI database that you restored to schema 2 is migrated again, after a new copy.

### 0b.5 What has been tested

With synthetic data only (`backend/tests/v09_ai_study.rs`, `db_golden.rs`, `ai-service/tests/` and `tests/integration/`):

- A schema-5 database written by v0.8 migrates to schema 6 with exactly one `xexam-schema5-…` backup (`migrate_check.py`, case J); the old records have an empty key and start time; the result has the same `sqlite_master` as a new database; opening it again writes nothing. Older databases (schema 3 and 4) go to schema 6 in one step with one backup named after the schema they had.
- An AI database of schema 1 or 2 migrates to schema 3 with one `xexam-ai-schema<N>-…` copy and ends with the same `sqlite_master` as a new one; the saved-answer lookups use their indexes (no temporary sort); old usage rows read as AI Chat; the retention sweep deletes in small batches so that other writers are let in between them.
- The v0.8 backend opens and writes the schema-6 database, and the v0.8 ai-service refuses schema 3 and leaves the file unchanged (the rollback above).
- The full stack with the three images and a made-up provider (`stack_check.py`): hints and analyses with their saved answers and `Regenerate`, a hint that gives the answer away being withheld, a summary of at most 70 wrong answers, the one-answer-at-a-time refusal with its running answer, the access rules for users without the AI+ feature and for paused users, the chats of sessions and records, and no question, answer, session key or record time in any container's log (Nginx's access log included). Headless browser walks check the controls, the windows and the phone layout.

Not tested: a migration of **your own** databases, and real AI providers (CI uses a made-up one). Try the upgrade on a copy first if you want to be sure.

## 1. Upgrading from v0.7.5 (or v0.7) to v0.8

### 1.1 What changes

v0.8 adds AI Chat, which runs in a **third container**, `ai-service` (image `xexam-ai-service`), with its own database. That needs the **new `docker-compose.yml`**:

| | v0.7.5 | v0.8 |
|---|---|---|
| Images | `xexam-frontend`, `xexam-backend` | the same two plus **`xexam-ai-service`** |
| Containers (Compose services) | `xexam-frontend`, `xexam-backend` (`frontend`, `backend`) | the same two plus **`ai-service`** (`ai-service`) |
| Volumes | `xexam_data` | `xexam_data` (unchanged), **`xexam_ai_data`** (the AI database and its key file), **`xexam_link`** (only `internal.token`, the token the backend and the AI service share) |
| Networks | `internal` (no internet), `public` | the same plus **`egress`**, outbound internet for `ai-service` only; the backend still has none |

With the old compose file v0.8 starts too, and everything works except AI: AI Chat is not in the menu, **AI settings** says that ai-service is not running (with a hint to download the new compose file), and the backend logs `AI link off: cannot create /link/internal.token (…)`.

On its **first start** the backend does this before it accepts any request:

1. **Backup**: a copy of the database to `/data/backups/xexam-schema4-<YYYYmmdd-HHMMSS>.db`, an ordinary schema-4 database that v0.7.5 can open. If the copy cannot be written, nothing changes and the backend does not start.
2. **Migration to schema v5**, in one transaction: two new columns on `users`, `ai_enabled` (**AI+**, off for everyone) and `uid` (a random identity stamp per user). Nothing else changes: question banks, records, settings, accounts and sessions stay, and nobody is signed out. The log says `Upgraded the database to schema 5. A copy of the previous version was saved first: /data/backups/xexam-schema4-….db (…)`.
3. **The internal token**: the backend writes a random token to `/link/internal.token` (in `xexam_link`) and logs `AI link: /link/internal.token`. With `XEXAM_INTERNAL_TOKEN` set it writes no file and logs `AI link: using XEXAM_INTERNAL_TOKEN`.

ai-service, on its first start, creates its database `xexam-ai.db` (schema 2 in v0.8, schema 3 in v0.9) with the default settings and, unless `XEXAM_AI_SECRET_KEY` is set, the key file `secret.key` (`Created a new AI key file /data/secret.key`). Then it reports to the backend at once and every 25 seconds, so the main service knows that it is connected (a report older than 60 seconds counts as not connected); the backend never calls it. **AI is off** until an administrator sets it up in **AI settings** (section 17 of [usage.md](usage.md)); there is nothing to migrate for AI. (An AI database written by a pre-release build of v0.8, schema 1, is upgraded at ai-service's first start, after a copy `backups/xexam-ai-schema1-<YYYYMMDD-HHMMSS>.db` next to it in the AI volume; if that copy cannot be written, ai-service does not start and changes nothing. A schema-2 database from a later pre-release that lacks some of the schema-2 columns (`users.function_enabled`, `users.free_mode`, `messages.free`, `usage.tokens_cached`) gets them added at every start, with no copy: the step only adds columns that are missing and never removes one, so an old `chats.free` column of such a build is simply left unused.)

What else you will notice:

- **AI Chat** and **AI settings** in the menu (AI Chat only once AI is on, for administrators and for users who have the **AI+ feature**). **AI settings** always opens and shows whether ai-service is connected. Administrators find **AI chat mode** (**Normal** / **Free**) in **Settings** once AI is on.
- **User management**: an **AI+ feature** switch per user (shown only while ai-service is connected and the **AI+ feature** is on in AI settings; **Pause** and **Resume** for a user are in AI settings under **Users with AI+**); the bank choice of **Change** and **Add user** is more compact (**Manage banks**, **Select all** and **Deselect all** in one row on top of the list, **Include banks imported later** as the first entry of the list, the explanations as tooltips), and **Manage banks** moved there from **Edit**.
- A new icon in the header, the browser tab and on the phone's home screen (a home-screen icon added before may keep the old picture until you add it again).

Coming from v0.7 (not v0.7.5), the changes of v0.7.5 come along (see the [changelog](changelog.md)); the database steps are the same.

A database written by a **pre-release build of v0.7** (one that still has the column `users.all_banks`) is converted at the same start, in the same transaction and after the same one copy (`xexam-schema4-…`): a user who saw every bank gets every existing bank and **Include banks imported later**, any other user keeps their banks, and a user that an older version (v0.6 or before) added in between also gets every bank and **Include banks imported later**. The log then says `Converted the question bank access of a database written by a v0.7 pre-release to the released format and upgraded the database to schema 5. A copy of the previous version was saved first: <path>.` (section 6.5 of [database.md](database.md)).

### 1.2 Upgrade

```bash
# 1. Back up the volume (stop first; the backup file holds your whole database, do not commit it)
docker compose stop
docker run --rm -v xexam_data:/data -v "$PWD:/backup" alpine tar czf /backup/xexam-data-v075.tgz -C /data .

# 2. Replace docker-compose.yml with the new one (your .env stays; if you changed docker-compose.yml yourself, carry those changes over)
curl -fLO https://raw.githubusercontent.com/Yumin-Space/xexam/main/docker/docker-compose.yml

# 3. Pull the three images and start (do not use down -v)
docker compose pull
docker compose up -d

# 4. Look for the upgrade message and the AI service's start
docker compose ps
docker compose logs backend
docker compose logs ai-service
```

- **Optional, before step 3**: to keep the AI encryption key apart from the AI volume, put `XEXAM_AI_SECRET_KEY=<the output of openssl rand -base64 32>` in `docker/.env` now. Setting it later works too, but then the AI keys saved until then can no longer be decrypted and must be entered again. `XEXAM_INTERNAL_TOKEN` is optional as well; without it the backend creates the token (section 3 of [deployment.md](deployment.md)).
- **A host without `/etc/localtime`**: the new compose file mounts it into the AI service too; remove those six lines in both services and set `TZ` in `docker/.env`.
- **No internet for the AI service** (a host behind a strict firewall): Xexam works; only the test in AI settings fails with "Could not connect to the AI provider" until the AI service can reach the provider (or point it at a provider on your network).
- **Source build**: `git pull && docker compose -f docker/docker-compose.build.yml up -d --build` from the repository root; it builds `ai-service/Dockerfile` as the third image `xexam-ai-service:local`.
- **Coming from a pre-release build of v0.8** (a compose file in which the AI service is still called `ai`): `docker compose up -d --remove-orphans` with the new file removes the old container; the volumes `xexam_ai_data` and `xexam_link` are reused. No released version had that name, so a normal upgrade from v0.7.5 has nothing to remove.

### 1.3 Checks after the upgrade

- [ ] `docker compose ps` lists `xexam-frontend`, `xexam-backend` and `ai-service`, all `healthy`.
- [ ] `version` in `/api/info` is `0.8`.
- [ ] You can sign in as before; nobody had to sign in again. **User management** shows the same users, each plain user with the **AI+ feature** off (the switches appear once ai-service is connected and the **AI+ feature** is on in AI settings).
- [ ] **System log** has `AI: AI link: connected to the main service` once (the AI service's lines start with `AI: `; the line comes again after every reconnect, and `AI: AI link: lost the main service` when the link breaks).
- [ ] **AI settings** opens, says **ai-service is connected (version 0.8; …)** and **AI+ feature off**; the first chip reads **Connection not set** until you save a connection. To try AI: set up a provider, **Test**, switch on the **AI+ feature**, then ask something in **AI Chat**. When it says **Connection to ai-service** instead, read the cause it names (section 17 of [usage.md](usage.md)).
- [ ] Back up both data volumes from now on (section 5 and 8.4 of [database.md](database.md)).

### 1.4 Rollback to v0.7.5

Rolling back means **restoring the copy written before the migration**, putting the old `docker-compose.yml` back and running the v0.7.5 images. Everything done in the main app after the migration is lost (records, accounts, settings, imports); the AI data stays in its own volume, which v0.7.5 does not use.

```bash
# 1. Stop the new version (do not use down -v)
docker compose stop

# 2. Find the copy. After a normal stop any xexam.db-wal left in /data is empty.
docker run --rm -v xexam_data:/data alpine ls -l /data /data/backups

# 3. Put the copy back as xexam.db (use the file name from step 2); leftover -wal / -shm files are removed first
docker run --rm -v xexam_data:/data alpine sh -c 'rm -f /data/xexam.db-wal /data/xexam.db-shm && cp /data/backups/xexam-schema4-20261111-140501.db /data/xexam.db'

# 4. Put the docker-compose.yml of v0.7.5 back with its two images pinned (on Docker Hub, no login), then start:
#      image: docker.io/yuminspace/xexam-backend:v0.7.5
#      image: docker.io/yuminspace/xexam-frontend:v0.7.5
#    and remove the stopped `ai-service` container, which the old file no longer knows (its volume stays):
docker compose up -d --remove-orphans
```

For a source build, check out the release instead: `git fetch --tags && git checkout v0.7.5 && docker compose -f docker/docker-compose.build.yml up -d --build --remove-orphans`.

v0.7.5 can also open the migrated database itself (it ignores the two new columns), but keep that for emergencies: users it creates have no identity stamp and AI+ off, and v0.8 gives them a stamp at its next start (section 6.6 of [database.md](database.md)). The volumes `xexam_ai_data` and `xexam_link` can stay for a later upgrade; `docker volume rm xexam_ai_data xexam_link` removes them (this deletes the AI chats, memories and settings for good).

To upgrade again later, put the new `docker-compose.yml` back and run `docker compose pull && docker compose up -d`. The migration runs again and writes a new copy; the AI settings and chats in `xexam_ai_data` are still there, and the AI data of users that no longer exist is removed by the AI service.

### 1.5 What has been tested

With synthetic data only (`backend/tests/v08_ai_link.rs`, `db_golden.rs`, `ai-service/tests/` and `tests/integration/`):

- A schema-4 database written by v0.7.5 migrates to schema v5 with exactly one `xexam-schema4-…` backup; every user gets AI+ off and a unique 32-hex stamp; the result has the same `sqlite_master` as a new database, and opening it again writes nothing.
- A schema-3 database migrates to schema v5 in one step with one `xexam-schema3-…` backup; a pre-release v4 database is converted and migrated in one transaction.
- A user created by v0.7.5 after a rollback gets a stamp at the next start, without a backup.
- The backend creates `internal.token` (mode 0600) at the first `serve`, reuses it on restart, writes none with `XEXAM_INTERNAL_TOKEN`, and turns the link off with one log line for a value that breaks the rules.
- The full stack with the three images and a made-up provider (`stack_check.py`): sign-in, AI settings, test, enable, a streamed answer with keep-alive comments, the **AI+ feature** switched off during an answer (it ends within seconds) and a user paused during one (it ends at once), the AI+ switches of User management refused while ai-service is not connected or the feature is off, the connection state after `docker stop` and `docker start` of ai-service, the reply language, the administrator's free mode (set in Settings, kept per user, ignored when a send carries a `free`, the old per-chat route answering 404), the AI settings and data surviving a recreation of the containers, the main app answering while `ai-service` is stopped, which files each volume holds, and no key, question or answer in any log or response. Stop, Retry, the limits, the reports to the backend, the schema-2 upgrade of the AI database and the administrators' access are tested in `ai-service/tests/`.

Not tested: a migration of **your own** database, and real AI providers (CI uses a made-up one). Try the upgrade on a copy first if you want to be sure.

## 2. Upgrading from v0.6 to v0.8

### 2.1 What changes

On its **first start** on your data volume, the new version does this before it accepts any request:

1. **Backup.** It writes a copy of the current database to `/data/backups/xexam-schema3-<YYYYmmdd-HHMMSS>.db`. The copy is an ordinary schema-3 database that v0.6 can open. **If the copy cannot be written** (disk full, no permission), the backend refuses to start and leaves the database unchanged; the log says why.
2. **Migration to schema v5** (through v4), in one transaction:
   - **Every existing user keeps seeing every question bank**: each gets every existing bank and **Include banks imported later**, shown in User management as **Question banks: all, including ones imported later**. Users you add later start with no bank until you choose some.
   - **Every existing bank belongs to the first administrator** (the enabled administrator with the lowest id). Nobody is a **Bank manager** yet.
   - **Every existing study record is an exam.** Practice records appear only from now on.
   - **AI+** is off for everyone, and every user gets an identity stamp (the v5 step, section 1.1).
   - Question ids, statistics, records, settings, accounts and sessions do not change: nobody is signed out.
3. **WAL mode.** Only after a successful migration does the backend switch the database to WAL. While it runs, `xexam.db-wal` and `xexam.db-shm` sit next to `xexam.db`; a normal stop folds them back into `xexam.db` and empties `xexam.db-wal` (the two files may stay, empty and harmless).
4. **Old-layout questions are retired**, in one more transaction: every active question imported from the three- or four-column layout (with v0.6 that is every question) is retired, exactly as an import retires a question that left its workbook. The banks, their import history, the study records, the statistics and the wrong-answer notebook stay; the questions are no longer asked, and the bank list says that the bank's questions came from an older Excel layout. The log has one warning per bank, for example `retired 12 questions of bank "Kitchen basics": they use the old 3- / 4-column layout; import the six-column version of the workbook`. This check runs at every start (also of the command-line commands), and writes nothing when there is nothing to retire.

What else you will notice:

- **Only six-column workbooks can be imported.** A three- or four-column file is refused with a message that says so (section 12 of [usage.md](usage.md)).
- In **User management** each row shows the user's question banks, with **Change** to choose them (with **Manage banks**, the bank-manager permission), and the **AI+** switch. Taking a bank away keeps the user's records of it as history.
- **Start practice** next to **Start exam** on Home; practice records are tagged **Practice** in **Study records**.
- **System log** and **AI settings** in the menu of administrators, and **AI Chat** once AI is set up (section 1.1).
- The answers and explanations have their own **English** / **中文** / **日本語** switch on each card.

The new `docker-compose.yml` differs from the one of v0.6: it has the AI service with its two volumes and the network `egress` (section 1.1), it follows the host's time zone instead of a fixed default (it mounts `/etc/localtime`; a `TZ` in `docker/.env` overrides it), it limits each container's log to 3 files of 10 MB, and it passes `XEXAM_SQLITE_JOURNAL`, `XEXAM_COOKIE_SECURE`, `XEXAM_LOG` and the AI variables when `docker/.env` sets them. The v0.6 file still starts the new version, but without these changes and without AI. No variable has to be added to `.env`.

### 2.2 Before the upgrade

- [ ] Prepare a **six-column version of each workbook** you use: the question in English, Chinese and Japanese in columns A–C, the answer and the explanation in the same three languages in columns D–F, English authoritative (section 12 of [usage.md](usage.md)). Chinese and Japanese may stay empty where you have none.
- [ ] Note the number of questions of each bank (**Question banks** page), so you can compare after the upgrade.
- [ ] Submit or discard any unfinished exam: its questions are retired by the upgrade, so it cannot be continued afterwards.

### 2.3 Upgrade

```bash
# 1. Back up the volume (stop first; the backup file holds your whole database, do not commit it)
docker compose stop
docker run --rm -v xexam_data:/data -v "$PWD:/backup" alpine tar czf /backup/xexam-data-v06.tgz -C /data .

# 2. Replace docker-compose.yml with the new one (your .env stays; if you changed docker-compose.yml yourself, carry those changes over)
curl -fLO https://raw.githubusercontent.com/Yumin-Space/xexam/main/docker/docker-compose.yml

# 3. Pull the new version and start (do not use down -v)
docker compose pull
docker compose up -d

# 4. Look for the upgrade message, the name of the copy, the journal mode and the retired questions
docker compose logs backend
```

The log should contain `Upgraded the database to schema 5. A copy of the previous version was saved first: /data/backups/xexam-schema3-20261111-140501.db (…)`, `SQLite journal mode: wal` and one `retired … of bank "…"` warning per bank.

- **Time zone.** v0.6's compose file set a fixed time zone unless `.env` named another one. If the host's zone differs from the one your timestamps so far were written in, set that zone as `TZ` in `docker/.env` (for example `TZ=UTC`), so new records continue in it. Stored times are never rewritten.
- **A host without `/etc/localtime`**: Compose stops with "bind source path does not exist". Remove the six lines of that mount from `docker-compose.yml` (in the backend and in the AI service) and set `TZ` in `docker/.env` (section 3.1 of [deployment.md](deployment.md)).
- **Unusual storage**: if the volume is on a network filesystem or on a Windows folder bind-mounted into Docker Desktop, put `XEXAM_SQLITE_JOURNAL=delete` in `docker/.env` before step 3.
- **Source build**: `git pull && docker compose -f docker/docker-compose.build.yml up -d --build`, from the repository root, then `docker compose -f docker/docker-compose.build.yml logs backend`. Without Docker, the copy goes to `backups/` inside the data directory (`--data` or `XEXAM_DATA`).

### 2.4 Bringing the banks back

For each bank: **Question banks** → **Import Excel** → **Update a bank**, pick the bank under **Bank to update**, choose its six-column file and confirm.

- The bank keeps its name, its owner and the users who see it.
- The six-column layout recognises a question by its English text, the older layouts by their Japanese text, so every question of the file is **added** as a new question, without answer history, and the old ones stay retired. The study records from before still show the old questions, with a **Retired** tag; the wrong-answer notebook starts again with the new questions.
- Instead you can import the six-column file as a **New bank** and delete the old bank later (with or without its study records).

A user who opens Home before this sees a note that the banks have no active questions.

### 2.5 Checks after the upgrade

- [ ] `version` in `/api/info` is `0.8`, and `docker compose ps` lists three healthy containers.
- [ ] You can sign in as before; nobody had to sign in again.
- [ ] **User management**: every user from before shows **Question banks: all, including ones imported later**.
- [ ] **Question banks** lists the same banks, each **uploaded by** the first administrator. Before the update of section 2.4 each one shows its questions as retired (for example **0 questions (+25 retired)**, the number you noted); after it, the number of questions of the six-column file.
- [ ] **Study records** lists the earlier records, each tagged **Exam**.
- [ ] **System log** shows the start-up lines, among them `SQLite journal mode: wal` and the retired questions.
- [ ] Take a short practice and check its time in **Study records** against the clock. If it is off by whole hours, set `TZ` (section 2.3).
- [ ] Then decide, per user, which banks they see and who may manage banks; AI is off until you set it up (section 1.1).

### 2.6 Rollback to v0.6

Rolling back means **restoring the copy written before the migration** and running the v0.6 images. Everything done after the migration is lost: the six-column imports, new banks, records (also practice records), accounts, bank permissions and settings. The AI data stays in its own volume, which v0.6 does not use.

```bash
# 1. Stop the new version (do not use down -v)
docker compose stop

# 2. Find the copy. After a normal stop any xexam.db-wal left in /data is empty.
docker run --rm -v xexam_data:/data alpine ls -l /data /data/backups

# 3. Put the copy back as xexam.db (use the file name from step 2). Leftover -wal / -shm files belong to the
#    database being replaced and must not be applied to the copy, so they are removed first.
docker run --rm -v xexam_data:/data alpine sh -c 'rm -f /data/xexam.db-wal /data/xexam.db-shm && cp /data/backups/xexam-schema3-20261111-140501.db /data/xexam.db'

# 4. Pin the v0.6 images in docker-compose.yml (on Docker Hub, no login), remove the ai service (and its volumes)
#    from the file, or put the docker-compose.yml of v0.6 back, then start:
#      image: docker.io/yuminspace/xexam-backend:v0.6
#      image: docker.io/yuminspace/xexam-frontend:v0.6
docker compose up -d --remove-orphans
```

For a source build, check out the release instead: `git fetch --tags && git checkout v0.6 && docker compose -f docker/docker-compose.build.yml up -d --build --remove-orphans`.

Instead of step 3 you can also restore your own `.tgz` from section 2.3 (the restore command is in section 5 of [database.md](database.md)); it holds the same data. Step 4 is needed either way.

v0.6 can open the migrated database itself, and it switches the file back from WAL by itself, but keep that for emergencies: v0.6 does not know the rules of v0.7 and later, so every user sees every bank again, only administrators can manage banks, practice records look like exams, and it knows nothing of six-column questions. When the new version starts again afterwards, it repairs what v0.6 left behind (section 6.6 of [database.md](database.md)): banks v0.6 imported get an owner and reach the users with **Include banks imported later**, users v0.6 created get every bank and **Include banks imported later**, and questions v0.6 imported are retired again.

To upgrade again later, put the new images back (tag `:latest`) and run `docker compose pull && docker compose up -d`. The migration runs again and writes a new copy.

### 2.7 What has been tested

With synthetic data only (`backend/tests/v065_bank_access_and_practice.rs`, `v07_bank_access_and_layouts.rs`, `v07_wal_and_logs.rs`, `v08_ai_link.rs`, `db_golden.rs` and `tests/integration/`):

- A schema-3 database migrates to schema v5 with one backup; every user keeps every bank, with banks imported later included, and the banks belong to the first administrator.
- If the backup cannot be written, the database is not migrated and not changed.
- Old-layout questions are retired once, keeping the banks, the records and the notebook; a second start writes nothing. The command-line program does the same and logs one English line per bank. A bank whose questions were retired is updated with a six-column file: the new questions are added, the old ones stay retired.
- Uploads, the import confirm and the command-line `import` refuse three- and four-column workbooks before anything is staged or written.
- A database of a v0.7 pre-release is converted after a backup, also with users an older version added in between.
- A new database is in WAL mode; `XEXAM_SQLITE_JOURNAL=delete` keeps DELETE, and the next start without it is in WAL again with every record still there.
- A migration of a database in WAL mode backs up the changes that were still in the `-wal` file; the backup is one self-contained file.
- Opening an up-to-date database writes nothing (the database file and its `-wal` file stay the same, byte for byte).
- After `docker stop` the volume holds `xexam.db` and no `-wal` content.
- `reset-password` works while the server has the database open.
- Banks without an owner, banks and users that an older version created after a rollback, and stale bank assignments are repaired at the next start.

Not tested: a migration of **your own** database. Do it on a copy first if you want to be sure (as in section 6, "Try the new version on a copy").

## 3. Upgrading from v0.5 or v0.5.1 to v0.8

### 3.1 What changes

v0.6 introduced accounts and sign-in (schema v3); v0.7 added question banks per user (schema v4), v0.8 the AI+ switch and the identity stamp (schema v5). Coming from v0.5, all of them run on the **first start**, in one transaction, after one copy:

1. **Backup** to `/data/backups/xexam-schema2-<YYYYmmdd-HHMMSS>.db`, an ordinary schema-2 database that v0.5 can open. If it cannot be written, nothing changes and the backend does not start.
2. **Accounts.** The default administrator `admin` with the password `admin` is created, and it **receives everything you had**: every study record, the wrong-answer notebook and the settings.
3. **Schemas v4 and v5** as in section 2.1: `admin` sees and owns every bank; AI is off (section 1.1).
4. **WAL mode**, then the **retirement of the old-layout questions**, as in section 2.1: prepare six-column workbooks first (section 2.2) and update the banks with them afterwards (section 2.4).

From now on, every page needs a signed-in account. Each person gets their own records, notebook and settings, so add the other people under **User management** instead of sharing `admin`.

### 3.2 Upgrade

Use the commands of section 2.3, with `xexam-data-v05.tgz` as the backup name. A `docker-compose.yml` from v0.5 still pulls from `ghcr.io`; the new one pulls from Docker Hub (`docker.io/yuminspace/…`, public, no login). The project name and the volume `xexam_data` are the same, so your data stays.

The log names the copy (`…/backups/xexam-schema2-….db`), says `Created the default administrator 'admin' (password 'admin'); sign in and choose a new password.` and lists the retired questions per bank.

### 3.3 Checks after the upgrade

- [ ] Open Xexam: the sign-in page appears and names the account `admin`. Sign in with `admin` / `admin` and choose your own password right away; until then anyone who can open the page can use the default password.
- [ ] `version` in `/api/info` is `0.8`.
- [ ] **Question banks** lists the same banks, with their questions retired until you update them (section 2.4); **Study records** lists your earlier exams; **Settings** has your theme and languages.
- [ ] Add the other users and choose their question banks.

### 3.4 Rollback to v0.5

As in section 2.6, with the copy `xexam-schema2-….db` and the images `docker.io/yuminspace/xexam-backend:v0.5.1` and `docker.io/yuminspace/xexam-frontend:v0.5.1` (v0.5 itself is only on the private mirror `ghcr.io/yumin-space/`). Everything done since the upgrade is lost, the accounts included.

Do not run v0.5 on the migrated database: it knows no accounts, so its records would belong to nobody (the new version gives them to the first administrator at its next start), and every person would share one notebook again.

## 4. Upgrading from v0.4 to v0.5 and later

Today this upgrade goes straight to v0.8: the steps below, then those of sections 3.1 and 2.1, run in **one** transaction after **one** copy, `xexam-schema1-<YYYYmmdd-HHMMSS>.db`, and then the old-layout questions are retired (section 2.1, step 4). You then sign in with `admin` / `admin` (section 3.3), and `admin` receives all your data. Prepare six-column workbooks first (section 2.2).

### 4.1 What happens to your data

v0.5 added question banks, per-question exam detail and settings to the database (schema v2). The change is purely additive: nothing is rebuilt, and no question id changes. Full details: section 6 of [database.md](database.md).

On the **first start** of the new version on your data volume, the backend does this before it accepts any request:

1. **Backup.** It writes a copy of the current database to `backups/xexam-schema1-<YYYYmmdd-HHMMSS>.db` next to `xexam.db` (in Docker: `/data/backups/` in the volume `xexam_data`). The copy is an ordinary schema-1 database that v0.4 can open. An existing file is never overwritten. **If the copy cannot be written** (disk full, no permission), the backend refuses to start and leaves the database unchanged; the log says why.
2. **Migration**, in one transaction:
   - Everything you have becomes **one question bank** (the "legacy" bank). It is named after the file of your latest import, without `.xlsx` (`Question bank` if there was never an import).
   - **Question ids stay the same.** So the per-question statistics, the wrong-answer notebook and the exam history stay linked to their questions (which are then retired, see above).
   - **Exam records are kept** and are attributed to that bank. They have no per-question detail, because v0.4 never stored the answers you gave: their detail page shows the score but no answers, and offers no "retake" buttons.
   - The import history is kept and attributed to that bank.

Later starts check, read-only, that nothing needs fixing, and write nothing.

What else you will notice:

- **The interface starts in English.** v0.4 was in Chinese. Change it under **Settings → Language**, or on the sign-in page. The setting is stored in the database, so you set it once for every browser and device.
- The theme follows the system (light or dark) until you choose one in **Settings**.
- The question language (Japanese / Chinese / English on the exam page) used to be remembered by the browser. The first time a browser opens the new version, it moves that choice to the server, if the server still has the default (Japanese).
- An unfinished exam saved in the browser cannot be continued after the upgrade, because its questions are retired with the rest: submit it before you upgrade.
- The database now keeps the newest 500 records per user instead of 60.

### 4.2 Upgrade

Nothing has to change in `.env`, the port or the volume (see section 6 of [deployment.md](deployment.md)). Only the image source in `docker-compose.yml` is new since v0.5.1: it pulls from Docker Hub (`docker.io/yuminspace/…`, public, no login) instead of `ghcr.io`. Replace your `docker-compose.yml` with the new one, or change its two `image:` lines to `docker.io/yuminspace/xexam-backend:latest` and `docker.io/yuminspace/xexam-frontend:latest`. The project name and the volume `xexam_data` are unchanged, so your data stays.

```bash
# 1. Back up the volume (stop first; the backup file holds your whole database, do not commit it)
docker compose stop
docker run --rm -v xexam_data:/data -v "$PWD:/backup" alpine tar czf /backup/xexam-data-v04.tgz -C /data .

# 2. With the new docker-compose.yml in place (Docker Hub images), pull the current version and start (do not use down -v)
docker compose pull
docker compose up -d

# 3. Look for the upgrade message and the name of the copy
docker compose logs backend
```

The log should contain a line like `Upgraded the database to schema 5. A copy of the previous version was saved first: /data/backups/xexam-schema1-20261110-140501.db (…)`, then the retired questions of the bank. The program writes that copy itself; the `.tgz` from step 1 is your own second copy, outside the volume.

For a source build: `git pull && docker compose -f docker/docker-compose.build.yml up -d --build`, from the repository root. The compose files are now in `docker/`: before that command, move a `.env` at the repository root to `docker/.env` (Compose no longer reads it at the root), and add `-f docker/docker-compose.build.yml` to the later commands, such as `logs backend`. Without Docker, the copy goes to `backups/` inside the data directory (`--data` or `XEXAM_DATA`).

### 4.3 Checks after the upgrade

- [ ] Sign in with `admin` / `admin` and choose your own password (section 3.3).
- [ ] `version` in `/api/info` is the version you installed (`0.8` today).
- [ ] **Question banks** lists one bank with all your sheets; its questions are counted as retired, as many as the old version had.
- [ ] **Study records** lists your earlier exams.
- [ ] Update the bank with the six-column version of your workbook (section 2.4), answer a few questions and submit. The new record shows each question with your answer.

### 4.4 Rollback to v0.4

Rolling back means **restoring the copy written before the migration** and running the v0.4 images. Everything done in the new version after the migration is lost: accounts, new banks, imports, records and settings.

```bash
# 1. Stop the new version (do not use down -v)
docker compose stop

# 2. Find the copy. After a normal stop there is no xexam.db-journal in /data, and any xexam.db-wal is empty.
docker run --rm -v xexam_data:/data alpine ls -l /data /data/backups

# 3. Put the copy back as xexam.db (use the file name from step 2; leftover -wal / -shm files are removed first)
docker run --rm -v xexam_data:/data alpine sh -c 'rm -f /data/xexam.db-wal /data/xexam.db-shm && cp /data/backups/xexam-schema1-20261110-140501.db /data/xexam.db'

# 4. v0.4 is only on the private GHCR mirror (Docker Hub starts with v0.5.1): run docker login ghcr.io, then set the two images in docker-compose.yml to
#      image: ghcr.io/yumin-space/xexam-backend:v0.4
#      image: ghcr.io/yumin-space/xexam-frontend:v0.4
docker compose up -d
```

For a source build, check out the tag instead: `git checkout v0.4 && docker compose -f docker-compose.build.yml up -d --build` (at that tag the compose files are still at the repository root, and Compose reads `.env` from there: if you have a `docker/.env`, copy it to the repository root first).

Instead of step 3 you can also restore your own `.tgz` from section 4.2 (the restore command is in section 5 of [database.md](database.md)); it holds the same data. Step 4 is needed either way.

**Do not run v0.4 on the migrated database** instead of restoring the copy. v0.4 can open the file, but it sees all banks as one: its next import would retire the questions of every other bank, it cuts the exam records back to 60, and it knows nothing about accounts. If it happens anyway, the next start of the new version attaches the questions v0.4 added to the legacy bank (section 6.6 of [database.md](database.md)) and retires them, because they are in an old layout.

To upgrade again later, put the new `docker-compose.yml` back (Docker Hub images, tag `:latest`) and run `docker compose pull && docker compose up -d`. The migration runs again and writes a new copy.

### 4.5 What has been tested

With synthetic data only (`backend/tests/v05_banks_records_settings.rs`, `v06_users_and_sign_in.rs` and `db_golden.rs`):

- `sample.db`, a database written by the old Python implementation, migrates to the current schema keeping every question id, statistic, exam record and import. Its backup still reports schema 1 and holds exactly the original rows.
- A schema-1 database goes to the current schema in one step, with one schema-1 backup.
- If the backup cannot be written, the database is not migrated and not changed.
- Opening a migrated database again does not change a single byte of the file.
- Rows written by an older binary into a migrated file are attached to the legacy bank at the next start.
- A new database and a migrated one have the same structure.

Not tested: a migration of **your own** database. Do it on a copy first if you want to be sure (as in section 6, "Try the new version on a copy").

## 5. Python to Rust (v0.3): principles

1. **Never touch the original.** Back up first, try on a copy first, and switch only after you have confirmed everything is fine.
2. **You could always go back.** Up to v0.4 both implementations read and write the same file with the same schema (a round trip in both directions was rehearsed with synthetic data during the migration, see section 9). Since v0.5 the schema has new tables and columns; going back from a newer version means restoring the copy written before the migration (section 4.4).
3. **The old implementation is not lost.** The old code is at the Git tag `python-v0.2`; the old image `ghcr.io/yumin-space/xexam:v0.2.2` (along with `:v0.2` and `:v0.2.1`) is still on GHCR (private: `docker login ghcr.io` to pull it; it is not on Docker Hub) and will not be overwritten by the new pipeline. The repository's main branch no longer contains any Python code.

## 6. Python to Rust: pre-upgrade checklist

Do these in order. No step modifies your real database.

- [ ] **Back up the data volume** (stop the old container first):

  ```bash
  docker compose stop
  docker run --rm -v xexam_data:/data -v "$PWD:/backup" alpine tar czf /backup/xexam-data.tgz -C /data .
  docker compose start
  ```

  The backup file holds your entire database, so **do not commit it to Git** (`.gitignore` already ignores `*.tgz`).
- [ ] **Verify the backup**: extract it to a temporary directory; `sqlite3 work/xexam.db "PRAGMA integrity_check"` must print `ok` (for the command, see section 5 of [database.md](database.md)).
- [ ] **Try the new version on a copy**: put the extracted `xexam.db` into an **empty temporary directory** and start the backend with it, without using the named volume:

  ```bash
  cd backend && cargo build --release
  XEXAM_DATA=/tmp/xexam-trial ./target/release/xexam-backend serve --no-browser
  ```

  Open <http://localhost:8765> in a browser, sign in with `admin` / `admin`, and check that **Question banks** shows one bank whose retired questions are as many as the old version's questions, and that **Study records** has the old score history. (This also migrates the copy to the current schema, retires its questions, which are all in an old layout, and writes `backups/` into the temporary directory.) **Delete this temporary directory when you are done.**
- [ ] **Write down the old version's numbers**: `bankCount` from the old version's `/api/info` (the number of its questions; the current version no longer reports it), the total number of questions on the home page, and the number of exams in the score history. Compare them after the upgrade.
- [ ] **Prepare a six-column version of your workbook** (section 12 of [usage.md](usage.md)): the current version retires every question of the old layouts at its first start (section 2.1).
- [ ] **Check `.env`**: you can keep the old values for `XEXAM_ALLOWED_HOSTS`, `XEXAM_BIND` and so on; the variable names have not changed.

## 7. Python to Rust: upgrade

```bash
# 1. Stop the old version (do not use down -v)
docker compose stop

# 2. Remove the old container (only the container, not the volume). The old version's service and container are both named xexam, while the new ones are frontend / backend.
#    If you do not remove it, the old container stays in the project as an "orphan", and its port mapping still holds 8765, so the new xexam-frontend cannot start because of the port conflict.
docker rm xexam

# 3. Put the new docker-compose.yml in place (it now has three services, pulled from Docker Hub without login), then pull and start (same volume xexam_data, same port)
docker compose pull
docker compose up -d

# 4. Check that the containers are healthy (three since v0.8)
docker compose ps
```

`docker rm xexam` removes only the **container**. It does not remove the data volume `xexam_data` (that would take `docker volume rm` or `down -v`; do not do either).

Upgrading from Python straight to the current version also runs the schema migrations of section 4, with their backup copy, creates the account `admin` / `admin` that receives your data, and retires every question, because they are all in an old layout; then update the bank with your six-column workbook (section 2.4).

Checks after the upgrade:

- [ ] `version` in `/api/info` is the version you installed (`0.8` today).
- [ ] After signing in, **Question banks** shows the one bank with as many retired questions as the old version had questions (the old `bankCount`), and **Study records** has the same score history as before the upgrade.
- [ ] Update the bank with your six-column workbook (section 2.4), answer a few questions and submit; the score appears in the history.

> ⚠️ The old and new versions use the same project name (`xexam`), the same volume (`xexam_data`) and the same port, so they **cannot run at the same time**. The old version's container is named `xexam`, while the new version's are `xexam-frontend` and `xexam-backend`, so during the upgrade you have to remove the old container as in step 2 above.

## 8. Python to Rust: rollback

You can go back to the Python version. The old version's compose file is no longer on the main branch; get it from the tag:

```bash
# 1. Stop the new version (do not use down -v)
docker compose stop

# 2. Get the old version's compose file and start the old version (it pulls ghcr.io/yumin-space/xexam:latest from the private GHCR, so run docker login ghcr.io first; to pin a version, change :latest to :v0.2.2)
git show python-v0.2:docker-compose.yml > docker-compose.legacy.yml
docker compose -f docker-compose.legacy.yml up -d
```

Note that the image tag in the old compose file is `:latest`, **and `ghcr.io/yumin-space/xexam:latest` is no longer updated after this release** (the new images use the two new names `xexam-frontend` / `xexam-backend`), so it still points to the last Python version. To be safe, write `:v0.2.2` explicitly.

**From v0.3 or v0.4** the old version keeps using the same `xexam_data` volume with no data loss. It can read everything those versions wrote (newly imported questions, new scores), because they do not change the schema, do not change `journal_mode`, do not add tables or columns, do not set `user_version`, and leave no `-wal` / `-shm` files behind.

**From v0.5 or later** the schema has new tables and columns (and since v0.7 the file is in WAL mode). First restore the schema-1 copy written before the migration, as in steps 1–3 of section 4.4, then start the old version as above.

If you also want the data to go back to the state before the upgrade (for example, you suspect the new version corrupted it):

```bash
docker compose stop
docker run --rm -v xexam_data:/data -v "$PWD:/backup" alpine sh -c 'rm -f /data/xexam.db-wal /data/xexam.db-shm && tar xzf /backup/xexam-data.tgz -C /data'
docker compose -f docker-compose.legacy.yml up -d
```

> The restore overwrites `/data` with the backup, so every change made in the new version after the backup is lost. Do it only when you are sure you need it.

Rolling back without Docker: `git checkout python-v0.2` and run `python server.py` (requires Python 3.8+), with the data directory pointing at the same `xexam.db`.

## 9. Python to Rust: what has been rehearsed

The following was done during the migration (while the Python code was still in the repository), **using fictional data only**; no real database was read, printed or modified. The scripts that depended on running the old Python code live were removed along with that code; the conclusions continue to be guarded by the committed snapshot tests.

| Rehearsal | How it was done | Result | What guards it now |
|---|---|---|---|
| Round trip between old and new | Old Python creates the database → backup + `integrity_check` → Rust reads it on a **copy** (byte-for-byte identical to the old version) → Rust imports a second workbook and records scores → old Python reads back (identical to Rust) → old Python writes again → Rust reads again | 19 checks passed, including "the sha256 of the original is unchanged before and after" | `db_golden.rs`: `sample.db` (a database written by the old implementation) migrates to the current schema keeping every row, and the migrated file is byte-for-byte unchanged by any later open, read and close; the `sqlite_master` of a newly created database is the old implementation's text plus the later additions |
| Golden-response comparison | 172 responses from the old version compared item by item with the new backend | 0 unexplained differences, 23 recorded | `api_golden.rs` replays them on every CI run |
| Security probes | 340 malicious requests sent to both the old and the new version, in 3 configurations | 0 unexplained differences, 51 recorded (most of them are cases where the new version is stricter) | The probe script has been removed; the guards and upload checks are tested by `api_*.rs`, `parser_hardening.rs` and `nginx_check.py` |
| Parser differential | 49 synthetic workbooks, with the output of the old parser and the Rust parser compared item by item; 85 question-id vectors | All identical | `parser_golden.rs` and `id_vectors.rs` run on every CI run |
| Full stack | `tests/integration/stack_check.py`: bring up the stack, upload / import / submit, the data is still there after the container is deleted and the stack is recreated on the same volume, `docker stop` timing, non-root | 34 checks passed at the time (the script has grown since) | Runs on every CI run |

## 10. Python to Rust: intentional differences from the old version

The full table, with reasons, is in section 7 of [api.md](api.md). Every entry is recorded in `tests/baseline/golden/allowed_differences_rust.json` and `allowed_probe_differences_rust.json`, and a difference that is not recorded makes the tests fail. The only ones visible to users are:

| Item | Change |
|---|---|
| Architecture | One container becomes two (nginx + backend); the images are named `xexam-frontend` / `xexam-backend`; `XEXAM_BIND` now points at Nginx |
| Stricter upload checks | A DOCTYPE in any encoding / extension is rejected; duplicate ZIP entry names are rejected; the number of entries, the XML depth and the number of nodes are capped |
| Error messages | All other Chinese messages are kept verbatim; in two places that used to echo the text of a library exception, a fixed Chinese message is used instead |
| Grading | Still done in the browser, not moved to the server (the spec recommends moving it; here it is intentionally not moved, for the reasons see Q3 in [migration-plan.md](../zh-CN/history/migration-plan.md)) |
| iPhone | Requires iOS 15.4 or later (build target `safari15`) |
| No longer provided | The backend no longer serves static files (Nginx does); the `Server` response header |

The API changes of later versions are listed in [api.md](api.md).

## 11. Python to Rust: what has not been verified

- A real Cloudflare Tunnel / Access, a real iPhone, and running a full deployment on an ARM server outside CI (arm64 has only been built and started in CI, on GitHub's ARM runner).
- A rehearsal with a copy of **your own real database**: I do not have it and should not read it; please do it on your own machine following section 6.

## 12. Upgrading from v0.1

From v0.2 on, the database file is named `xexam.db`. If the data directory contains another `.db` file but no `xexam.db`, the backend's startup log warns you; rename that file to `xexam.db` and start again (the old file is neither deleted nor read). The old `progress.json` is checked once on every startup, and it is migrated only if the database has no study records yet and no migration has happened before; it is migrated only once and never overwrites existing records.
