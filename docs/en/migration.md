**English** | [日本語](../ja/migration.md) | [简体中文](../zh-CN/migration.md)

# Migration and rollback

> This document covers every upgrade path, newest first:
>
> - **v0.6 → v0.7** (section 1): the database moves to schema v4 and to WAL mode, automatically, after a backup copy is written.
> - **v0.5 or v0.5.1 → v0.7** (section 2): sign-in arrives together with schema v4, in one step.
> - **v0.4 → v0.5 and later** (section 3): question banks, exam detail and settings (schema v2), today together with everything after it.
> - **Python v0.2.x → Rust + TypeScript + Nginx (v0.3)** (sections 4–10): the database file and its schema did not change then; the switch meant "swapping the program", not "moving data". Basis for that decision: [migration-plan.md](../zh-CN/history/migration-plan.md). Risks: [risk-assessment.md](../zh-CN/history/risk-assessment.md).
> - **v0.1 → later** (section 11): the database file name.
>
> Every upgrade that changes the schema writes **one** copy of the database first, named after the schema it had: `backups/xexam-schema<N>-<YYYYmmdd-HHMMSS>.db` next to `xexam.db` (in Docker: `/data/backups/` in the volume `xexam_data`). Rolling back always means **restoring that copy** and running the older images. The background is in section 6 of [database.md](database.md).

## 1. Upgrading from v0.6 to v0.7

### 1.1 What changes

On its **first start** on your data volume, v0.7 does this before it accepts any request:

1. **Backup.** It writes a copy of the current database to `/data/backups/xexam-schema3-<YYYYmmdd-HHMMSS>.db`. The copy is an ordinary schema-3 database that v0.6 can open. **If the copy cannot be written** (disk full, no permission), the backend refuses to start and leaves the database unchanged; the log says why.
2. **Migration to schema v4**, in one transaction:
   - **Every existing user keeps seeing every question bank** (`all_banks = 1`, shown as **All question banks, including ones imported later**). Users you add later start with no bank until you choose some.
   - **Every existing bank belongs to the first administrator** (the enabled administrator with the lowest id). Nobody is a **Bank manager** yet.
   - **Every existing study record is an exam.** Practice records appear only from now on.
   - Question ids, statistics, records, settings, accounts and sessions do not change: nobody is signed out.
3. **WAL mode.** Only after a successful migration does the backend switch the database to WAL. While it runs, `xexam.db-wal` and `xexam.db-shm` sit next to `xexam.db`; a normal stop folds them back into `xexam.db` and empties `xexam.db-wal` (the two files may stay, empty and harmless).

What else you will notice:

- In **User management** each row shows the user's question banks, with **Change** to limit them, and the **Bank manager** permission in the Add user form and the Edit dialog. Taking a bank away keeps the user's records of it as history.
- **Start practice** next to **Start exam** on Home; practice records are tagged **Practice** in **Study records**.
- A **System log** entry in the menu of administrators.
- The answers and explanations have their own **English** / **中文** / **日本語** switch on each card, and six-column workbooks can be imported (import them as a **New bank**; see the README).

The new `docker-compose.yml` differs from the one of v0.6 in three ways: it follows the host's time zone instead of a fixed default (it mounts `/etc/localtime`; a `TZ` in `docker/.env` overrides it), it limits each container's log to 3 files of 10 MB, and it passes `XEXAM_SQLITE_JOURNAL` when `docker/.env` sets it. The v0.6 file still starts v0.7, but without these changes. No variable has to be added to `.env`.

### 1.2 Upgrade

```bash
# 1. Back up the volume (stop first; the backup file holds your whole database, do not commit it)
docker compose stop
docker run --rm -v xexam_data:/data -v "$PWD:/backup" alpine tar czf /backup/xexam-data-v06.tgz -C /data .

# 2. Replace docker-compose.yml with the new one (your .env stays; if you changed docker-compose.yml yourself, carry those changes over)
curl -fLO https://raw.githubusercontent.com/Yumin-Space/xexam/main/docker/docker-compose.yml

# 3. Pull v0.7 and start (do not use down -v)
docker compose pull
docker compose up -d

# 4. Look for the upgrade message, the name of the copy and the journal mode
docker compose logs backend
```

