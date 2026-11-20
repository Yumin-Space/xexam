**English** | [日本語](../ja/database.md) | [简体中文](../zh-CN/database.md)

# Database

> The database is SQLite, **a single file, `xexam.db`**. The Rust backend reuses the old Python version's database: the original table definitions (schema v1) are kept **character for character**, and every later schema only adds to them (new tables and new columns; nothing is rebuilt or rewritten): **schema v2** (v0.5: question banks, exam detail, settings), **schema v3** (v0.6: users and sign-in), **schema v4** (v0.7: question banks per user, bank owners, practice records), **schema v5** (v0.8: the per-user AI+ feature grant and an identity stamp) and **schema v6** (v0.9: the key and start time of the practice or exam session a record was saved from), and **schema v7** (v1.1: the time limit and the end of an exam, and the AI-generated banks), **schema v8** (v1.3: sessions, the activity log and system settings), **schema v9** (v1.4: the last-activity columns) and **schema v10** (v1.6: the per-user permission to download question banks). An existing database is migrated automatically at the first start of a newer version, after a backup copy is written (section 6). Since v0.7 the database runs in **WAL mode** (section 2).
> Since v0.8 ai-service has a **second, separate database**, `xexam-ai.db`, in its own volume (section 8); its schema is **9** since v1.9 (8 since v1.4, 7 since v1.3, 6 since v1.1, 4 since v1.0, 3 since v0.9). The two services never open each other's database. **v1.1 changes the main database** to schema v7 (section 6); the AI database does not touch it.
> Evidence: [audit/database.md](../zh-CN/history/audit/database.md); golden tests: `backend/tests/db_golden.rs` (run against `tests/baseline/fixtures/sample.db`, built by the old Python implementation, and the committed snapshots). The migrations and their backups are tested in `backend/tests/v05_banks_records_settings.rs` (v2), `v06_users_and_sign_in.rs` (v3), `v065_bank_access_and_practice.rs` (v4), `v07_bank_access_and_layouts.rs` (the bank access of v4, the conversion of pre-release databases, the retirement of old-layout questions), `v07_wal_and_logs.rs` (WAL mode), `v08_ai_link.rs` (v5) and `v09_ai_study.rs` (v6, the study routes of the backend); the AI database in `ai-service/tests/` (`v09_store.rs` for schema 3) and the unit tests of `ai-service/src/store/`.

## 1. Where the file lives

| How you run it | Location |
|---|---|
| Docker (recommended) | Named volume `xexam_backend_data`, mounted at `/data` in the container; the file is `/data/xexam.db`. Updating or deleting the container does not lose it |
| Without Docker | `xexam.db` in the data directory (default `./data`, or `XEXAM_DATA`, or `--data`) |

The volume's real name is `<project name>_<volume name>` = `xexam_backend_data`, and `docker volume ls` shows it. **Do not run `docker compose down -v`**: `-v` deletes the volume along with the containers.

Other files next to the database:

- **`xexam.db-wal` and `xexam.db-shm`** exist while the backend runs (WAL mode, section 2). The `-wal` file holds the latest committed changes until SQLite copies them into `xexam.db`. When the backend stops normally everything is copied into `xexam.db` and `xexam.db-wal` is emptied; the two files may stay, empty and harmless. They belong to the database: never delete them while the backend runs, and copy them together with `xexam.db` (section 5).
- **`backups/`** receives the copy written before a schema migration (section 6), for example `/data/backups/xexam-schema3-20261111-140501.db`. The program never deletes these files; remove them yourself once you no longer need them.

What is kept where:

| Where | What |
|---|---|
| `xexam.db` in the volume (with `-wal` / `-shm` while running) | The question banks with their owners and questions (retired ones included), the import history, the users (password hashes only), their sessions and which banks each user sees, each user's per-question statistics (wrong-answer notebook), study records (exams and practice) with their per-question detail and, since v0.9, the key and start time of the session each was saved from, and settings (theme, interface language, auto-advance, question language) |
| `backups/` in the volume | The copies of the database written before each schema migration (`xexam-schema1-…` up to `xexam-schema5-…`) |
| Memory of the backend only | The System log, the sign-in locks and the import previews; all of them start empty after a restart |
| The volume `xexam_ai_auth` (v0.8) | Only `internal.token`, the token the backend and ai-service share; written by the backend, read by ai-service. No database |
| The volume `xexam_ai_data` of ai-service (v0.8) | `xexam-ai.db` (AI settings, encrypted provider keys, safety limits, chats, memories, usage statistics; since v0.9 the chats include one chat per practice session, per exam and per record summary, with the question texts and the AI answers they hold), the key file `secret.key` and the folder `backups/` with the copies written before an AI schema upgrade (section 8) |
| The browser (`localStorage`) | Per user: the unfinished exam or practice (question ids, answers, marks, checked questions, time used; no question text) and a copy of the settings to paint the first screen |

## 2. Connection settings

| Setting | Value | Why |
|---|---|---|
| `journal_mode` | `WAL` (since v0.7); `DELETE` with `XEXAM_SQLITE_JOURNAL=delete` | A commit writes only to the `-wal` file, and a reader (for example a command-line tool started next to the server) does not block the writer. See 2.1 |
| `synchronous` | `FULL` | Each commit waits until it is on disk (in WAL mode: one fsync of the `-wal` file), so nothing committed is lost on a power failure |
| `foreign_keys` | `ON` | (The schema actually has no foreign keys; kept the same as the old version) |
| `busy_timeout` | 5000 ms | When another connection holds a lock, wait up to 5 seconds |
| Automatic checkpoint | SQLite's default (every 1000 pages) | Keeps the `-wal` file small while the backend runs |
| Transactions | `BEGIN IMMEDIATE … COMMIT`, `ROLLBACK` on failure (a failed `COMMIT` is also rolled back) | Either everything succeeds or nothing changes |

The backend has only **one connection**, kept behind a mutex. Database calls run on a dedicated thread (`spawn_blocking`), so they do not block Tokio's executor threads, and the lock is never held across an `.await`. This corresponds one to one with the old version's "one connection + RLock".

### 2.1 WAL mode (v0.7)

- **Persistent.** The journal mode is stored in the header of `xexam.db`. The backend asks for it every time it opens the database (the server and the command-line commands alike), and only **after** a successful migration: if an open fails, for example because the pre-migration backup cannot be written, an older database file is left exactly as it was.
- **The opt-out.** `XEXAM_SQLITE_JOURNAL=delete` (in `docker/.env`, or in the environment without Docker) keeps the rollback journal of v0.6 and older. Use it only when the data directory is on storage where WAL's shared-memory file `xexam.db-shm` is unreliable: a network filesystem, or a Windows folder bind-mounted into Docker Desktop. Any other value, or no value, means WAL. The next start without the variable switches back to WAL.
- **When SQLite refuses.** The backend never fails to start because of the journal mode. If SQLite cannot switch to WAL (another process holds the database in a way that prevents it, or the filesystem does not support it), the log has a warning such as `SQLite did not switch the journal mode to wal; continuing in mode delete`, and the backend continues in the mode SQLite reports.
- **The startup line.** Each start logs the mode in effect once: `SQLite journal mode: wal`, or `SQLite journal mode: delete (XEXAM_SQLITE_JOURNAL=delete)`.
- **At shutdown.** On a normal stop (`docker compose stop`, Ctrl+C) the backend runs `PRAGMA wal_checkpoint(TRUNCATE)`: everything in `xexam.db-wal` is copied into `xexam.db` and the `-wal` file is emptied. Closing the last connection can then also remove `-wal` and `-shm`, but they may stay (for example on Windows), empty and harmless. The log says `WAL checkpoint at shutdown: everything is in xexam.db, the -wal file is empty`. If another process still has the database open at that moment (for example a command-line command), the checkpoint cannot finish; the log says so in a warning, and SQLite completes it later.
- **Command-line commands while the server runs.** `reset-password`, `import` and `migrate-progress` open the same file from another process. In the same container (`docker compose exec backend …`) or on the same machine this is safe in WAL mode too.
- **Older versions.** Xexam v0.6 and older ask for `DELETE` when they open the database, so they switch a WAL database back by themselves. A rollback needs no extra step for the journal mode (section 6.7).

## 3. Tables

```mermaid
erDiagram
    questions {
        TEXT id PK "Question identity (section 4.1)"
        TEXT sheet "Sheet name (indexed: sheet, active)"
        INTEGER row_no "Row number in the Excel file (display only)"
        INTEGER seq "Order in the question bank"
        TEXT data "JSON of the whole question (section 4.7)"
        INTEGER active "1 active · 0 retired"
        TEXT added_at "When it first entered the question bank"
        TEXT updated_at "Time of the latest import or retirement"
        INTEGER bank_id "banks.id (v2)"
    }
    banks {
        INTEGER id PK "Auto-increment, never reused (v2)"
        TEXT name "Shown name: the file name without .xlsx"
        TEXT filename "File name of the latest import into this bank"
        INTEGER legacy_ids "1 = the legacy bank (unscoped question ids)"
        TEXT created_at "When the bank was created"
        TEXT updated_at "Time of the latest import into this bank"
        INTEGER owner_id "users.id of the user who imported it as a new bank (v4)"
    }
    imports {
        INTEGER id PK "Auto-increment"
        TEXT at "Import time"
        TEXT filename "Uploaded file name"
        INTEGER added "Added"
        INTEGER updated "Updated"
        INTEGER removed "Retired"
        INTEGER restored "Restored"
        INTEGER skipped "Unreadable"
        INTEGER total "Active questions after the import"
        INTEGER bank_id "banks.id (v2)"
    }
    users {
        INTEGER id PK "Auto-increment, never reused (v3)"
        TEXT username "Unique, ignoring case"
        TEXT password_hash "Argon2id, PHC string"
        TEXT role "admin · user"
        INTEGER disabled "1 = cannot sign in"
        TEXT created_at "Local time"
        TEXT last_login_at "Local time, nullable"
        TEXT password_changed_at "Local time"
        INTEGER must_change_password "1 = must choose a new password first"
        INTEGER new_banks "1 = gets every bank created later (v4)"
        INTEGER can_manage_banks "1 = bank manager (v4)"
        INTEGER ai_enabled "1 = AI+ on (v5)"
        TEXT uid "identity stamp, 32 hex, never changes (v5)"
        INTEGER can_download_banks "1 = may download the banks they can see (v10)"
    }
    sessions {
        TEXT token_hash PK "SHA-256 of the cookie value (v3)"
        INTEGER user_id "users.id (indexed)"
        TEXT created_at "Local time"
        INTEGER last_seen_at "Unix seconds"
        INTEGER expires_at "Unix seconds"
    }
    user_banks {
        INTEGER user_id PK "users.id (v4)"
        INTEGER bank_id PK "banks.id (indexed)"
    }
    attempts {
        INTEGER id PK "Auto-increment"
        TEXT at "Submission time"
        TEXT sheets "Sheets taken (JSON array)"
        TEXT mode "all · weak · retry"
        INTEGER total "Number of questions"
        INTEGER correct "Number of correct answers"
        INTEGER percent "Percentage (round half to even)"
        INTEGER seconds "Time taken (seconds)"
        TEXT banks "JSON snapshot of the banks (v2)"
        INTEGER user_id "users.id (v3, indexed)"
        TEXT kind "exam · practice (v4)"
        TEXT session_key "32 hex of the practice / exam session, NULL for older records (v6)"
        INTEGER started_at "Session start, unix seconds, NULL like session_key (v6)"
    }
    attempt_items {
        INTEGER attempt_id PK "attempts.id (v2)"
        INTEGER seq PK "Position in the exam"
        TEXT question_id "Indexed"
        TEXT answer "JSON array as submitted, NULL = unanswered"
        INTEGER correct "1 · 0"
    }
    user_stats {
        INTEGER user_id PK "users.id (v3)"
        TEXT question_id PK "questions.id (no constraint)"
        INTEGER attempts "Times answered"
        INTEGER correct "Times answered correctly"
        INTEGER wrong "Times answered incorrectly"
        INTEGER streak "Current run of correct answers"
        INTEGER last "Result of the previous answer, nullable"
        TEXT at "Time of the latest answer, nullable"
    }
    user_settings {
        INTEGER user_id PK "users.id (v3)"
        TEXT key PK "theme · uiLang · autoAdvance · questionLang"
        TEXT value "JSON text of the value"
    }
    meta {
        TEXT key PK "schema · legacy_migrated · users_known · banks_known"
        TEXT value
    }
    banks ||--o{ questions : "questions.bank_id"
    banks ||--o{ imports : "imports.bank_id"
    users |o--o{ banks : "banks.owner_id"
    users ||--o{ user_banks : "user_banks.user_id"
    banks ||--o{ user_banks : "user_banks.bank_id"
    users ||--o{ sessions : "sessions.user_id"
    users ||--o{ attempts : "attempts.user_id"
    attempts ||--o{ attempt_items : "attempt_id"
    users ||--o{ user_stats : "user_stats.user_id"
    users ||--o{ user_settings : "user_settings.user_id"
```

