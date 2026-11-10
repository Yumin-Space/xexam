**English** | [日本語](../ja/migration.md) | [简体中文](../zh-CN/migration.md)

# Migration and rollback

> This document covers every upgrade path, newest first:
>
> - **v0.4 → v0.5** (section 1): the database moves to schema v2, automatically, after a backup copy is written.
> - **Python v0.2.x → Rust + TypeScript + Nginx (v0.3)** (sections 2–8): the database file and its schema did not change then; the switch meant "swapping the program", not "moving data". Basis for that decision: [migration-plan.md](../zh-CN/history/migration-plan.md). Risks: [risk-assessment.md](../zh-CN/history/risk-assessment.md).
> - **v0.1 → later** (section 9): the database file name.

## 1. Upgrading from v0.4 to v0.5

### 1.1 What happens to your data

v0.5 adds question banks, per-question exam detail and settings to the database (schema v2). The change is purely additive: nothing is rebuilt, and no question id changes. Full details: section 6 of [database.md](database.md).

On the **first start** of v0.5 on your data volume, the backend does this before it accepts any request:

1. **Backup.** It writes a copy of the current database to `backups/xexam-schema1-<YYYYmmdd-HHMMSS>.db` next to `xexam.db` (in Docker: `/data/backups/` in the volume `xexam_data`). The copy is an ordinary schema-1 database that v0.4 can open. An existing file is never overwritten. **If the copy cannot be written** (disk full, no permission), the backend refuses to start and leaves the database unchanged; the log says why.
2. **Migration**, in one transaction:
   - Everything you have becomes **one question bank** (the "legacy" bank). It is named after the file of your latest import, without `.xlsx` (`Question bank` if there was never an import).
   - **Question ids stay the same.** So the per-question statistics, the wrong-answer notebook and the exam history stay linked to their questions.
   - **Exam records are kept** and are attributed to that bank. They have no per-question detail, because v0.4 never stored the answers you gave: their detail page shows the score but no answers, and offers no "retake" buttons.
   - The import history is kept and attributed to that bank.

Later starts check, read-only, that nothing needs fixing, and write nothing.

What else you will notice:

- **The interface starts in English.** v0.4 was in Chinese. Change it under **Settings → Interface language**. The setting is stored in the database, so you set it once for every browser and device.
- The theme follows the system (light or dark) until you choose one in **Settings**.
- The question language (Japanese / Chinese / English on the exam page) used to be remembered by the browser. The first time a browser opens v0.5, it moves that choice to the server, if the server still has the default (Japanese).
- An unfinished exam saved in the browser is kept and can be continued.
- The database now keeps the newest 500 exams instead of 60.

### 1.2 Upgrade

Nothing changes in `.env`, the port or the volume (see section 6 of [deployment.md](deployment.md)). Only the image source in `docker-compose.yml` is new since v0.5.1: it pulls from Docker Hub (`docker.io/yuminspace/…`, public, no login) instead of `ghcr.io`. Replace your `docker-compose.yml` with the new one, or change its two `image:` lines to `docker.io/yuminspace/xexam-backend:latest` and `docker.io/yuminspace/xexam-frontend:latest`. The project name and the volume `xexam_data` are unchanged, so your data stays.

```bash
# 1. Back up the volume (stop first; the backup file holds your whole database, do not commit it)
docker compose stop
docker run --rm -v xexam_data:/data -v "$PWD:/backup" alpine tar czf /backup/xexam-data-v04.tgz -C /data .

# 2. With the new docker-compose.yml in place (Docker Hub images), pull v0.5 and start (do not use down -v)
docker compose pull
docker compose up -d

# 3. Look for the upgrade message and the name of the copy
docker compose logs backend
```

The log should contain a line like `Upgraded the database to schema 2. A copy of the previous version was saved first: /data/backups/xexam-schema1-20261110-140501.db (…)`. The program writes that copy itself; the `.tgz` from step 1 is your own second copy, outside the volume.

For a source build: `git pull && docker compose -f docker/docker-compose.build.yml up -d --build`, from the repository root. The compose files are now in `docker/`: before that command, move a `.env` at the repository root to `docker/.env` (Compose no longer reads it at the root), and add `-f docker/docker-compose.build.yml` to the later commands, such as `logs backend`. Without Docker, the copy goes to `backups/` inside the data directory (`--data` or `XEXAM_DATA`).

### 1.3 Checks after the upgrade

- [ ] `version` in `/api/info` is `0.6`, and `bankCount` (the number of active questions) is the same as before.
- [ ] **Question banks** lists one bank with all your sheets and the same number of questions as before.
- [ ] **Study records** lists your earlier exams, and **Study records → Wrong-answer review** has the same number of questions as the wrong-answer notebook had before.
- [ ] (Optional) Choose **Update a bank**, pick that bank and upload the same Excel file as last time. The preview should say that the file has exactly the same questions and there is nothing to import. Then cancel. This proves the question ids did not change.
- [ ] Answer a few questions and submit. The new record shows each question with your answer.