The log should contain `Upgraded the database to schema 4. A copy of the previous version was saved first: /data/backups/xexam-schema3-20261111-140501.db (…)` and `SQLite journal mode: wal`.

- **Time zone.** v0.6's compose file set a fixed time zone unless `.env` named another one. If the host's zone differs from the one your timestamps so far were written in, set that zone as `TZ` in `docker/.env` (for example `TZ=UTC`), so new records continue in it. Stored times are never rewritten.
- **A host without `/etc/localtime`**: Compose stops with "bind source path does not exist". Remove the six lines of that mount from `docker-compose.yml` and set `TZ` in `docker/.env` (section 3.1 of [deployment.md](deployment.md)).
- **Unusual storage**: if the volume is on a network filesystem or on a Windows folder bind-mounted into Docker Desktop, put `XEXAM_SQLITE_JOURNAL=delete` in `docker/.env` before step 3.
- **Source build**: `git pull && docker compose -f docker/docker-compose.build.yml up -d --build`, from the repository root, then `docker compose -f docker/docker-compose.build.yml logs backend`. Without Docker, the copy goes to `backups/` inside the data directory (`--data` or `XEXAM_DATA`).

### 1.3 Checks after the upgrade

- [ ] `version` in `/api/info` is `0.7`, and `bankCount` (the number of active questions) is the same as before.
- [ ] You can sign in as before; nobody had to sign in again.
- [ ] **User management**: every user from before shows **Question banks: all, including ones imported later**.
- [ ] **Question banks** lists the same banks, each **uploaded by** the first administrator.
- [ ] **Study records** lists the earlier records, each tagged **Exam**, and the wrong-answer review has as many questions as before.
- [ ] **System log** shows the start-up lines, among them `SQLite journal mode: wal`.
- [ ] Take a short practice and check its time in **Study records** against the clock. If it is off by whole hours, set `TZ` (section 1.2).
- [ ] Then decide, per user, which banks they see and who may manage banks.

### 1.4 Rollback to v0.6

Rolling back means **restoring the copy written before the migration** and running the v0.6 images. Everything done in v0.7 after the migration is lost: new banks, imports, records (also practice records), accounts, bank permissions and settings.

```bash
# 1. Stop v0.7 (do not use down -v)
docker compose stop

# 2. Find the copy. After a normal stop any xexam.db-wal left in /data is empty.
docker run --rm -v xexam_data:/data alpine ls -l /data /data/backups

# 3. Put the copy back as xexam.db (use the file name from step 2). Leftover -wal / -shm files belong to the
#    database being replaced and must not be applied to the copy, so they are removed first.
docker run --rm -v xexam_data:/data alpine sh -c 'rm -f /data/xexam.db-wal /data/xexam.db-shm && cp /data/backups/xexam-schema3-20261111-140501.db /data/xexam.db'

# 4. Pin the v0.6 images in docker-compose.yml (on Docker Hub, no login), then start:
#      image: docker.io/yuminspace/xexam-backend:v0.6
#      image: docker.io/yuminspace/xexam-frontend:v0.6
docker compose up -d
```

The new `docker-compose.yml` works with the v0.6 images too. For a source build, check out the release instead: `git fetch --tags && git checkout v0.6 && docker compose -f docker/docker-compose.build.yml up -d --build`.

Instead of step 3 you can also restore your own `.tgz` from section 1.2 (the restore command is in section 5 of [database.md](database.md)); it holds the same data. Step 4 is needed either way.

v0.6 can open the migrated database itself, and it switches the file back from WAL by itself, but keep that for emergencies: v0.6 does not know the v0.7 rules, so every user sees every bank again, only administrators can manage banks, and practice records look like exams. When v0.7 starts again afterwards, it repairs what v0.6 left behind: banks v0.6 imported get an owner, and users v0.6 created get **All question banks** (section 6.6 of [database.md](database.md)).

To upgrade again later, put the v0.7 images back (tag `:latest`) and run `docker compose pull && docker compose up -d`. The migration runs again and writes a new copy.

### 1.5 What has been tested

With synthetic data only (`backend/tests/v065_bank_access_and_practice.rs`, `v07_wal_and_logs.rs`, `db_golden.rs` and `tests/integration/`):