Thirteen tables, still **without foreign key constraints**: the links are kept by the code, and every delete removes the rows that belong to it in the same transaction. Two tables of earlier schemas are not shown: `stats` (the per-question statistics of v0.5 and older) and `settings` (the settings of v0.5). At the v3 migration their rows were copied to the first administrator (`user_stats`, `user_settings`); they are kept, unused, so that a rollback stays "restore the backup".

What each schema added:

- **v1** (the old Python version, kept verbatim): `meta`, `questions`, `attempts`, `stats`, `imports`.
- **v2** (v0.5): the tables `banks`, `attempt_items` and `settings`; the columns `questions.bank_id INTEGER` (NULL only for rows written by an older binary, see 6.6), `attempts.banks TEXT NOT NULL DEFAULT '[]'` (JSON snapshot of the banks an exam drew from, `[{"id": 1, "name": "…"}]`) and `imports.bank_id INTEGER`; the indexes `idx_questions_bank (bank_id, active)` and `idx_attempt_items_question (question_id)`.
- **v3** (v0.6): the tables `users`, `sessions`, `user_stats` and `user_settings`; the column `attempts.user_id INTEGER`; the indexes `idx_sessions_user` and `idx_attempts_user`.
- **v5** (v0.8): the columns `users.ai_enabled INTEGER NOT NULL DEFAULT 0` (the per-user **AI+ feature** grant: the user may use AI Chat while the AI+ feature is on and available; stored for administrators too, but ignored for them, who always have it) and `users.uid TEXT NOT NULL DEFAULT ''` (a random identity stamp of 32 lower-case hex digits, set when the account is created and never changed; the AI service stores it next to the user id and drops AI data whose stamp differs, section 8.3). Nothing else: AI settings, chats and memories live in the AI database.
- **v4** (v0.7): the table `user_banks`; the columns `users.new_banks INTEGER NOT NULL DEFAULT 0`, `users.can_manage_banks INTEGER NOT NULL DEFAULT 0`, `banks.owner_id INTEGER` and `attempts.kind TEXT NOT NULL DEFAULT 'exam' CHECK (kind IN ('exam', 'practice'))`; the indexes `idx_user_banks_bank (bank_id)` and `idx_banks_owner (owner_id)`; the `meta` keys `users_known` and `banks_known` (section 6.6). The six-column workbook layout of v0.7 needed no column: its per-language texts live in `questions.data` (section 4.7). Pre-release builds of v0.7 had `users.all_banks` instead of `users.new_banks`; such a database is converted when it is opened (section 6.5).
- **v6** (v0.9): the columns `attempts.session_key TEXT` (the random key of the practice or exam session the record was saved from: 32 lower-case hex digits; the AI service names the session's chat by it, section 8.2) and `attempts.started_at INTEGER` (when that session started, unix seconds on the browser's clock). Both are NULL for every record saved before v0.9, and for a save in which the browser sent no valid value: each of the two is judged on its own, an invalid one is stored as NULL, and the save is never refused for it. There is no index and no constraint. The columns are never used to look a record up (a record is always found by `attempts.id`), and a key is not guaranteed to be unique: nothing relies on that. Nothing else changed in the main database: the AI chats, their texts and their titles live in the AI database.

The v1 table-creation statements are stored verbatim in `backend/src/database/schema.rs` (`SCHEMA`), identical to `SCHEMA` in the old version's `app/db.py`, and are still executed first on every start; the later additions are next to them (`MIGRATE_V2_TABLES`, `V2_COLUMNS`, … `MIGRATE_V4_INDEXES`, `V5_COLUMNS`, `V6_COLUMNS`). Everything uses `IF NOT EXISTS` or is checked first, so opening an up-to-date database **writes nothing** (tested: the database file together with its `-wal` file stays byte for byte the same). Because SQLite records `ALTER TABLE … ADD COLUMN` by appending the column to the stored `CREATE TABLE` text, the `sqlite_master` text of `questions`, `attempts` and `imports` is the v1 text plus the added columns (and that of `users` the v3 text plus the v4 and v5 columns); a new database and a migrated one have exactly the same schema (tested in `db_golden.rs`).

## 4. Details that decide the fate of your study records

### 4.1 Question id
The statistics (`user_stats`) and the wrong-answer notebook are both keyed by question id. The id is:

```
sha1( sheet_name + "\n" + strip_whitespace(question_stem)[:300] + "|" + strip_whitespace(first_option_or_first_sub_item)[:200] )  first 12 hex digits
```

- The stem and the first option are those of the question's **base text**: the English question (column A) in the six-column layout, the only one that can be imported since v0.7. The three- and four-column layouts of earlier versions used the Japanese question; their questions are retired (section 4.2), and updating such a bank with a six-column file gives every question a new id: they are added as new questions, without statistics, and the old ones stay retired.
- "Strip whitespace" means Python's `\s` (29 code points, including U+001C–U+001F and the full-width space U+3000). **Whitespace is stripped first, then the text is truncated**, and truncation is by **code point**. Rust's `char::is_whitespace` covers only 25, so the backend implements its own check for these 29 code points.
- **No Unicode normalization at all** (no NFKC). Many real question banks contain full-width ASCII, and normalization would change every id.
- Duplicate ids within the same workbook get `-2`, `-3`, ... appended in turn, **depending on row order**: if you swap the rows of two identical questions, the study records move to the other question. This is the old version's behavior; it is kept and documented here.
- Changing the wording of the question stem = a new question (the old one is retired, and its records do not carry over to the new one); changing only the options, the answer, the explanation or the translations is an "update", and the records are not affected.
- 85 assertions (`tests/baseline/golden/id_vectors.json`) guarantee that the ids from Rust and Python are byte-for-byte identical.
- **Banks (v0.5)**: the id above is what the questions of the **legacy bank** (all data from before v0.5, and uploads without `X-Bank-Target`) are stored under, so existing statistics, the wrong-answer notebook and history stay linked. Every other bank stores **bank-scoped ids**: `sha1("xexam-bank:" + bank_id + "\n" + id)`, first 12 hex digits. That is why the same workbook can be imported as two independent banks without their records mixing. Bank ids are never reused (`AUTOINCREMENT`), so neither are scoped ids.

### 4.2 Retire instead of delete
During an import, a question of the **target bank** that cannot be found in the file gets `active = 0`, and its statistics are kept as they are. If you upload that question into the same bank again later, it is restored to active automatically, and its wrong-answer notebook entry comes back with it. Questions of other banks are never touched by an import.

**Old layouts (v0.7).** Every start of the backend, and of the command-line commands that open the database, retires every active question that is not of the six-column layout (its `data` has no `"lang": "en"`, section 4.7), after the migrations and fix-ups of section 6: `active = 0`, `updated_at` = now, and a `seq` after the bank's other questions (their order kept), exactly as an import retires a question that left its file. Banks, the import history, `user_stats`, `attempts` and `attempt_items` stay; a bank left without active questions stays too, so it can be updated with a six-column file or deleted. It runs in one transaction and logs one warning per bank (`retired 12 questions of bank "Kitchen basics": they use the old 3- / 4-column layout; import the six-column version of the workbook`); when there is nothing to retire, nothing is written. Such questions are never restored by an import, because a three- or four-column file cannot be imported.

Deleting a bank (`POST /api/banks/delete`) is the only way questions leave the database. In one transaction it deletes the bank's questions (active and retired), its `imports` rows, its `user_banks` rows (who may see it) and its `banks` row. With `withRecords` the study data of those questions goes too: their `user_stats` and `attempt_items` rows, and every `attempts` row whose `banks` snapshot names that bank and no bank that still exists (with its remaining detail). An administrator deletes the study data of **every** user this way; a bank manager deleting their own bank deletes only **their own** study data of it, and other users' records stay (their questions then show as deleted). Without `withRecords`, `user_stats`, `attempts` and `attempt_items` stay as they are.

### 4.3 Wrong-answer notebook
Each user has their own notebook in `user_stats`. A question is in it when `wrong > 0` and `streak < 2`; it is moved out after 2 consecutive correct answers. Exams and practice sessions update it in the same way. A user limited to some banks (section 4.8) sees the statistics of those banks' questions only.

### 4.4 Study records
Each user's study records are rows of `attempts` with their `user_id`; `kind` says whether a record is an `exam` or a `practice` session (every record from before v0.7 is an exam, the column default). Each user keeps their latest **500** records (`RECORD_RETENTION`; it was 60 before v0.5); after each submission that user's older ones are deleted together with their `attempt_items`. `GET /api/progress` returns only the latest 60 in `history`.

- Since v0.5 each new record stores one `attempt_items` row per question (the answer given, if the page sent it, and whether it was correct) and the `banks` snapshot; records saved earlier have no detail.
- Since v0.9 a record also keeps the key and the start time of the practice or exam session it was saved from (`session_key`, `started_at`; NULL for older records). `POST /api/attempt` answers with the new record's id (`recordId`, always the last key), and `GET /api/record` returns the two values as `sessionKey` and `startedAt` (`GET /api/records`, the list, is unchanged). They exist only to tie a record to the AI chat of its session (section 8.2); nothing else reads them. The v0.9 AI routes read them, and the record's questions and answers, through the backend's token routes, never from this file (section 5 of [architecture.md](architecture.md)).
- Only questions that still exist, and that the user may see, are counted (v0.7): a result for a question whose bank was deleted during the exam, or that belongs to a bank outside the user's scope, writes no statistics and no detail row, and is left out of `total`, `correct` and `percent`. `sheets` then lists only the sheets that a counted question belongs to. When no question is left, nothing is saved.
- Deleting one record (`POST /api/records/delete`) removes it and its detail but leaves the statistics alone; deleting all of them can also empty the user's statistics (`withStats`), which equals `POST /api/reset`.

### 4.5 Settings
`user_settings` holds one row per user and changed setting; `value` is the JSON text of the value (`"dark"`, `true`). Missing rows mean the default (`theme` `system`, `uiLang` `en`, `autoAdvance` false, `questionLang` `ja`); a stored value that is not valid is ignored. The browser keeps a copy only to paint the first frame; the database is the source of truth, so settings survive container updates, a cleared browser cache and other browsers.

### 4.6 Timestamps
Local-time strings in the format `%Y-%m-%d %H:%M`, **with no time zone marker**, accurate to the minute (the `sessions` table also keeps two Unix times). Inside the container the time zone is the host's by default (`docker/docker-compose.yml` mounts the host's `/etc/localtime`, read only); a `TZ` set in `docker/.env` overrides it, for example `TZ=UTC`, and without either the image falls back to UTC. On Docker Desktop (Windows, macOS) the mounted file is the one of Docker's own virtual machine, usually UTC, so set `TZ` there. The image ships a time zone database, so `TZ` can name any zone. Changing the zone later does not rewrite stored times: they keep the zone they were written in.

### 4.7 The question JSON (`questions.data`)
`data` is the whole question as JSON, without `id`, `sheet` and `row` (they have their own columns). Its main keys: `kind` (`single`, `multi`, `hotspot`), `stem`, `options`, `items`, `answer`, `translation`, `zh` (the Chinese text, split along the base question), `explanation`, `mnemonic` and `note`. A 4-column question also has `english` and `en` (the English text).