### 1.4 Rollback to v0.4

Rolling back means **restoring the copy written before the migration** and running the v0.4 images. Everything done in v0.5 after the migration is lost: new banks, imports, exam records and settings.

```bash
# 1. Stop v0.5 (do not use down -v)
docker compose stop

# 2. Find the copy. After a normal stop there is no xexam.db-journal in /data.
docker run --rm -v xexam_data:/data alpine ls -l /data /data/backups

# 3. Put the copy back as xexam.db (use the file name from step 2)
docker run --rm -v xexam_data:/data alpine cp /data/backups/xexam-schema1-20261110-140501.db /data/xexam.db

# 4. v0.4 is only on the private GHCR mirror (Docker Hub starts with v0.5.1): run docker login ghcr.io, then set the two images in docker-compose.yml to
#      image: ghcr.io/yumin-space/xexam-backend:v0.4
#      image: ghcr.io/yumin-space/xexam-frontend:v0.4
docker compose up -d
```

For a source build, check out the tag instead: `git checkout v0.4 && docker compose -f docker-compose.build.yml up -d --build` (at that tag the compose files are still at the repository root, and Compose reads `.env` from there: if you have a `docker/.env`, copy it to the repository root first).

Instead of step 3 you can also restore your own `.tgz` from section 1.2 (the restore command is in section 5 of [database.md](database.md)); it holds the same data. Step 4 is needed either way.

**Do not run v0.4 on the migrated database** instead of restoring the copy, at least not once you have more than one bank. v0.4 can open the file, but it sees all banks as one: its next import would deactivate the questions of every other bank, and it cuts the exam records back to 60. If it happens anyway, v0.5 attaches the questions v0.4 added to the legacy bank at its next start (section 6.2 of [database.md](database.md)).

To upgrade again later, put the new `docker-compose.yml` back (Docker Hub images, tag `:latest`) and run `docker compose pull && docker compose up -d`. The migration runs again and writes a new copy.

### 1.5 What has been tested

With synthetic data only (`backend/tests/v05_banks_records_settings.rs` and `db_golden.rs`):

- `sample.db`, a database written by the old Python implementation, migrates to schema v2 keeping every question id, statistic, exam record and import. Its backup still reports schema 1 and holds exactly the original rows.
- If the backup cannot be written, the database is not migrated and not changed.
- Opening a migrated database again does not change a single byte of the file.
- Rows written by an older binary into a schema-v2 file are attached to the legacy bank at the next start.
- A new database and a migrated one have the same structure.

Not tested: a migration of **your own** database. Do it on a copy first if you want to be sure (as in section 3, "Try the new version on a copy").

## 2. Python to Rust (v0.3): principles

1. **Never touch the original.** Back up first, try on a copy first, and switch only after you have confirmed everything is fine.
2. **You could always go back.** Up to v0.4 both implementations read and write the same file with the same schema (a round trip in both directions was rehearsed with synthetic data during the migration, see section 6). Since v0.5 the schema has new tables and columns; going back from v0.5 means restoring the copy written before the migration (section 1.4).
3. **The old implementation is not lost.** The old code is at the Git tag `python-v0.2`; the old image `ghcr.io/yumin-space/xexam:v0.2.2` (along with `:v0.2` and `:v0.2.1`) is still on GHCR (private: `docker login ghcr.io` to pull it; it is not on Docker Hub) and will not be overwritten by the new pipeline. The repository's main branch no longer contains any Python code.

## 3. Python to Rust: pre-upgrade checklist

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

  Open <http://localhost:8765> in a browser and check that the number of questions, the wrong-answer notebook and the score history match the old version. (Since v0.5 this also migrates the copy to schema v2 and writes `backups/` into the temporary directory.) **Delete this temporary directory when you are done.**
- [ ] **Write down the old version's numbers**: `bankCount` from `/api/info`, the total number of questions on the home page, and the number of questions in the wrong-answer notebook. Compare them one by one after the upgrade.
- [ ] **Check `.env`**: you can keep the old values for `XEXAM_ALLOWED_HOSTS`, `XEXAM_BIND` and so on; the variable names have not changed.

## 4. Python to Rust: upgrade

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

Upgrading from Python straight to v0.5 also runs the schema-v2 migration of section 1, with its backup copy.

Checks after the upgrade:

- [ ] `version` in `/api/info` is the version you installed (`0.6` today), and `bankCount` is the same as before the upgrade.
- [ ] The question count, the question count in the wrong-answer notebook and the score history are the same as before the upgrade.
- [ ] Answer a few questions and submit; the score appears in the history.
- [ ] (Optional) Upload the same Excel file again (since v0.5: **Update a bank**, the one bank you have): the preview should show no added, updated or retired questions (chips with a count of 0 are hidden) and say that the file has exactly the same questions, so there is nothing to import. This proves that the old and new parsers compute the **same question ids** for the same set of questions. If it shows a large number of "retired + added" entries instead, **cancel and roll back immediately** (section 5).