- A schema-3 database migrates to schema v4 with a backup; every user keeps every bank, and the banks belong to the first administrator.
- If the backup cannot be written, the database is not migrated and not changed.
- A new database is in WAL mode; `XEXAM_SQLITE_JOURNAL=delete` keeps DELETE, and the next start without it is in WAL again with every record still there.
- A migration of a database in WAL mode backs up the changes that were still in the `-wal` file; the backup is one self-contained file.
- Opening an up-to-date database writes nothing (the database file and its `-wal` file stay the same, byte for byte).
- After `docker stop` the volume holds `xexam.db` and no `-wal` content.
- `reset-password` works while the server has the database open.
- Banks without an owner, stale bank assignments and users that an older version created after a rollback are repaired at the next start.

Not tested: a migration of **your own** database. Do it on a copy first if you want to be sure (as in section 5, "Try the new version on a copy").

## 2. Upgrading from v0.5 or v0.5.1 to v0.7

### 2.1 What changes

v0.6 introduced accounts and sign-in (schema v3); v0.7 adds question banks per user (schema v4). Coming from v0.5, both run on the **first start**, in one transaction, after one copy:

1. **Backup** to `/data/backups/xexam-schema2-<YYYYmmdd-HHMMSS>.db`, an ordinary schema-2 database that v0.5 can open. If it cannot be written, nothing changes and the backend does not start.
2. **Accounts.** The default administrator `admin` with the password `admin` is created, and it **receives everything you had**: every study record, the wrong-answer notebook and the settings.
3. **Schema v4** as in section 1.1: `admin` sees and owns every bank.
4. **WAL mode**, as in section 1.1.

From now on, every page needs a signed-in account. Each person gets their own records, notebook and settings, so add the other people under **User management** instead of sharing `admin`.

### 2.2 Upgrade

Use the commands of section 1.2, with `xexam-data-v05.tgz` as the backup name. A `docker-compose.yml` from v0.5 still pulls from `ghcr.io`; the new one pulls from Docker Hub (`docker.io/yuminspace/…`, public, no login). The project name and the volume `xexam_data` are the same, so your data stays.

The log names the copy (`…/backups/xexam-schema2-….db`) and says `Created the default administrator 'admin' (password 'admin'); sign in and choose a new password.`

### 2.3 Checks after the upgrade

- [ ] Open Xexam: the sign-in page appears and names the account `admin`. Sign in with `admin` / `admin` and choose your own password right away; until then anyone who can open the page can use the default password.
- [ ] `version` in `/api/info` is `0.7`, and `bankCount` is the same as before.
- [ ] **Question banks** lists the same banks; **Study records** lists your earlier exams, and the wrong-answer review has as many questions as before; **Settings** has your theme and languages.
- [ ] Add the other users and choose their question banks.

### 2.4 Rollback to v0.5

As in section 1.4, with the copy `xexam-schema2-….db` and the images `docker.io/yuminspace/xexam-backend:v0.5.1` and `docker.io/yuminspace/xexam-frontend:v0.5.1` (v0.5 itself is only on the private mirror `ghcr.io/yumin-space/`). Everything done since the upgrade is lost, the accounts included.

Do not run v0.5 on the migrated database: it knows no accounts, so its records would belong to nobody (v0.7 gives them to the first administrator at its next start), and every person would share one notebook again.

## 3. Upgrading from v0.4 to v0.5 and later

Today this upgrade goes straight to v0.7: the steps below, then those of sections 2.1 and 1.1, run in **one** transaction after **one** copy, `xexam-schema1-<YYYYmmdd-HHMMSS>.db`. You then sign in with `admin` / `admin` (section 2.3), and `admin` receives all your data.

### 3.1 What happens to your data

v0.5 added question banks, per-question exam detail and settings to the database (schema v2). The change is purely additive: nothing is rebuilt, and no question id changes. Full details: section 6 of [database.md](database.md).

On the **first start** of the new version on your data volume, the backend does this before it accepts any request:

1. **Backup.** It writes a copy of the current database to `backups/xexam-schema1-<YYYYmmdd-HHMMSS>.db` next to `xexam.db` (in Docker: `/data/backups/` in the volume `xexam_data`). The copy is an ordinary schema-1 database that v0.4 can open. An existing file is never overwritten. **If the copy cannot be written** (disk full, no permission), the backend refuses to start and leaves the database unchanged; the log says why.
2. **Migration**, in one transaction:
   - Everything you have becomes **one question bank** (the "legacy" bank). It is named after the file of your latest import, without `.xlsx` (`Question bank` if there was never an import).
   - **Question ids stay the same.** So the per-question statistics, the wrong-answer notebook and the exam history stay linked to their questions.
   - **Exam records are kept** and are attributed to that bank. They have no per-question detail, because v0.4 never stored the answers you gave: their detail page shows the score but no answers, and offers no "retake" buttons.
   - The import history is kept and attributed to that bank.

Later starts check, read-only, that nothing needs fixing, and write nothing.

What else you will notice:

- **The interface starts in English.** v0.4 was in Chinese. Change it under **Settings → Interface language**, or on the sign-in page. The setting is stored in the database, so you set it once for every browser and device.
- The theme follows the system (light or dark) until you choose one in **Settings**.
- The question language (Japanese / Chinese / English on the exam page) used to be remembered by the browser. The first time a browser opens the new version, it moves that choice to the server, if the server still has the default (Japanese).
- An unfinished exam saved in the browser is kept and can be continued (it goes to the first administrator who signs in on that browser).
- The database now keeps the newest 500 records per user instead of 60.

### 3.2 Upgrade

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

The log should contain a line like `Upgraded the database to schema 4. A copy of the previous version was saved first: /data/backups/xexam-schema1-20261110-140501.db (…)`. The program writes that copy itself; the `.tgz` from step 1 is your own second copy, outside the volume.

For a source build: `git pull && docker compose -f docker/docker-compose.build.yml up -d --build`, from the repository root. The compose files are now in `docker/`: before that command, move a `.env` at the repository root to `docker/.env` (Compose no longer reads it at the root), and add `-f docker/docker-compose.build.yml` to the later commands, such as `logs backend`. Without Docker, the copy goes to `backups/` inside the data directory (`--data` or `XEXAM_DATA`).

### 3.3 Checks after the upgrade

- [ ] Sign in with `admin` / `admin` and choose your own password (section 2.3).
- [ ] `version` in `/api/info` is the version you installed (`0.7` today), and `bankCount` (the number of active questions) is the same as before.
- [ ] **Question banks** lists one bank with all your sheets and the same number of questions as before.
- [ ] **Study records** lists your earlier exams, and **Study records → Wrong-answer review** has the same number of questions as the wrong-answer notebook had before.
- [ ] (Optional) Choose **Update a bank**, pick that bank and upload the same Excel file as last time. The preview should say that the file has exactly the same questions and there is nothing to import. Then cancel. This proves the question ids did not change.
- [ ] Answer a few questions and submit. The new record shows each question with your answer.

### 3.4 Rollback to v0.4

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

Instead of step 3 you can also restore your own `.tgz` from section 3.2 (the restore command is in section 5 of [database.md](database.md)); it holds the same data. Step 4 is needed either way.

**Do not run v0.4 on the migrated database** instead of restoring the copy. v0.4 can open the file, but it sees all banks as one: its next import would retire the questions of every other bank, it cuts the exam records back to 60, and it knows nothing about accounts. If it happens anyway, the next start of the new version attaches the questions v0.4 added to the legacy bank (section 6.6 of [database.md](database.md)).

To upgrade again later, put the new `docker-compose.yml` back (Docker Hub images, tag `:latest`) and run `docker compose pull && docker compose up -d`. The migration runs again and writes a new copy.

### 3.5 What has been tested

With synthetic data only (`backend/tests/v05_banks_records_settings.rs`, `v06_users_and_sign_in.rs` and `db_golden.rs`):

- `sample.db`, a database written by the old Python implementation, migrates to the current schema keeping every question id, statistic, exam record and import. Its backup still reports schema 1 and holds exactly the original rows.
- A schema-1 database goes to the current schema in one step, with one schema-1 backup.
- If the backup cannot be written, the database is not migrated and not changed.
- Opening a migrated database again does not change a single byte of the file.
- Rows written by an older binary into a migrated file are attached to the legacy bank at the next start.
- A new database and a migrated one have the same structure.