A **six-column** question (v0.7) has three more keys, and no `english` / `en`, because its base text is English already:

- `"lang": "en"`: the language of the base text (`stem`, `options`, `items`). Questions of the 3- and 4-column layouts have no `lang` key: their base text is Japanese.
- `"ja"`: the Japanese question (column C), split along the English one, same shape as `zh`. Missing when column C does not line up with the English question (the exam page then greys out the **日本語** button).
- `"explain"`: the explanation, memory aid and note of each language, `{"en": {"explanation", "mnemonic", "note"}, "zh": {…}, "ja": {…}}`. A language whose answer column has none of the three is left out.

For clients that know only one explanation, `explanation`, `mnemonic` and `note` of a six-column question hold the Chinese text when column E has one, else the Japanese (F), else the English (D). Questions of the 3- and 4-column layouts, which stay in the database as retired questions, keep exactly the JSON of earlier versions (no new keys); the parser still reads those layouts (that is how an import tells them apart and refuses them), and the parser golden tests check its output byte for byte.

### 4.8 Who sees which bank (v0.7)
- **Administrators** see and manage every bank.
- **Plain users** see exactly the banks listed for them in `user_banks` (the existing banks an administrator ticked). `users.new_banks = 1` (**Include banks imported later**) only adds rows: whenever a bank is created (an import as a new bank, by anyone, or the command-line `import` creating the legacy bank), every user with the flag gets a row for it in the same transaction. It never adds an existing bank, and unticking a bank removes only that row. A new user starts with no row and the flag off. Every user that existed at the v4 migration got a row for every existing bank and the flag, so nobody lost a bank at the upgrade.
- **Bank managers** (`users.can_manage_banks = 1`) also see the banks they own (`banks.owner_id`) while they have the permission, and may update and delete only those. A bank imported as a new bank belongs to the user who imported it; banks from before v0.7, and banks created without a signed-in user (the command-line `import`), belong to the first administrator.
- Deleting a user deletes their sessions, study records (with their detail), statistics, settings and `user_banks` rows; the banks they own move to the administrator who deletes them. Their AI chats and memories are in the AI database: the AI service removes them at its next reconcile (section 8.3).

## 5. Backup and restore

Since v0.8 there are **two data volumes** to back up: `xexam_backend_data` (this database) and `xexam_ai_data` (the AI database and its key file, section 8.4). `xexam_ai_auth` needs no backup. The commands below are for `xexam_backend_data`; the AI volume works the same way.

**Stop the backend before you copy the database.** In WAL mode the latest changes can sit in `xexam.db-wal` while the backend runs, so a copy of `xexam.db` alone can miss them; after a normal stop everything is in `xexam.db`, and a `-wal` file left next to it is empty.

```bash
docker compose stop

# Back up: pack the volume into xexam-data.tgz in the current directory (it holds the whole database; it is ignored by .gitignore, do not commit it)
docker run --rm -v xexam_backend_data:/data -v "$PWD:/backup" alpine tar czf /backup/xexam-data.tgz -C /data .

docker compose start
```

If you cannot stop it, copy `xexam.db`, `xexam.db-wal` and `xexam.db-shm` together, in one go, and check the copy as below; only all three files together hold every committed change. A copy taken while a write is in progress can still be inconsistent, so a copy of the stopped volume is the one to rely on. A consistent copy of a running database can also be made with SQLite itself: `VACUUM INTO`, or `.backup` in the `sqlite3` command-line tool.

**Verify the backup** (on a copy; never experiment on the original):

```bash
mkdir work && tar xzf xexam-data.tgz -C work
sqlite3 work/xexam.db "PRAGMA integrity_check"      # must print ok
```

**Restore.** Remove any leftover `-wal` and `-shm` files before you put the backup in place: they belong to the database you are replacing, and SQLite must not apply them to the restored one. After a normal stop they are empty or gone, so the `rm` removes nothing that matters.

```bash
docker compose stop
docker run --rm -v xexam_backend_data:/data -v "$PWD:/backup" alpine sh -c 'rm -f /data/xexam.db-wal /data/xexam.db-shm && tar xzf /backup/xexam-data.tgz -C /data'
docker compose start
```

The files in `backups/` are part of the volume, so a volume backup includes them.

**Moving to another computer**: put the new `docker-compose.yml` in place on the new machine, run `docker compose up -d` once to create the volumes, then restore the backups there as above (both data volumes). The backup files hold your whole database (password hashes included) and the AI keys with the key that decrypts them, so keep them private and do not commit them (`.gitignore` ignores `*.tgz`).

## 6. Schema migrations and going back

`meta.schema` says which schema a database has (`1` up to v0.4, `2` in v0.5, `3` in v0.6, `4` in v0.7 and v0.7.5, `5` in v0.8, `6` in v0.9 and v1.0, up to `10` since v1.6: sections 6.12 to 6.16). When a version opens a database with an older schema, it brings it up to its own schema in one go:

1. **Backup.** If the database contains any data, a consistent copy is written with SQLite's `VACUUM INTO` to `backups/xexam-schema<N>-<YYYYmmdd-HHMMSS>.db` next to `xexam.db`, where `<N>` is the schema the file had before this start (an existing file is never overwritten; `-2`, `-3`, … are appended instead). There is always **one** backup, also when several steps follow. The copy is one self-contained file in the old schema; changes that were still in the `-wal` file are in it too. `VACUUM INTO` writes a complete file, so **keep free disk space of about the size of the database** before you upgrade; the copy is never pruned, and it keeps what you delete later (records, banks, users), so remove old copies yourself when that matters. **If the copy cannot be written** (disk full, no permission), the program refuses to start and changes nothing; the log says why.
2. **Migration**, in **one** `BEGIN IMMEDIATE` transaction: every step from the old schema to the current one (6.1, 6.4, 6.5, 6.8, 6.10, and the conversion of a pre-release v4 database), then `meta.schema` is set.
3. **Journal mode.** Only after that does the backend switch the file to WAL (section 2.1).

The log names the copy: `Upgraded the database to schema 6. A copy of the previous version was saved first: /data/backups/xexam-schema5-….db (restore it if you ever go back to an older version).` A brand-new database goes through the same steps (without a backup, and without a legacy bank), so new and migrated databases have the same structure.

### 6.1 Schema v2 (v0.5)

When v0.5 or later opens a database whose `meta.schema` is missing or `1`, the backup is named `xexam-schema1-<YYYYmmdd-HHMMSS>.db` (an ordinary schema-1 database). The v2 step creates `banks`, `attempt_items`, `settings` and the new indexes, adds the three columns, then, if there are questions, creates the **legacy bank** and attaches everything to it: `name` = file name of the latest import without `.xlsx` (`Question bank` if there is no import), `filename` = that file name, `created_at` = the earliest `questions.added_at`, `updated_at` = the time of the latest import (or of the latest question change if there is no import); every question and import row gets its `bank_id`, and every exam record gets that bank as its `banks` snapshot. Question ids, statistics, exam records and import history are not changed.

### 6.2 Going back to v0.4 (or older)

A v2 file can still be opened by v0.4 and by the old Python version: the new columns are nullable or have defaults, and `user_version` is still 0. **But do not keep using an older version once you have more than one question bank**: it treats the whole database as one bank, so its next import would retire the questions of every other bank, and it ignores the new tables. It also prunes exam records back to 60. A database of schema v3 or later also has users that v0.4 knows nothing about.

**To roll back, restore the backup instead**: stop the program, replace `xexam.db` with the `xexam-schema1-…` file from `backups/` (rename it to `xexam.db`, and remove any `xexam.db-wal` / `xexam.db-shm` next to it), start the old version. Changes made with the newer version after the migration are not in that copy.

If an older version did write to the file anyway, the next start of v0.5 or later attaches the questions it added to the legacy bank (6.6). Exam records it saved have no detail and an empty bank snapshot; detail rows of exams it pruned remain in `attempt_items` as unused rows (they are never shown).

### 6.3 Compatibility with the old Python implementation