> ⚠️ The old and new versions use the same project name (`xexam`), the same volume (`xexam_data`) and the same port, so they **cannot run at the same time**. The old version's container is named `xexam`, while the new version's are `xexam-frontend` and `xexam-backend`, so during the upgrade you have to remove the old container as in step 2 above.

## 5. Python to Rust: rollback

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

**From v0.5** the schema has new tables and columns. First restore the copy written before the migration, as in steps 1–3 of section 1.4, then start the old version as above.

If you also want the data to go back to the state before the upgrade (for example, you suspect the new version corrupted it):

```bash
docker compose stop
docker run --rm -v xexam_data:/data -v "$PWD:/backup" alpine tar xzf /backup/xexam-data.tgz -C /data
docker compose -f docker-compose.legacy.yml up -d
```

> The restore overwrites `/data` with the backup, so every change made in the new version after the backup is lost. Do it only when you are sure you need it.

Rolling back without Docker: `git checkout python-v0.2` and run `python server.py` (requires Python 3.8+), with the data directory pointing at the same `xexam.db`.

## 6. Python to Rust: what has been rehearsed

The following was done during the migration (while the Python code was still in the repository), **using fictional data only**; no real database was read, printed or modified. The scripts that depended on running the old Python code live were removed along with that code; the conclusions continue to be guarded by the committed snapshot tests.

| Rehearsal | How it was done | Result | What guards it now |
|---|---|---|---|
| Round trip between old and new | Old Python creates the database → backup + `integrity_check` → Rust reads it on a **copy** (byte-for-byte identical to the old version) → Rust imports a second workbook and records scores → old Python reads back (identical to Rust) → old Python writes again → Rust reads again | 19 checks passed, including "the sha256 of the original is unchanged before and after" | `db_golden.rs`: `sample.db` (a database written by the old implementation) migrates to schema v2 keeping every row, and the migrated file is byte-for-byte unchanged by any later open, read and close; the `sqlite_master` of a newly created database is the old implementation's text plus the v2 additions |
| Golden-response comparison | 172 responses from the old version compared item by item with the new backend | 0 unexplained differences, 23 recorded | `api_golden.rs` replays them on every CI run |
| Security probes | 340 malicious requests sent to both the old and the new version, in 3 configurations | 0 unexplained differences, 51 recorded (most of them are cases where the new version is stricter) | The probe script has been removed; the guards and upload checks are tested by `api_*.rs`, `parser_hardening.rs` and `nginx_check.py` |
| Parser differential | 49 synthetic workbooks, with the output of the old parser and the Rust parser compared item by item; 85 question-id vectors | All identical | `parser_golden.rs` and `id_vectors.rs` run on every CI run |
| Full stack | `tests/integration/stack_check.py`: bring up the stack, upload / import / submit, the data is still there after the container is deleted and the stack is recreated on the same volume, `docker stop` timing, non-root | 34 checks passed | Runs on every CI run |

## 7. Python to Rust: intentional differences from the old version

The full table, with reasons, is in section 5 of [api.md](api.md). Every entry is recorded in `tests/baseline/golden/allowed_differences_rust.json` and `allowed_probe_differences_rust.json`, and a difference that is not recorded makes the tests fail. The only ones visible to users are:

| Item | Change |
|---|---|
| Architecture | One container becomes two (nginx + backend); the images are named `xexam-frontend` / `xexam-backend`; `XEXAM_BIND` now points at Nginx |
| Stricter upload checks | A DOCTYPE in any encoding / extension is rejected; duplicate ZIP entry names are rejected; the number of entries, the XML depth and the number of nodes are capped |
| Error messages | All other Chinese messages are kept verbatim; in two places that used to echo the text of a library exception, a fixed Chinese message is used instead |
| Grading | Still done in the browser, not moved to the server (the spec recommends moving it; here it is intentionally not moved, for the reasons see Q3 in [migration-plan.md](../zh-CN/history/migration-plan.md)) |
| iPhone | Requires iOS 15.4 or later (build target `safari15`) |
| No longer provided | The backend no longer serves static files (Nginx does); the `Server` response header |

The API changes of v0.5 are listed under "Changes in v0.5" in [api.md](api.md).

## 8. Python to Rust: what has not been verified

- A real Cloudflare Tunnel / Access, a real iPhone, and running on real ARM hardware (arm64 has only been built and started under QEMU).
- A rehearsal with a copy of **your own real database**: I do not have it and should not read it; please do it on your own machine following section 3.

## 9. Upgrading from v0.1

From v0.2 on, the database file is named `xexam.db`. If the data directory contains another `.db` file but no `xexam.db`, the backend's startup log warns you; rename that file to `xexam.db` and start again (the old file is neither deleted nor read). The old `progress.json` is checked once on every startup, and it is migrated only if the database has no study records yet and no migration has happened before; it is migrated only once and never overwrites existing records.