Not tested: a migration of **your own** database. Do it on a copy first if you want to be sure (as in section 5, "Try the new version on a copy").

## 4. Python to Rust (v0.3): principles

1. **Never touch the original.** Back up first, try on a copy first, and switch only after you have confirmed everything is fine.
2. **You could always go back.** Up to v0.4 both implementations read and write the same file with the same schema (a round trip in both directions was rehearsed with synthetic data during the migration, see section 8). Since v0.5 the schema has new tables and columns; going back from a newer version means restoring the copy written before the migration (section 3.4).
3. **The old implementation is not lost.** The old code is at the Git tag `python-v0.2`; the old image `ghcr.io/yumin-space/xexam:v0.2.2` (along with `:v0.2` and `:v0.2.1`) is still on GHCR (private: `docker login ghcr.io` to pull it; it is not on Docker Hub) and will not be overwritten by the new pipeline. The repository's main branch no longer contains any Python code.

## 5. Python to Rust: pre-upgrade checklist

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

  Open <http://localhost:8765> in a browser, sign in with `admin` / `admin`, and check that the number of questions, the wrong-answer notebook and the score history match the old version. (This also migrates the copy to the current schema and writes `backups/` into the temporary directory.) **Delete this temporary directory when you are done.**
- [ ] **Write down the old version's numbers**: `bankCount` from `/api/info`, the total number of questions on the home page, and the number of questions in the wrong-answer notebook. Compare them one by one after the upgrade.
- [ ] **Check `.env`**: you can keep the old values for `XEXAM_ALLOWED_HOSTS`, `XEXAM_BIND` and so on; the variable names have not changed.

## 6. Python to Rust: upgrade

```bash
# 1. Stop the old version (do not use down -v)
docker compose stop

# 2. Remove the old container (only the container, not the volume). The old version's service and container are both named xexam, while the new ones are frontend / backend.
#    If you do not remove it, the old container stays in the project as an "orphan", and its port mapping still holds 8765, so the new xexam-frontend cannot start because of the port conflict.
docker rm xexam

# 3. Put the new docker-compose.yml in place (it now has two services, pulled from Docker Hub without login), then pull and start (same volume xexam_data, same port)
docker compose pull
docker compose up -d

# 4. Check that both containers are healthy
docker compose ps
```

`docker rm xexam` removes only the **container**. It does not remove the data volume `xexam_data` (that would take `docker volume rm` or `down -v`; do not do either).

Upgrading from Python straight to the current version also runs the schema migrations of section 3, with their backup copy, and creates the account `admin` / `admin` that receives your data.

Checks after the upgrade:

- [ ] `version` in `/api/info` is the version you installed (`0.7` today), and `bankCount` is the same as before the upgrade.
- [ ] After signing in, the question count, the question count in the wrong-answer notebook and the score history are the same as before the upgrade.
- [ ] Answer a few questions and submit; the score appears in the history.
- [ ] (Optional) Upload the same Excel file again (**Update a bank**, the one bank you have): the preview should show no added, updated or retired questions (chips with a count of 0 are hidden) and say that the file has exactly the same questions, so there is nothing to import. This proves that the old and new parsers compute the **same question ids** for the same set of questions. If it shows a large number of "retired + added" entries instead, **cancel and roll back immediately** (section 7).

> ⚠️ The old and new versions use the same project name (`xexam`), the same volume (`xexam_data`) and the same port, so they **cannot run at the same time**. The old version's container is named `xexam`, while the new version's are `xexam-frontend` and `xexam-backend`, so during the upgrade you have to remove the old container as in step 2 above.

## 7. Python to Rust: rollback

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

**From v0.5 or later** the schema has new tables and columns (and since v0.7 the file is in WAL mode). First restore the schema-1 copy written before the migration, as in steps 1–3 of section 3.4, then start the old version as above.

If you also want the data to go back to the state before the upgrade (for example, you suspect the new version corrupted it):

```bash
docker compose stop
docker run --rm -v xexam_data:/data -v "$PWD:/backup" alpine sh -c 'rm -f /data/xexam.db-wal /data/xexam.db-shm && tar xzf /backup/xexam-data.tgz -C /data'
docker compose -f docker-compose.legacy.yml up -d
```