Up to v0.4 both implementations used **the same file and the same schema**. This round trip was originally rehearsed with synthetic data (old Python creates the database → Rust reads and writes → old Python reads it back; the original's sha256 was unchanged before and after, and `PRAGMA integrity_check` passed). Since the old Python code was removed, tests keep the remaining guarantees in place: `sample.db` (a database written by the old implementation) migrates to the current schema keeping every question id, statistic, exam record and import, its backup still reports schema 1 with exactly the original rows, and the migrated file is byte-for-byte unchanged by any later open, read and close; and the `sqlite_master` of a new database is the old implementation's text plus the later additions. To actually roll back to the Python version, restore the schema-1 backup (6.2) and open it with the old image (`ghcr.io/yumin-space/xexam:v0.2.2`, only on the private GHCR, so `docker login ghcr.io` first).

### 6.4 Schema v3 (v0.6)

From `meta.schema` `2` (backup `xexam-schema2-<YYYYmmdd-HHMMSS>.db`), or from `1` in the same transaction as 6.1. The v3 step creates `users`, `sessions`, `user_stats` and `user_settings` and adds `attempts.user_id`. If there are no users yet, it creates the default administrator `admin` with the password `admin` (which must be changed at the first sign-in) and gives it the existing data: every exam record without a user, a copy of every `stats` row in `user_stats` and of every `settings` row in `user_settings`. The old `stats` and `settings` tables stay, unused.

### 6.5 Schema v4 (v0.7)

From `meta.schema` `3` (backup `xexam-schema3-<YYYYmmdd-HHMMSS>.db`), or from `1` or `2` in the same transaction as the steps before. The v4 step:

- creates `user_banks` and its index, and adds `users.new_banks`, `users.can_manage_banks`, `banks.owner_id` and `attempts.kind`;
- gives **every user that exists** a `user_banks` row for every existing bank and `new_banks = 1`, so everyone keeps seeing every bank, also the ones imported later;
- leaves `can_manage_banks = 0` for everyone (administrators manage banks anyway);
- gives every bank to the **first administrator**: the enabled administrator with the lowest id (a disabled one only when no administrator is enabled);
- needs no backfill for `attempts.kind`: its default `exam` is what every older record is;
- writes `meta.users_known` and `meta.banks_known` (see 6.6); since v0.8 the v5 step follows in the same transaction (6.8), which sets `meta.schema`.

After it, the start continues with the checks of 6.6, the switch to WAL (section 2.1) and the retirement of the old-layout questions (section 4.2): a database of v0.6 or older holds only questions of the three- and four-column layouts, so all of them are retired.

**Pre-release v4.** Builds of v0.7 made before its release stored `users.all_banks` (1 = every bank, including later ones) instead of `users.new_banks`. A database stamped `4` that still has that column is converted when it is opened, after a backup `backups/xexam-schema4-<YYYYmmdd-HHMMSS>.db`, in one transaction (since v0.8 the same transaction as the v5 step, with one backup): a user with `all_banks = 1` gets a row for every existing bank and `new_banks = 1`; a user with `all_banks = 0` keeps their rows, with `new_banks = 0`; then the column `all_banks` is dropped and `meta.banks_known` is written. `meta.users_known` is left as the pre-release build wrote it, so a user that an older version created in between (above that mark) gets every bank and `new_banks = 1` from the check of 6.6 that runs next, like the users of the upgrade. The log says `Converted the question bank access of a database written by a v0.7 pre-release to the released format and upgraded the database to schema 5. A copy of the previous version was saved first: <path>.`

### 6.6 Checks on every start

On every start, after any migration, the backend checks (read-only) for rows that only an older binary can have left behind after a rollback, and fixes them in one transaction each. When there is nothing to fix, **nothing is written**. Older versions know neither `user_banks` nor `users.new_banks`, and every user saw every bank there.

- **Banks created by an older version**: `meta.banks_known` holds the highest bank id this version knows; it is written by the v4 migration and whenever this version creates a bank (bank ids are never reused). Every user with `new_banks = 1` gets a row for each bank above it, as if this version had created the bank; then the mark moves. This check runs first, so a bank that the next check creates cannot hide them.
- **Questions without a bank** (`bank_id` NULL), and import rows without a bank while there is a legacy bank: attached to the legacy bank, which is created if needed.
- **No user at all**: the default administrator `admin` / `admin` is created again. **Exam records without a user**: given to the first administrator.
- **Banks without a valid owner** (NULL, or a user that no longer exists): given to the first administrator.
- **`user_banks` rows** of a user or a bank that no longer exists: deleted.
- **Users created by an older version**: `meta.users_known` holds the highest user id this version knows; it is written by the v4 migration and whenever this version creates a user (user ids are never reused). A user above it was created by an older version after a rollback, without any `user_banks` row; such users get a row for every existing bank and `new_banks = 1`, like the users of the upgrade, and the mark moves.
- Without a mark (a database first opened by a build that did not write it), such banks and users cannot be told apart, so only the mark is written.
- **Users without an identity stamp** (`uid = ''`, v0.8): a user that v0.7.5 created after a rollback has no stamp; it gets one. Its AI+ is off (the column default).

Then the questions of the old layouts are retired (section 4.2).

### 6.7 Going back to v0.6

**Restore the backup**: stop the backend, replace `xexam.db` with `backups/xexam-schema3-….db` (rename it to `xexam.db`, and remove any `xexam.db-wal` / `xexam.db-shm` next to it), then start v0.6. Everything done with v0.7 or later after the migration is not in that copy. The exact commands are in section 2.6 of [migration.md](migration.md).

v0.6 can also open the schema-4 file itself: it ignores the new table and columns and switches the file back to the `DELETE` journal. Keep that for emergencies, because v0.6 does not know the rules of v0.7: every user sees every bank again, only administrators can import or delete banks, practice records look like exams, and it knows nothing of six-column questions. When you start the new version again afterwards, 6.6 repairs what v0.6 left: banks it imported get an owner and reach the users with `new_banks = 1`, users it created get every bank and `new_banks = 1`, and the questions it imported are retired (section 4.2).

### 6.8 Schema v5 (v0.8)

From `meta.schema` `4` (backup `xexam-schema4-<YYYYmmdd-HHMMSS>.db`), or from `1`, `2` or `3` in the same transaction as the steps before (then the backup is named after that older schema). The v5 step adds `users.ai_enabled` and `users.uid`, gives every user a random stamp (`lower(hex(randomblob(16)))`, also the default administrator a v3 step may just have created) and sets `meta.schema` to `5`. Every user starts with AI+ off. The log says `Upgraded the database to schema 5. A copy of the previous version was saved first: …`.

### 6.9 Going back to v0.7.5 (or v0.7)

**Restore the backup**: stop, replace `xexam.db` with `backups/xexam-schema4-….db` (rename it to `xexam.db`, and remove any `xexam.db-wal` / `xexam.db-shm` next to it), put the old `docker-compose.yml` back and start v0.7.5. The commands are in section 1.4 of [migration.md](migration.md). The AI volume is not used by v0.7.5; keep it for a later upgrade, or remove it.

v0.7.5 can also open the schema-5 file itself (it reads schema `5` as a newer v4 and ignores the two columns); users it creates have no stamp and AI+ off, and v0.8 gives them a stamp at its next start (6.6). Keep that for emergencies too.

### 6.10 Schema v6 (v0.9)

From `meta.schema` `5` (backup `xexam-schema5-<YYYYmmdd-HHMMSS>.db`), or from an older schema in the same transaction as the steps before (then the one backup is named after that older schema). The v6 step adds `attempts.session_key` and `attempts.started_at`, leaves both NULL for every existing record and sets `meta.schema` to `6`. No row changes meaning and no index is built. The log says `Upgraded the database to schema 6. A copy of the previous version was saved first: …`. Tested in `v09_ai_study.rs` and by case J of `tests/integration/migrate_check.py` (one backup `xexam-schema5-…`, the two columns present and NULL, reopening writes nothing).

### 6.11 Going back to v0.8

The main database needs **no restore** to go back: a v0.8 backend opens a schema-6 file as it is (it treats a newer schema as current and never reads the two columns), the records it saves have NULL there, and `meta.schema` stays `6`. When v0.9 starts again later it finds schema 6 and writes no new backup. Restoring `backups/xexam-schema5-….db` still works (stop, replace `xexam.db`, remove any `xexam.db-wal` / `xexam.db-shm`), but it throws away everything done since the upgrade.

The **AI database is different**: a v0.8 ai-service refuses a schema-3 file (section 8.3), so going back to v0.8 means restoring the AI backup `backups/xexam-ai-schema2-….db` and losing every chat, memory, setting and usage row written since the upgrade. The step-by-step commands are in [migration.md](migration.md).

### 6.12 Schema v7 (v1.1)

The main database moves from schema 6 to **schema 7** once, on the first start of the v1.1 backend, after one backup copy `backups/xexam-schema6-<date-time>.db`. It adds four columns (`database/schema.rs`, `V7_COLUMNS`):

- `attempts.time_limit` (INTEGER): the limit of a timed exam in seconds (60 to 36000). NULL for untimed exams, practice and every record saved before v1.1.
- `attempts.ended` (TEXT): `submit` (submitted by hand) or `timeUp` (the countdown ended the exam; timed exams only). Set on every exam saved by v1.1; NULL for practice and older records. In a record with `ended` set, an item with no answer and `correct` 0 is a question **not completed**: it is not wrong and not in the wrong-answer notebook.
- `banks.ai_job` (TEXT): the job key of an AI-generated bank (32 lower-case hex). NULL for imported banks.
- `banks.ai_meta` (TEXT): JSON facts of an AI-generated bank (source, author, counts, time). NULL for imported banks.

New indexes cover the job key (one bank per job) and the scoped queries of v1.1. Records and banks of v1.0 are not changed.


### 6.13 Schema v8 (v1.3)

The main database moves from schema 7 to **schema 8** once, on the first start of the v1.3 backend, after one backup copy `backups/xexam-schema7-<date-time>.db`. The step is additive.

- `users`: `last_active_at` (TEXT, `YYYY-MM-DD HH:MM:SS`; set from `last_login_at` at the upgrade), and `last_login_ip`, `last_login_country`, `last_login_region`, `last_login_city` (TEXT). The last four are written at a sign-in only while the sign-in info eye is open (while it is closed they are left as they are, hidden in the answers) and set to NULL by **Clear records**.
- `sessions`: `started_at` (INTEGER, unix seconds; the upgrade time for existing sessions), `remember` (INTEGER, default 1), `ip` (TEXT, written only while the eye is open; hidden while it is closed) and `ua` (TEXT, the browser, at most 256 characters).
- `activity_log` (new): one row per activity entry: `ts`, `last_ts`, `count` (merged page visits and refused sign-ins), `actor` (`user`, `anon` or `system`), `user_id` (no foreign key: entries outlive the account), `username` and `role` (snapshots), `action`, `object`, `detail` (a small JSON object with allow-listed keys), `result` (`ok` or `fail`), `reason`, `ip`, `country`, `region`, `city` (written only while the sign-in info eye is open; hidden in the answers while it is closed), `ua`, `source` (`backend` or `ai`) and `src_id` (the AI service's event id, unique, so a repeated delivery is ignored). Indexes on time, user, action and `src_id`. Retention: 90 days by default and at most 500 000 rows, except the sign-in rows (`action = 'auth.signin'`), which are never deleted and not counted against the cap (a partial index `idx_activity_prune` on `ts` serves the retention queries).
- `system_settings` (new): `key`, `value`, `updated_at`, `updated_by`. Keys: `activity.retention_days` (90), `activity.user_visits` (0) and `users.signin_info` (0: while 0 nothing new is recorded and stored IP / location are hidden; nothing is deleted).
- Index `idx_attempts_session` on `attempts(user_id, session_key)` for the repeat save of a result.

The first log row is `system.migrate`. A v1.2 backend opens a schema-8 file as it is: the new columns have defaults, the new tables are ignored, and its sessions get their start time at the first v1.3 touch.

### 6.14 Going back to v1.2

No restore is needed (see above). Restoring the schema-7 copy is optional and loses what was written since; the sessions and the activity log are lost with it.

### 6.15 Schema v9 (v1.4)

The main database moves from schema 8 to **schema 9** once, on the first start of the v1.4 backend, after one backup copy `backups/xexam-schema8-<date-time>.db`. The step is additive and backfills nothing.

- `users`: `last_active_ip`, `last_active_country`, `last_active_region`, `last_active_city` (TEXT; the IP and place of the last activity, written together with `last_active_at`, at most once a minute, **only while the sign-in info eye is open**; set to NULL by **Clear records**; hidden in the answers while the eye is closed) and `last_active_ua` (TEXT, the cleaned User-Agent of the last activity, at most 256 characters; written also while the eye is closed and **not** cleared by **Clear records**, like `ua` of the activity log). The place columns hold a two-letter country code or a country name, a region and a city, any of which may be NULL.
- `system_settings`: a new key `geo.source` (`cloudflare`, the default when missing, or `api`): where the place of a record comes from (section 14.2 of [usage.md](usage.md)).
- Nothing else changes: the per-IP memory and the queue of the external lookup are in memory only.

The log row is `system.migrate` with `{"from": 8, "to": 9}` (one row, also when upgrading from schema 7). A v1.3 backend opens a schema-9 file as it is: the new columns are ignored.

### 6.16 Schema v10 (v1.6)

The main database moves from schema 9 to **schema 10** once, on the first start of the v1.6 backend, after one backup copy `backups/xexam-schema9-<date-time>.db`. The step adds one column and backfills nothing.

- `users.can_download_banks INTEGER NOT NULL DEFAULT 0`: `1` = a plain user may download the question banks they can see (Excel or Word, section 11.1 of [usage.md](usage.md)). Every existing user gets `0`. It is stored like `can_manage_banks`: kept across role changes, set in the bank dialog of **User management**, read from the database on every request; **administrators may always download**, whatever is stored. The downloaded file is built from the questions of `questions.data` and is never stored.

The log row is `system.migrate` with `{"from": 9, "to": 10}` (one row, also when upgrading from an older schema, only when the file held data). A v1.5 backend opens a schema-10 file as it is: the new column is ignored (its own inserts get the default `0`).

## 7. The old `progress.json`

Earlier versions kept scores in `progress.json`. As in the old Python version, **on every startup** the app checks whether the data directory contains it: if it does, and the database has no study records yet and has not been migrated before, it is migrated into the database (into the records of the first administrator). The `legacy_migrated` flag in `meta` guarantees it is migrated only once, and existing study records are never overwritten.
So "start once to create the data volume → put `progress.json` into the volume → restart" works. A file in the wrong format is ignored; the log states the reason and suggests running `xexam-backend migrate-progress` by hand, and the program still starts.
(`tests/integration/migrate_check.py` checks these cases with synthetic data, in WAL mode and with `XEXAM_SQLITE_JOURNAL=delete`.)

In Docker, copy the file into the volume with a temporary container (the commands work in bash and in PowerShell):

```bash
docker compose stop
docker run --rm -v xexam_backend_data:/data -v "${PWD}:/src:ro" alpine sh -c "cp /src/progress.json /data/progress.json && chown 10001:10001 /data/progress.json"
docker compose start
```

`docker cp` is not used here: it writes as root by default, and the backend in the container runs as uid 10001 and might not be able to read the file. After the restart the log says `Migrated the old study records into the database` (`docker compose logs backend`).

> ⚠️ When upgrading from v0.1: the database file name changed, and the new version only reads `xexam.db`. If the data directory has another `.db` file but no `xexam.db`, the startup log will warn you; rename that file to `xexam.db` and start again (the old file is neither deleted nor read).

## 8. The AI database (`xexam-ai.db`, v0.8; schema 6 since v1.1)

ai-service keeps everything of AI in its **own** SQLite database, which only it opens and migrates; the backend never reads it, and ai-service never opens `xexam.db` (the data boundary, section 5 of [architecture.md](architecture.md)).

| How you run it | Location |
|---|---|
| Docker (recommended) | Named volume `xexam_ai_data`, mounted at `/data` in the `ai-service` container (directory mode 0700): `/data/xexam-ai.db`, its `-wal` / `-shm` files while it runs, and the key file `/data/secret.key` |
| Without Docker | The directory `XEXAM_AI_DATA` (default `./data-ai`) |

### 8.1 Connection settings

Like the main database: `busy_timeout` 5000 ms, `foreign_keys=ON` (here the schema does use foreign keys, see below), `synchronous=FULL`, WAL mode (`XEXAM_AI_SQLITE_JOURNAL=delete` keeps the rollback journal, with the same rules as 2.1), and in addition **`secure_delete=ON`**. One connection behind a mutex, every write in `BEGIN IMMEDIATE … COMMIT`, the lock never held across an `.await`. The start-up log says `AI database: /data/xexam-ai.db (journal: wal)`.

### 8.2 Tables (schema v4)

```mermaid
erDiagram
    settings {
        INTEGER id PK "always 1"
        TEXT provider "openai · anthropic · gateway"
        TEXT base_url "default https://api.openai.com/v1"
        TEXT format "openai · anthropic (gateway only)"
        INTEGER responses "1 = OpenAI Responses (format stays openai)"
        TEXT gateway_auth "bearer · x-api-key · cf-aig-authorization"
        TEXT model "'' = none"
        INTEGER manual_model "1 = typed by hand"
        INTEGER usage_option "OpenAI format: 1 = ask for token usage"
        INTEGER max_tokens "Anthropic format: 4096 · 8192"
        INTEGER enabled "the AI+ feature switch"
        INTEGER verified_at "unix seconds of the last passed test"
        TEXT verified_fp "settings fingerprint of that test"
        INTEGER retention_days "1-3650, default 90"
        INTEGER updated_at "unix seconds"
        TEXT updated_by "username of the administrator"
    }
    secrets {
        TEXT name PK "api_key · gateway_key"
        TEXT key_id "id of the encryption key (16 hex)"
        BLOB nonce "24 bytes"
        BLOB ciphertext "XChaCha20-Poly1305 with tag"
        TEXT hint "last 4 characters, '' for short keys"
        INTEGER rev "counts every change"
        INTEGER updated_at "unix seconds"
    }
    limits {
        INTEGER id PK "always 1"
        INTEGER per_user_running "1-4, default 1"
        INTEGER global_running "1-64, default 16"
        INTEGER chats_per_user "10-100000, default 1000"
        INTEGER messages_per_chat "20-5000, default 400"
        INTEGER memories_per_user "1-1000, default 50"
        INTEGER memory_tokens "50-250000, default 1000 (v1.1; before v1.1 the limit was 200 characters: a value still at 200 becomes 1000, any other value is multiplied by 1.5 once, at the upgrade to schema 6; memory_chars stays unused)"
        INTEGER message_chars "unused since v1.0 (was 1000-32000, default 16000); kept, never read or written"
        INTEGER calls_per_minute "1-100, default 5 (v1.0)"
        INTEGER calls_per_hour "1-2000, default 20 (v1.0)"
        INTEGER calls_per_day "1-20000, default 100 (v1.0)"
        INTEGER reply_max_tokens "1-250000, default 3000: the answer length in tokens (v1.1; reply_max_chars, 1000 characters in v1.0, stays unused)"
        INTEGER updated_at "unix seconds"
        TEXT updated_by "username"
    }
    users {
        INTEGER id PK "users.id of the main database"
        TEXT stamp "users.uid of the main database"
        TEXT username "last known"
        INTEGER first_seen_at "unix seconds"
        INTEGER last_seen_at "unix seconds"
        INTEGER function_enabled "1 = active, 0 = paused (v2)"
        INTEGER free_mode "1 = the administrator's free mode is on (v2)"
    }
    chats {
        INTEGER id PK "auto-increment"
        INTEGER user_id FK "users.id, ON DELETE CASCADE"
        TEXT title "start of the first message, max 60 (a study chat: its start time)"
        INTEGER created_at "unix seconds"
        INTEGER updated_at "last message; retention counts from here"
        TEXT kind "chat · practice · exam · summary (v3)"
        TEXT ref "the session key, or rID@time for a record (NULL for chat) (v3)"
        INTEGER record_id "the main-app record, display only (v3)"
    }
    messages {
        INTEGER id PK "auto-increment"
        INTEGER chat_id FK "chats.id, ON DELETE CASCADE"
        TEXT role "user · assistant"
        TEXT content "the text"
        TEXT status "streaming · done · stopped · error"
        TEXT error_code "an ai.* code when status = error"
        INTEGER truncated "1 = the model's length limit"
        TEXT model "assistant messages"
        INTEGER created_at "unix seconds"
        INTEGER finished_at "unix seconds"
        INTEGER free "1 = written in free mode (v2)"
        TEXT kind "chat · hint · explain · summary (v3)"
        TEXT question_id "hint / explain, else NULL (v3)"
        INTEGER question_no "the number the user saw (v3)"
        TEXT source "practice · result (v3)"
        TEXT lang "reply language: en · zh-CN · ja (v3)"
        INTEGER sample_used "summary: wrong answers sent (v3)"
        INTEGER sample_usable "summary: wrong answers still available (v3)"
        INTEGER sample_wrong "summary: wrong answers in the record (v3)"
        TEXT reasoning "AI Chat answers: the model's thinking, trimmed, at most 32 KiB, NULL = none (v7)"
        INTEGER reasoning_ms "ms from the first reasoning to the first answer text, NULL = none (v7)"
        INTEGER reasoning_truncated "1 = cut at the cap (v7)"
    }
    memories {
        INTEGER id PK "auto-increment"
        INTEGER user_id FK "users.id, ON DELETE CASCADE"
        TEXT content "1-250000 estimated tokens (limit: memoryTokens); the column holds up to 2000000 characters since v1.1 (the table is rebuilt once)"
        TEXT source "manual · chat"
        INTEGER created_at "unix seconds"
        INTEGER updated_at "unix seconds"
    }
    tasks {
        INTEGER id PK "structure only in v0.8"
        INTEGER user_id FK "users.id, ON DELETE CASCADE; NULL = instance"
        TEXT kind
        TEXT state "queued · running · done · failed · cancelled"
    }
    usage {
        INTEGER id PK "auto-increment"
        INTEGER user_id FK "users.id, ON DELETE SET NULL"
        INTEGER at "unix seconds"
        TEXT day "YYYY-MM-DD, local"
        TEXT kind "chat · verify · models · memory"
        TEXT provider "openai · anthropic · gateway"
        TEXT model "'' for model lists"
        INTEGER tokens_in "NULL = not reported"
        INTEGER tokens_out "NULL = not reported"
        TEXT purpose "hint · explain · summary, NULL = AI Chat or a call without a chat (v3)"
        INTEGER tokens_cached "input tokens served from the prompt cache, NULL = not reported (v2)"
        INTEGER latency_ms "request to last byte"
        INTEGER first_byte_ms "request to first text (chat)"
        TEXT status "ok · stopped · error"
        TEXT error_code "ai.up.* code without the prefix"
    }
    audit_outbox {
        INTEGER id PK "auto-increment"
        INTEGER at "unix seconds"
        TEXT message "an audit line, no content"
    }
    meta {
        TEXT key PK "schema = 3"
        TEXT value
    }
    users ||--o{ chats : "chats.user_id"
    chats ||--o{ messages : "messages.chat_id"
    users ||--o{ memories : "memories.user_id"
    users |o--o{ tasks : "tasks.user_id"
    users |o--o{ usage : "usage.user_id"
```

- **`settings`** (one row) never holds a secret. **`secrets`** holds the API key and the gateway key encrypted (section 11 of [security.md](security.md)); `key_id` tells which encryption key sealed a value, and `rev` feeds the fingerprint. **`limits`** (one row) holds the safety limits of AI settings, each with a `CHECK` of its range; they are not part of the fingerprint, so changing them never switches AI off.
- **The fingerprint** is a SHA-256 over the provider, the address, the format and auth scheme (for a gateway), the model, the revisions of the keys and the current encryption key id, never over key material. AI counts as on only while `enabled = 1`, `verified_fp` equals the current fingerprint and the keys can be decrypted. A settings change that changes the fingerprint clears `enabled` and `verified_*` in the same transaction.
- **`users`** holds only the users ai-service has seen (their id and identity stamp from the main database, and the last known name); a row is written at the first chat, memory or call of that user, not when they merely look. **`function_enabled`** (v2, default 1) is the per-user **pause** of the AI+ feature: 0 means an administrator paused the user (**Pause** in AI settings), 1 means active; a user without a row is active, and administrators are never set to 0. It is separate from the AI+ grant (`users.ai_enabled` in the main database): switching the feature off and on again does not clear a pause, only **Resume** does. The ids with 0 are what the heartbeat reports to the backend (section 5 of [architecture.md](architecture.md)). **`free_mode`** (v2, default 0) is the administrator's **free mode**: a state of the user, not of a chat. It is set in Settings under **AI chat mode** (`POST /api/ai/free`, administrators only) and is read only for administrators, so a value left on a former administrator does nothing.
- **`chats`**, **`messages`** and **`memories`** are the user content. A chat has no mode: while an administrator's free mode is on, every message in any chat, old or new, is answered in free mode. Every message records the mode it was written in (**`messages.free`**, v2, default 0), so the history can tell the model when the mode changed (section 5 of [architecture.md](architecture.md)). Deleting a user's row deletes their chats, messages, memories and tasks (`ON DELETE CASCADE`); their **`usage`** rows stay with `user_id` NULL, so the totals per model and per day stay right. `memories.content` allows up to 2000000 characters since v1.1 (the highest memory limit, 250000 tokens); the current limit is checked by the service. Study messages (below) are written with `free = 0`.
- **`usage`** has one row per model call (chats, tests, model lists, memory notes, and since v0.9 the study answers), with tokens, times and status, never any content, key or address. **`tokens_cached`** (v2) is the part of the input tokens that the provider served from its prompt cache (the **Cached** column of Usage in AI settings); NULL means the provider did not report it. **`purpose`** (v3) is `hint`, `explain` or `summary` for the model calls of the study features and NULL for AI Chat and for the calls without a chat; the **By feature** table of Usage groups the chat rows by it (AI Chat, hint, explain, summary; the four rows are always there, zeros included; `GET /api/ai/admin/usage` returns them as `byPurpose`). Its rows are kept (no retention); they are small. **The per-user request caps (v1.0)** read this table, not a counter of their own: the calls of a user in the last minute, the last hour and since 0:00 of the server's time zone are the `chat` and `memory` rows of that user (every status counts), plus the calls still running in memory. The count uses the existing `idx_usage_user` and no new index or table is added.
- **`tasks`** exists for later features; v0.8 has no route or worker for it (tasks that ended are deleted after 30 days).
- **`audit_outbox`** holds the audit lines of administrators' clears until the backend has taken them for the System log (section 11 of [security.md](security.md)).
- Indexes: `idx_chats_user (user_id, updated_at)`, `idx_chats_updated (updated_at)`, `idx_messages_chat (chat_id, id)`, `idx_memories_user (user_id, id)`, `idx_tasks_state (state, run_after)`, `idx_usage_day (day)`, `idx_usage_user (user_id, day)`, `idx_usage_model (model, day)` and, since v3, three partial ones: `idx_chats_ref`, unique on `(user_id, kind, ref) WHERE ref IS NOT NULL` (one chat per session or record, found without scanning); `idx_messages_study` on `(chat_id, question_id, kind, source, role, id) WHERE question_id IS NOT NULL` (the saved hint or analysis of one question); `idx_messages_streaming` on `(status) WHERE status = 'streaming'` (the start-up repair). `tests/v09_store.rs` checks with `EXPLAIN QUERY PLAN` that the saved-answer lookups, the chat-by-reference lookup and the repair count use them, without a temporary sort.
- Times are unix seconds; only `usage.day` is a local date (`YYYY-MM-DD` in the container's time zone, `TZ`), for the statistics.

**Study chats and messages (v0.9).** The AI+ study features write their answers into the same tables as AI Chat, so they follow the same rules: the user can read and delete them in AI Chat, administrators can read and delete them (read only, audited), and retention removes them.

- **One chat per session or record.** `chats.kind` is `chat` for AI Chat (every chat of v0.8 and earlier, by the column default), `practice` or `exam` for the hints and analyses of one practice session or one exam, `summary` for the AI summaries of one record. `chats.ref` names it: the session key (32 lower-case hex digits) of a practice or exam, or `r<recordId>@<record time>` (for example `r42@2026-11-11 21:58`) for a summary and for the chat of a record that was saved without a session key (every record from before v0.9); it is NULL for `chat`. `idx_chats_ref` makes `(user_id, kind, ref)` unique, so two asks of one session can never make two chats. `chats.record_id` is the id of the main-app record, kept for display only and never used as a lookup key (a record id can come back after the main database was restored, and a summary outlives its record); it is set once and never overwritten.
- **Titles.** The stored title of a study chat is only a time, `YYYY-MM-DD HH:MM`, in the time zone of the ai-service container: the start of the session (the browser's clock, validated) for a practice or exam, the record's own time when no start time is known, and the record time for a summary. The page adds the suffix in the interface language (`-Practice`, `-Exam`, `-AI summary` in English; `-练习`, `-考试`, `-AI总结`; `-練習`, `-試験`, `-AIまとめ`), so a title is not stored in a language. A record saved without a session key (every record of v0.8 and older) gets a chat of its own, titled with the record time, apart from the chat of the practice page it was made in.
- **Messages.** Each ask adds a pair, a user message and an assistant message, both with the same `kind`: `hint`, `explain` or `summary` (`chat` for AI Chat). `question_id`, `question_no` (the number the user saw) and `source` (`practice` for the practice page, `result` for a result page and a record's detail page) are set for hints and analyses; `lang` is the reply language (`en`, `zh-CN` or `ja`); `sample_used`, `sample_usable` and `sample_wrong` are set for a summary only. The user message of a hint or analysis holds the question as the model was given it (stem, options, statements; at most 4000 characters) and **no** answer key, no explanation and no answer of the user; the user message of a summary is one machine line (`[AI summary] <record time> <kind> <used>/<usable>/<wrong>`), and the question texts of a summary exist only in the request to the provider, never in this file. The assistant message holds the AI text. **The question texts and the AI texts are therefore copied into this database**: administrators can read them, and they stay after the bank access of the user was removed or the record was deleted, until retention or a delete removes the chat.
- **Saved answers.** The newest assistant message of a (chat, question, kind, source) is the answer the page shows. A failed, stopped or withheld regeneration is stored as its own message, and the page keeps showing the last good one under a note (`lastGood` in `GET /api/ai/study/saved`).
- **Quotas count them.** Every practice, every exam and every record summary is a chat, so study chats count toward `chats_per_user` (a new session beyond it is refused with 409 `ai.chatLimit`; chats already open keep working), and every ask adds two messages, so `messages_per_chat` 400 allows 200 asks in one session: the next one answers 409 `ai.messageLimit` until an administrator raises the limit in **Safety limits**. Deleting study chats in AI Chat frees the quota.

### 8.2a Schema v5 (v1.1) and growth

The AI database moves from schema 4 to **schema 6** once (the schema-5 steps below, then the schema-6 step), on the first start of the v1.1 ai-service, after the copy `backups/xexam-ai-schema4-<date-time>.db`. It adds:

- the bookkeeping of generation and import jobs (`tasks`, kind `bank_gen`): `charged` and `charged_at`; a job counts toward the daily quota on the day it was charged (at start for a generation, at confirm for an import);
- `gen_calls.purpose`, which tells generation calls from import calls and recognition calls;
- `users.features_off` (INTEGER, default 0): a set bit is a feature switched off for that user; the six features are chat, hint, explain, summary, generate and import (from v1.3 the hint and explain bits are always equal: one switch);
- the limits of generation and import (`gen_max_calls` 15, `import_max_calls` 25, `recognize_max_calls` 20 (8 to 40), `gen_call_tokens` 20000, `import_max_mb` 20, `import_max_questions` 50 (1 to 50)) next to the v1.0 limits.

A schema-5 file made by an earlier v1.1 development build that lacks `limits.recognize_max_calls` gets the column when it is opened (an idempotent step, no version change and no backup).

No file content is stored anywhere: an import file lives in memory while it is processed, and its text is not kept after the job.

**Growth (main database).** An AI-generated bank of 20 questions takes about 90 KB, and the same amount is in the bank download of everyone who can see it. At 5 generations a day that is about 170 MB a year; at 20 users with 3 generations a day it is about 2 GB a year. v1.1 puts no cap on kept AI banks. `AI_BANKS_AUTO_SHARE` (default `true`) decides whether a new AI bank is shared like any bank; an administrator can remove it from one user in User management.

**Author.** An AI bank's name holds the first 8 characters of its author's username, and its tip shows the author (a deliberate exception, DR1-13). `authorId` never leaves the server.


### 8.2b Schema v7 (v1.3)

The AI database moves from schema 6 to **schema 7** once (one transaction, one copy `backups/xexam-ai-schema6-<date-time>.db`), on the first start of the v1.3 ai-service.

- **`connections`** (new): `id`, `name` (unique, 1 to 40 characters), `provider` (`openai`, `anthropic` or `gateway`), `base_url`, `format`, `responses` (v1.8), `gateway_auth`, `status` (`ok` or `broken`), `error_code`, `lists_models`, `tested_at`, `rev` (+1 on every save; the cache key and the lock), `sort`, and the creation and update facts. Up to 8 rows.
- **`connection_secrets`** (new): the encrypted `api_key` and `gateway_key` of a connection (key id, 24-byte nonce, ciphertext, a hint of the last characters, and `aad_ver` 2, which binds the ciphertext to the connection; migrated rows keep 1 until their next rotation). The old `secrets` table stays, unused.
- **`models`** (new): the pool: one row per tested (connection, model), with `status`, `error_code`, `context`, `latency_ms`, `tested_at`, `manual`, and the call settings. **`model_slots`** (new): six rows (`default`, `chat`, `study`, `summary`, `generate`, `import`), each pointing to a pool model or to nothing (an empty feature slot uses the default).
- **`study_shared`** (new): the shared hints and analyses, **without a user id**: `question_id`, `content_hash`, `kind` (`hint` or `explain`), `lang`, `content`, `model`, `connection`, token counts, `gen` (1 + replacements), `hits`, and times. Unique on (`question_id`, `content_hash`, `kind`, `lang`); at most 50 000 rows, unused ones removed after `study_share_days`.
- **`gen_items`** (new): the accepted questions of a **running** generation job (`task_id`, `no`, `data`), deleted when the job ends. A removed question keeps its number.
- `messages`: `reasoning`, `reasoning_ms` and `reasoning_truncated` (below), `parent_id` and `connection` (the connection name at call time); `chats`: `leaf_id` (the shown branch) and `pick_conn`, `pick_model` (an administrator's model for a free-mode chat; no foreign key: removing a connection clears the picks that point to it, and a start clears any that point to a missing connection). Index `idx_messages_parent`. A chat is a tree by `parent_id`; what is shown or sent to the model is the path from the leaf up to the root, and `messages_per_chat` counts every version.
- `settings.features_off` (bit mask of the six global switches; the hint and explain bits are always equal); `usage` and `gen_calls` get `connection_id` and `connection`.
- `limits`: `calls_admins_exempt` (1), `study_share` (1), `study_share_days` (180), `gen_max_chunks` (8), `summary_max_items` (40), `import_file_questions` (1 to 300; the old 50-capped `import_max_questions` stays unused) and `daily_token_budget` (0 = off).

The one-time statements fill the six slots, the chat tree (`parent_id` = the message before, `leaf_id` = the last one), connection 1 with its keys and the default slot, the hint and explain bits of `users.features_off` (a user with only one of the two off gets both on), and the limits as described in section 000.1 of [migration.md](migration.md). A new file and an upgraded file end with the same tables. A v1.2 ai-service refuses a schema-7 file; restore the schema-6 copy to go back.

**The reasoning columns (v1.3).** Three nullable columns of `messages`, which holds one row per answer version, store the thinking of an AI Chat answer: `reasoning` (text; trimmed; at most 32 KiB = 32 768 bytes, the head is kept and cut at a character boundary), `reasoning_ms` (milliseconds from the first reasoning to the first answer text, or to the end; NULL without reasoning) and `reasoning_truncated` (`0` / `1`, default 0; 1 = the text was cut at the cap). They join the schema-7 step, so there is no schema 8 and no extra backup: schema 7 had not shipped before v1.3; a database upgraded from 1 to 6 gets them in the same transaction, and a schema-7 file written by an earlier v1.3 build gets them at the next start. They are written only for AI Chat answers (never for a hint, an analysis or a summary, which discard reasoning), are never part of the context sent to the model, and are deleted with the message: deleting a chat, **Delete all chats**, an administrator's clearing, the removal of a user and the retention sweep remove them with the row (`secure_delete=ON` overwrites the freed pages). Older answers have NULL and show no block. `messages.model` and `messages.connection` stay stored for every answer (administrators and the usage statistics); only the responses to non-administrators leave them out.

### 8.2c Schema v8 (v1.4)

The AI database moves from schema 7 to **schema 8** once (one transaction, one copy `backups/xexam-ai-schema7-<date-time>.db`), on the first start of the v1.4 ai-service. The step is additive.

- **`geo_settings`** (new): at most one row (`id = 1`), the settings of the external location lookup: `preset` (`ipinfo`, `ipapi`, `ipapico`, `ipwhois`, `ipsb` or `custom`, for display), `url_template` (with `{ip}` and optionally `{key}`), `key_header_name` and `key_header_value` (the optional header that carries the key), the field mapping (`map_country`, `map_country_code`, `map_region`, `map_city`, `ok_path`, `ok_value`, `error_path`, `message_path`; dot paths), the **sealed key** (`key_id`, `key_nonce`, `key_ciphertext`, `key_hint`; all NULL / empty without a key; sealed with the master key and its own binding, never returned) and `tested_at`, `updated_at`, `updated_by`. A row exists only after a passing "test and save", so saved means tested; deleting the settings deletes the row. The table is empty after the upgrade.

A new file and an upgraded file end with the same tables. A v1.3 ai-service refuses a schema-8 file; restore the schema-7 copy to go back.

### 8.2d Thinking strength (v1.6, no schema change)

`model_slots` gets one nullable column, `effort TEXT`: the thinking strength of the slot as `"<family>:<level>"` (for example `anthropic:high`, `google:MEDIUM`, `responses:none`; family `anthropic`, `openai`, `google` or `responses`; `gemma` was removed in v1.9, section 8.2f), or NULL for **Default** (nothing is sent). The column is added at start when it is missing (`ALTER TABLE ... ADD COLUMN`); **the AI schema stays 8, so there is no backup copy and no new migration step**, and a v1.5 ai-service opens the file as it is (it never reads the column). A value that does not parse is read as NULL. The page sets it with `POST /api/ai/admin/models/effort` (section 5.10 of [api.md](api.md)). The refusal memory of the parameter (and of structured output) is in memory only.

### 8.2e OpenAI Responses format (v1.8, no schema change)

`connections` gets one column, `responses INTEGER NOT NULL DEFAULT 0 CHECK (responses IN (0, 1))`: 1 marks a gateway connection that uses the OpenAI Responses API. Its `format` stays `openai`, because the `CHECK` on `format` allows only `openai` and `anthropic` and the table is not rebuilt. The column is added at start when it is missing (`ALTER TABLE ... ADD COLUMN`, the same way as `model_slots.effort`); **the AI schema stays 8, so there is no backup copy and no new migration step**. New and upgraded files end with the same tables. Saving a Responses connection writes `format = 'openai', responses = 1`, any other format writes `responses = 0`, and `format = 'openai' AND responses = 1` is read as Responses. A v1.7 ai-service opens the file as it is and never reads the column, so a rollback needs no restore, but a Responses connection then reads as OpenAI chat completions.

### 8.2f Schema v9 (v1.9)

The AI database moves from schema 8 to **schema 9** once, on the first start of the v1.9 ai-service, after one backup copy `backups/xexam-ai-schema8-<date-time>.db` (the same mechanism as every earlier step). The main database does not change.

- **`connections.enabled`** (`INTEGER NOT NULL DEFAULT 1 CHECK (enabled IN (0, 1))`): the on/off switch of a connection. Every existing row starts on. It is added like the earlier columns (at start, when `pragma_table_info` does not list it); the `status` check is untouched, "Off" is derived from `enabled`, and `connections` is not rebuilt (it has `CASCADE` children).
- **`limits.pace_on`, `limits.pace_rpm`, `limits.pace_tpm`**: the provider rate-limit pacing of AI generation and import: `pace_on` 0 or 1 (default 1), `pace_rpm` 1 to 10 000 (default 15), `pace_tpm` 1 000 to 100 000 000 (default 250 000). They are the keys `paceOn`, `paceRpm` and `paceTpm` of the limits API. The unlimited switches never change them.
- **`model_slots` is rebuilt**: the `CHECK` on `slot` now allows `default`, `chat`, `hint`, `explain`, `summary`, `generate` and `import` (seven rows; `study` is gone). The upgrade copies the `study` row (model, time, author and thinking strength) into both `hint` and `explain`, then drops the old table; a new file creates the new table directly, so both end with the same `sqlite_master`. Nothing references `model_slots`, so the rebuild runs with `foreign_keys` on.
- **Gemma folded into Google**: on every start, a stored thinking strength `gemma:HIGH` or `gemma:true` becomes `google:HIGH`, `gemma:MINIMAL` or `gemma:false` becomes `google:MINIMAL`, and any other `gemma:` value is cleared (NULL). The wire request is the same as before. The step only runs when a `gemma:` row exists. Families are now `anthropic`, `openai`, `google` and `responses`.
- **Global switches are independent**: `settings.features_off` bits 2 (hint) and 4 (explain) are no longer kept equal, and a start never pairs them again; `users.features_off` still pairs them (a user with only one of the two off gets both). The step that pairs the user bits runs without touching the global bits.
- **Job waiting flag**: while a generation or import job waits for provider quota, `tasks.payload` carries a transient `rateWait` flag (never bumps `updated_at`); it is the `waiting` field of the job objects.
- **Going back**: a v1.8 ai-service refuses schema 9 (`written by a newer ai-service`): stop, restore `backups/xexam-ai-schema8-<date-time>.db` as in 8.4, and run the v1.8 images. Settings, chats and usage written after the upgrade are not in the copy and are lost.

### 8.3 What the service does with it

- **Opening**: the tables are created (`IF NOT EXISTS`), the `settings` and `limits` rows are added with their defaults, and `meta.schema` is set to `4` on a new database. A database with `meta.schema` `1`, `2` or `3` is upgraded (see Migrations below). A database written by a newer ai-service (`meta.schema` above `4`) is refused: the service logs one line and does not start, so an older image never opens a newer file. Answers left `streaming` by a crash or a kill are marked `error` with the code `ai.interrupted` (only when there are any; `idx_messages_streaming` finds them).
- **Retention**: 60 seconds after the start and then every hour, chats whose last message is older than the retention days are deleted (not one whose answer is being written), with their messages, study chats like any other. The sweep deletes **20 chats per transaction** with a 20 ms pause after every full batch, on a blocking thread, so a large clean-up (the retention lowered, or the service stopped for a long time) never holds the database locked for long and the answers being written keep saving; one log line gives the total. A user's delete of all their chats, an administrator's clear and the removal of a deleted user's data still delete in one transaction. Memories are not subject to retention.
- **Reconcile** with the main app: at start (retried every 30 seconds until it works), every 10 minutes and when an administrator opens **Users with AI+**, the service asks the backend which of its user ids still exist, with which stamp and whether they hold the AI+ grant. The data of an id that is gone, or that now has another stamp (a user id reused after the main database was restored), is deleted after any answer of that user was stopped. The same happens at once when a request arrives with such an identity.
- **Deleted content**: with `secure_delete=ON` freed pages are overwritten, and after every delete of user content the service runs `PRAGMA wal_checkpoint(TRUNCATE)` (at most once every 10 seconds) so that the old pages do not remain in `xexam-ai.db-wal`. Backups and volume snapshots still have what was deleted.
- **Stopping**: on a normal stop the running answers are saved as stopped, then a checkpoint runs.
- **Migrations**: schema v1 was the first schema (a pre-release v0.8 build); **schema v2** (v0.8) added four columns: `users.function_enabled` (default 1), `users.free_mode` (default 0), `messages.free` (default 0) and `usage.tokens_cached` (NULL); **schema v3** (v0.9) adds **twelve columns and three indexes** and nothing else: `chats.kind` (default `chat`), `chats.ref`, `chats.record_id`, `messages.kind` (default `chat`), `messages.question_id`, `messages.question_no`, `messages.source`, `messages.lang`, `messages.sample_used`, `messages.sample_usable`, `messages.sample_wrong` and `usage.purpose` (NULL), plus `idx_chats_ref`, `idx_messages_study` and `idx_messages_streaming`. No row is rewritten and no row changes meaning: every old chat and message is `chat` and every old usage row has no purpose. The same additive step creates a new database and upgrades an old one, so both end up with the same tables and the same `sqlite_master`. **Every database of schema 1, 2 or 3 is copied first** with `VACUUM INTO` to `backups/xexam-ai-schema<N>-<YYYYMMDD-HHMMSS>.db` inside the AI data directory (`<N>` is the schema it had, so `xexam-ai-schema2-…` for a v0.8 database and `xexam-ai-schema3-…` for a v0.9 or v0.9.1 database; directory mode 0700; the time is the container's local time; a `-2`, `-3`… suffix if the name is taken; an empty database is copied too), then upgraded in one transaction, and one log line names the copy: `Upgraded the AI database to schema 4. A copy of the previous version was saved first: <path>.` If the copy cannot be made the service does not start and nothing is changed. A schema-2 file of an earlier pre-release build that lacks some of the v2 columns gets them added in the same step. Reopening a schema-3 database writes nothing. **Keep free disk space of about the size of the AI database** for the copy; copies are never pruned and keep chats and memories that were deleted later (delete them yourself when that matters, section 11 of [security.md](security.md)). **Going back to v0.8 needs the copy**: a v0.8 ai-service refuses schema 3 (`written by a newer ai-service … restore a backup`) and exits, so restore `xexam-ai-schema2-….db` as in 8.4; chats, memories, settings and usage written after the upgrade are not in it and are lost.
- **Schema 4 (v1.0)** adds **four columns** and nothing else: `limits.calls_per_minute` (default 5), `limits.calls_per_hour` (default 20), `limits.calls_per_day` (default 100) and `limits.reply_max_chars` (default 1000), each with a `CHECK` of its range (the ranges are the same as in the application's list of limits). Nothing is dropped: `limits.message_chars` stays in the table, unused. Once, in the same transaction, `UPDATE limits SET memory_chars = 200 WHERE memory_chars = 300` moves the old default of the memory length to the new one; any other value an administrator chose stays. The same step runs on a new database and on an upgrade from 1, 2 or 3, so both end up with the same `sqlite_master`, and never again on a later start. A database upgraded from 1, 2 or 3 gets the defaults for the four new columns; nothing is derived from old values. **Keep the copy of the schema-3 database** (`backups/xexam-ai-schema3-<date-time>.db`): a v0.9.1 ai-service refuses schema 4 with its existing message, so going back means restoring that copy and running the v0.9.1 images (section 8.4 and [migration.md](migration.md)).

- **Schema 5 to 6 (v1.1)** moves the lengths to tokens. Three columns are added with their own `CHECK` ranges: `limits.reply_max_tokens` (1 to 250000, default 3000), `limits.memory_tokens` (50 to 250000, default 1000) and `limits.gen_call_tokens` (1000 to 250000, default 20000). `reply_max_chars`, `memory_chars` and `gen_call_chars` stay in the table, unused. Once, in the same transaction: a value still at its old default becomes the new default, any other value is multiplied by 1.5 and clamped to its range. The `memories` table is rebuilt once (rows and ids kept) so that one memory may hold up to 2000000 characters. Upgrading from schema 5 writes the copy `backups/xexam-ai-schema5-<date-time>.db` first.

### 8.4 Backup and restore of the AI volume

```bash
docker compose stop

# Back up the AI volume (the AI database AND the key file secret.key, which decrypts the saved keys: keep it private)
docker run --rm -v xexam_ai_data:/data -v "$PWD:/backup" alpine tar czf /backup/xexam_ai_data.tgz -C /data .

docker compose start
```

Restore it like the main volume: stop, remove `xexam-ai.db-wal` and `xexam-ai.db-shm`, unpack, start (`docker run --rm -v xexam_ai_data:/data -v "$PWD:/backup" alpine sh -c 'rm -f /data/xexam-ai.db-wal /data/xexam-ai.db-shm && tar xzf /backup/xexam_ai_data.tgz -C /data'`). The folder `backups/` of the AI volume (the copy of an AI schema upgrade) is part of the volume and of its backup.

- With `XEXAM_AI_SECRET_KEY` set, the key file is not used: back up that value separately (it is not in the volume). Without the right key the restored keys cannot be decrypted; AI then stays off until an administrator enters the keys again, and nothing else is lost.
- A restored AI volume and a main volume from another point in time fit together: users that no longer exist lose their AI data at the next reconcile, and a user id that now belongs to another account is recognised by its stamp.
- `xexam_ai_auth` needs no backup: it holds only `internal.token`, which the backend creates again when it is missing (the AI service picks up the new token by itself).

## 9. Schema map of both databases (v1.9)

A reading guide: which tables exist, what each is for, and how they relate. It describes the main database at schema 10 and the AI database at schema 9; table and column names are exactly as stored. Nothing here changes a table.

### 9.1 Main database `xexam.db` (backend)

| Area | Tables | Note |
|---|---|---|
| Question banks | `banks`, `questions`, `imports` | A bank is one imported workbook; a question that leaves a workbook is retired (`active = 0`), not deleted |
| Study records and the wrong-answer notebook | `attempts`, `attempt_items`, `user_stats` | `attempts` is what the app calls a study record (exam or practice) |
| Users and access | `users`, `sessions`, `user_banks`, `user_settings` | |
| System | `meta`, `system_settings`, `activity_log` | |
| Legacy (unused) | `stats`, `settings` | Copied to `user_stats` and `user_settings` by the v0.6 step; read only when upgrading from schema 2 or older |

The main database declares **no foreign keys**: the relations below are kept by the code (deletes run child rows first), so the diagram shows them dotted.

```mermaid
erDiagram
  users ||..o{ sessions : "user_id"
  users ||..o{ attempts : "user_id (NULL = unclaimed old record)"
  users ||..o{ user_stats : "user_id"
  users ||..o{ user_settings : "user_id"
  users ||..o{ user_banks : "user_id"
  users ||..o{ banks : "owner_id"
  users |o..o{ system_settings : "updated_by"
  users |o..o{ activity_log : "user_id (snapshot, not linked)"
  banks ||..o{ user_banks : "bank_id"
  banks ||..o{ questions : "bank_id"
  banks ||..o{ imports : "bank_id"
  attempts ||..o{ attempt_items : "attempt_id"
  questions |o..o{ attempt_items : "question_id (soft)"
  questions |o..o{ user_stats : "question_id (soft)"

  users {
    int id PK
    text username UK
    text password_hash
    text role
    int disabled
    int must_change_password
    int new_banks
    int can_manage_banks
    int can_download_banks
    int ai_enabled
    text uid
    text created_at
    text last_login_at
    text last_active_at
  }
  sessions {
    text token_hash PK
    int user_id
    int started_at
    int last_seen_at
    int expires_at
    int remember
    text ip
    text ua
  }
  banks {
    int id PK
    text name
    text filename
    int legacy_ids
    int owner_id
    text ai_job UK
    text ai_meta
    text created_at
    text updated_at
  }
  questions {
    text id PK
    int bank_id
    text sheet
    int row_no
    int seq
    text data
    int active
    text added_at
    text updated_at
  }
  imports {
    int id PK
    int bank_id
    text at
    text filename
    int added
    int updated
    int removed
    int restored
    int skipped
    int total
  }
  attempts {
    int id PK
    int user_id
    text kind
    text mode
    text at
    text sheets
    text banks
    int total
    int correct
    int percent
    int seconds
    text session_key
    int started_at
    int time_limit
    text ended
  }
  attempt_items {
    int attempt_id PK
    int seq PK
    text question_id
    text answer
    int correct
  }
  user_stats {
    int user_id PK
    text question_id PK
    int attempts
    int correct
    int wrong
    int streak
    int last
    text at
  }
  user_settings {
    int user_id PK
    text key PK
    text value
  }
  user_banks {
    int user_id PK
    int bank_id PK
  }
  system_settings {
    text key PK
    text value
    text updated_at
    int updated_by
  }
  activity_log {
    int id PK
    int ts
    int last_ts
    int count
    text actor
    int user_id
    text username
    text role
    text action
    text object
    text detail
    text result
    text reason
    text source
    text src_id UK
  }
  meta {
    text key PK
    text value
  }
  stats {
    text question_id PK
  }
  settings {
    text key PK
  }
```

| Table | Purpose | Key and links | Notes |
|---|---|---|---|
| `meta` | Database-level key/value: `schema`, `banks_known`, `users_known`, `legacy_migrated` | PK `key` | |
| `banks` | Question banks, including the job key and facts of AI-generated banks | PK `id`; `owner_id` to `users` | `ai_job` is a partial unique index |
| `questions` | One row per question; the whole question (all languages) is JSON in `data` | PK `id` (text); `bank_id` to `banks` | Indexes by sheet, bank and layout |
| `imports` | Result counts of each Excel import | PK `id`; `bank_id` to `banks` | |
| `attempts` | Study records: one row per exam or practice session | PK `id`; `user_id` to `users` (NULL = unclaimed old record) | `sheets` and `banks` are JSON snapshots on purpose |
| `attempt_items` | The answer to each question of a record | PK `(attempt_id, seq)`; `question_id` to `questions` (soft) | A record may outlive its question |
| `user_stats` | Per user and question: attempts, correct, wrong, streak (wrong-answer notebook) | PK `(user_id, question_id)` | |
| `users` | Accounts, role, permissions, AI+ switch, identity stamp `uid`, last sign-in and last activity (IP, place, user agent) | PK `id`; `username` unique, case-insensitive | Mixes account data with sign-in telemetry |
| `sessions` | Sign-in sessions | PK `token_hash`; `user_id` to `users` | Times are unix seconds |
| `user_banks` | Which banks a plain user sees | PK `(user_id, bank_id)` | |
| `user_settings` | Per-user interface settings (JSON values) | PK `(user_id, key)` | |
| `system_settings` | Site-wide switches (activity retention, visits, sign-in info, location source) | PK `key`; `updated_by` is a user id | |
| `activity_log` | Activity history, including the rows ai-service forwards | PK `id`; `src_id` partial unique (de-duplication) | `username` and `role` are snapshots, so the log stays readable after a user is deleted |
| `stats`, `settings` | Legacy, see above | | |

### 9.2 AI database `xexam-ai.db` (ai-service)

| Area | Tables | Note |
|---|---|---|
| AI settings | `settings` (one row), `limits` (one row), `connections`, `connection_secrets`, `models`, `model_slots`, `geo_settings` (one row) | `model_slots` has seven rows: `default`, `chat`, `hint`, `explain`, `summary`, `generate`, `import` |
| Identity | `users` | A mirror of the main database's users plus per-user AI state |
| Chats and study sessions | `chats`, `messages`, `memories` | `chats.kind` is `chat`, `practice`, `exam` or `summary` |
| Shared hints and analyses | `study_shared` | Reused across users, without a user id |
| AI generation and import | `tasks`, `gen_items`, `gen_calls` | `tasks.kind` is `bank_gen` for both; the payload tells them apart |
| Metering | `usage` (chat, hint, analysis, summary, checks), `gen_calls` (generation and import) | Two tables on purpose: the per-user call caps read only `usage` |
| System | `meta`, `audit_outbox` | |
| Legacy | `secrets`; the old connection columns of `settings`; four retired length columns of `limits` | Read only when upgrading from an old schema |

Solid lines are declared foreign keys; dotted lines are relations kept without a foreign key on purpose (message branches, a chat's chosen connection, usage history).

```mermaid
erDiagram
  users ||--o{ chats : "CASCADE"
  users ||--o{ memories : "CASCADE"
  users |o--o{ tasks : "CASCADE"
  users |o--o{ usage : "SET NULL"
  users |o--o{ gen_calls : "SET NULL"
  chats ||--o{ messages : "CASCADE"
  messages |o..o{ messages : "parent_id (no FK)"
  chats |o..o| messages : "leaf_id (no FK)"
  connections |o..o{ chats : "pick_conn (no FK)"
  tasks ||--o{ gen_items : "CASCADE"
  tasks |o--o{ gen_calls : "SET NULL"
  connections ||--o{ connection_secrets : "CASCADE"
  connections ||--o{ models : "CASCADE"
  models |o--o{ model_slots : "SET NULL"
  connections |o..o{ usage : "connection_id (no FK)"
  connections |o..o{ gen_calls : "connection_id (no FK)"

  users {
    int id PK
    text stamp
    text username
    int function_enabled
    int free_mode
    int features_off
    int first_seen_at
    int last_seen_at
  }
  chats {
    int id PK
    int user_id FK
    text kind
    text ref UK
    int record_id
    text title
    int leaf_id
    int pick_conn
    text pick_model
    int created_at
    int updated_at
  }
  messages {
    int id PK
    int chat_id FK
    int parent_id
    text role
    text kind
    text content
    text status
    text error_code
    int truncated
    int free
    text model
    text connection
    text question_id
    int question_no
    text source
    text lang
    text reasoning
    int reasoning_ms
    int reasoning_truncated
    int created_at
    int finished_at
  }
  memories {
    int id PK
    int user_id FK
    text content
    text source
    int created_at
    int updated_at
  }
  tasks {
    int id PK
    int user_id FK
    text kind
    text state
    text payload
    text result
    text error_code
    int attempts
    int run_after
    text job_key UK
    int goal
    int made
    text phase
    int charged
    int seen
    int created_at
    int updated_at
  }
  gen_items {
    int task_id PK
    int no PK
    int slot
    text data
    int at
  }
  gen_calls {
    int id PK
    int task_id FK
    int user_id FK
    text purpose
    int at
    text day
    text provider
    text model
    int connection_id
    text connection
    int tokens_in
    int tokens_out
    int latency_ms
    text status
    text error_code
    int asked
    int accepted
    int truncated
  }
  usage {
    int id PK
    int user_id FK
    int at
    text day
    text kind
    text purpose
    text provider
    text model
    int connection_id
    text connection
    int tokens_in
    int tokens_out
    int latency_ms
    text status
    text error_code
  }
  connections {
    int id PK
    text name UK
    text provider
    text base_url
    text format
    int responses
    int enabled
    text gateway_auth
    text status
    text error_code
    int lists_models
    int tested_at
    int rev
    int sort
  }
  connection_secrets {
    int connection_id PK
    text name PK
    text key_id
    blob nonce
    blob ciphertext
    text hint
    int rev
    int aad_ver
  }
  models {
    int id PK
    int connection_id FK
    text model
    int manual
    text status
    text error_code
    int usage_option
    int max_tokens
    int context
    int latency_ms
    int tested_at
  }
  model_slots {
    text slot PK
    int model_id FK
    text effort
    int updated_at
    text updated_by
  }
  study_shared {
    int id PK
    text question_id
    text content_hash
    text kind
    text lang
    text content
    text model
    int gen
    int hits
    int last_hit_at
  }
  settings {
    int id PK
    int enabled
    int retention_days
    int features_off
  }
  limits {
    int id PK
    int pace_on
    int pace_rpm
    int pace_tpm
  }
  geo_settings {
    int id PK
    text preset
    text url_template
    blob key_ciphertext
    int updated_by
  }
  secrets {
    text name PK
  }
  meta {
    text key PK
    text value
  }
  audit_outbox {
    int id PK
    int at
    text message
  }
```

| Table | Purpose | Key and links | Notes |
|---|---|---|---|
| `meta` | `schema`, `secret_rev`, one-time flags | PK `key` | |
| `settings` | One row: the AI master switch `enabled`, `retention_days`, `features_off` (global switch bits) | PK `id = 1` | Eleven old single-connection columns remain, read only by old upgrades |
| `limits` | One row: every limit (concurrency, chat and memory sizes, per-user call caps, answer length, generation and import limits, shared cache, daily token budget) and, since v1.9, the pacing settings | PK `id = 1`; every column has a range `CHECK` | Four retired length columns stay, unused |
| `users` | Mirror of the main users (`id` plus `stamp` = main `uid`) with `function_enabled` (paused), `free_mode` and `features_off` (per-user feature bits) | PK `id` (given by the main database) | A different stamp means a different person: the AI data is dropped |
| `chats` | AI Chat conversations and the chat of each practice, exam or record summary | PK `id`; `user_id` to `users` (CASCADE) | `leaf_id` and `pick_conn` have no foreign key on purpose |
| `messages` | Messages and their branch versions; hints, analyses and summaries too | PK `id`; `chat_id` to `chats` (CASCADE) | Some columns only matter for one `kind` |
| `memories` | Per-user memory notes | PK `id`; `user_id` to `users` (CASCADE) | |
| `tasks` | Background jobs: AI generation and AI import | PK `id`; `user_id` to `users` (CASCADE) | `job_key` is a partial unique index |
| `gen_items` | Accepted questions of a running job (resume) | PK `(task_id, no)`; to `tasks` (CASCADE) | |
| `gen_calls` | Each model call of generation and import (tokens, time, result) | PK `id`; `task_id`, `user_id` (SET NULL) | |
| `usage` | Every other model call, with tokens, time and status, never content | PK `id`; `user_id` (SET NULL) | |
| `audit_outbox` | Activity rows waiting to be sent to the backend | PK `id` | A temporary queue |
| `connections` | AI connections: address, format, gateway auth, status and the on/off switch `enabled` | PK `id`; `name` unique, case-insensitive | `format` allows `openai` and `anthropic`; OpenAI Responses is `openai` plus `responses = 1` |
| `connection_secrets` | The sealed keys of a connection | PK `(connection_id, name)`; to `connections` (CASCADE) | |
| `models` | The model pool: one row per tested connection and model | PK `id`; to `connections` (CASCADE); unique `(connection_id, model)` | |
| `model_slots` | Which pool model each feature uses, and its thinking strength | PK `slot`; `model_id` to `models` (SET NULL) | |
| `study_shared` | The shared cache of hints and analyses | PK `id`; unique `(question_id, content_hash, kind, lang)` | `lang` uses `zh`, while `messages.lang` uses `zh-CN` |
| `geo_settings` | The external location lookup (one row, written only after a passing test) | PK `id = 1` | |
| `secrets` | Legacy: the keys of schemas before 7 | PK `name` | Unused by the running code |

### 9.3 References between the two databases

A foreign key cannot cross two files, so these links are kept by identity and by code:

| AI database | Main database | What keeps them consistent |
|---|---|---|
| `users.id` + `users.stamp` | `users.id` + `users.uid` | A different stamp means a different person: the AI data is deleted |
| `chats.ref` | `attempts.session_key` (or `r<recordId>@<at>`) | A lookup key only |
| `chats.record_id` | `attempts.id` | Shown only, never looked up (ids can be reused after a restore) |
| `messages.question_id`, `study_shared.question_id` | `questions.id` | Soft links; `study_shared.content_hash` guards against a question that changed |
| `tasks.job_key` | `banks.ai_job` | A partial unique index in the main database gives one bank per job |
| `audit_outbox` | `activity_log` (`source = 'ai'`, `src_id`) | De-duplicated by `src_id` |

### 9.4 Conventions for new tables and columns

- Times are `*_at INTEGER` (unix seconds, UTC); booleans are `INTEGER NOT NULL DEFAULT x CHECK (col IN (0, 1))`; enumerations carry a `CHECK`.
- An id that points into the same database is `*_id` with a foreign key and a stated `ON DELETE`; when there is none, a comment says why.
- A new `updated_by` is a user id; language codes are `en`, `zh-CN` and `ja`.
- Existing names and the older text time columns of the main database are kept as they are.