> The restore overwrites `/data` with the backup, so every change made in the new version after the backup is lost. Do it only when you are sure you need it.

Rolling back without Docker: `git checkout python-v0.2` and run `python server.py` (requires Python 3.8+), with the data directory pointing at the same `xexam.db`.

## 8. Python to Rust: what has been rehearsed

The following was done during the migration (while the Python code was still in the repository), **using fictional data only**; no real database was read, printed or modified. The scripts that depended on running the old Python code live were removed along with that code; the conclusions continue to be guarded by the committed snapshot tests.

| Rehearsal | How it was done | Result | What guards it now |
|---|---|---|---|
| Round trip between old and new | Old Python creates the database → backup + `integrity_check` → Rust reads it on a **copy** (byte-for-byte identical to the old version) → Rust imports a second workbook and records scores → old Python reads back (identical to Rust) → old Python writes again → Rust reads again | 19 checks passed, including "the sha256 of the original is unchanged before and after" | `db_golden.rs`: `sample.db` (a database written by the old implementation) migrates to the current schema keeping every row, and the migrated file is byte-for-byte unchanged by any later open, read and close; the `sqlite_master` of a newly created database is the old implementation's text plus the later additions |
| Golden-response comparison | 172 responses from the old version compared item by item with the new backend | 0 unexplained differences, 23 recorded | `api_golden.rs` replays them on every CI run |
| Security probes | 340 malicious requests sent to both the old and the new version, in 3 configurations | 0 unexplained differences, 51 recorded (most of them are cases where the new version is stricter) | The probe script has been removed; the guards and upload checks are tested by `api_*.rs`, `parser_hardening.rs` and `nginx_check.py` |
| Parser differential | 49 synthetic workbooks, with the output of the old parser and the Rust parser compared item by item; 85 question-id vectors | All identical | `parser_golden.rs` and `id_vectors.rs` run on every CI run |
| Full stack | `tests/integration/stack_check.py`: bring up the stack, upload / import / submit, the data is still there after the container is deleted and the stack is recreated on the same volume, `docker stop` timing, non-root | 34 checks passed at the time (the script has grown since) | Runs on every CI run |

## 9. Python to Rust: intentional differences from the old version

The full table, with reasons, is in section 5 of [api.md](api.md). Every entry is recorded in `tests/baseline/golden/allowed_differences_rust.json` and `allowed_probe_differences_rust.json`, and a difference that is not recorded makes the tests fail. The only ones visible to users are:

| Item | Change |
|---|---|
| Architecture | One container becomes two (nginx + backend); the images are named `xexam-frontend` / `xexam-backend`; `XEXAM_BIND` now points at Nginx |
| Stricter upload checks | A DOCTYPE in any encoding / extension is rejected; duplicate ZIP entry names are rejected; the number of entries, the XML depth and the number of nodes are capped |
| Error messages | All other Chinese messages are kept verbatim; in two places that used to echo the text of a library exception, a fixed Chinese message is used instead |
| Grading | Still done in the browser, not moved to the server (the spec recommends moving it; here it is intentionally not moved, for the reasons see Q3 in [migration-plan.md](../zh-CN/history/migration-plan.md)) |
| iPhone | Requires iOS 15.4 or later (build target `safari15`) |
| No longer provided | The backend no longer serves static files (Nginx does); the `Server` response header |

The API changes of later versions are listed in [api.md](api.md).

## 10. Python to Rust: what has not been verified

- A real Cloudflare Tunnel / Access, a real iPhone, and running on real ARM hardware (arm64 has only been built and started under QEMU).
- A rehearsal with a copy of **your own real database**: I do not have it and should not read it; please do it on your own machine following section 5.

## 11. Upgrading from v0.1

From v0.2 on, the database file is named `xexam.db`. If the data directory contains another `.db` file but no `xexam.db`, the backend's startup log warns you; rename that file to `xexam.db` and start again (the old file is neither deleted nor read). The old `progress.json` is checked once on every startup, and it is migrated only if the database has no study records yet and no migration has happened before; it is migrated only once and never overwrites existing records.
