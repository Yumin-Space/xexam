**English** | [日本語](migration.ja.md) | [简体中文](migration.zh-CN.md)

# Migration and rollback

> Switching from the Python version (v0.2.x) to the Rust + TypeScript + Nginx version (v0.3). **The database file and its schema have not changed at all**, so the switch means "swapping the program", not "moving data".
> Basis for the decision: [migration-plan.md](migration-plan.md). Risks: [risk-assessment.md](risk-assessment.md).

## 1. Principles

1. **Never touch the original.** Back up first, try on a copy first, and switch only after you have confirmed everything is fine.
2. **You can always go back.** Both implementations read and write the same file with the same schema (a round trip in both directions was rehearsed with synthetic data during the migration, see §5).
3. **The old implementation is not lost.** The old code is at the Git tag `python-v0.2`; the old image `ghcr.io/yumin-space/xexam:v0.2.2` (along with `:v0.2` and `:v0.2.1`) is still on GHCR and will not be overwritten by the new pipeline. The repository's main branch no longer contains any Python code.

## 2. Pre-upgrade checklist

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

  Open <http://localhost:8765> in a browser and check that the number of questions, the wrong-answer notebook and the score history match the old version. **Delete this temporary directory when you are done.**
- [ ] **Write down the old version's numbers**: `bankCount` from `/api/info`, the total number of questions on the home page, and the number of questions in the wrong-answer notebook. Compare them one by one after the upgrade.
- [ ] **Check `.env`**: you can keep the old values for `XEXAM_ALLOWED_HOSTS`, `XEXAM_BIND` and so on; the variable names have not changed.

## 3. Upgrade

```bash
# 1. Stop the old version (do not use down -v)
docker compose stop

# 2. Remove the old container (only the container, not the volume). The old version's service and container are both named xexam, while the new ones are frontend / backend.
#    If you do not remove it, the old container stays in the project as an "orphan", and its port mapping still holds 8765, so the new xexam-frontend cannot start because of the port conflict.
docker rm xexam

# 3. Put the new docker-compose.yml in place (it now has two services), then pull and start (same volume xexam_data, same port)
docker compose pull
docker compose up -d

# 4. Check that both containers are healthy
docker compose ps
```

`docker rm xexam` removes only the **container**. It does not remove the data volume `xexam_data` (that would take `docker volume rm` or `down -v`; do not do either).

Checks after the upgrade:

- [ ] `version` in `/api/info` is `0.4`, and `bankCount` is the same as before the upgrade.
- [ ] The question count on the home page, the question count in the wrong-answer notebook and the score history are the same as before the upgrade.
- [ ] Answer a few questions and submit; the score appears in the history.
- [ ] (Optional) Upload the same Excel file again: the preview should show `新增 0、更新 0、下架 0` ("0 added, 0 updated, 0 deactivated"). This proves that the old and new parsers compute the **same question ids** for the same set of questions. If it shows a large number of "deactivated + added" entries instead, **roll back immediately** (§4).

> ⚠️ The old and new versions use the same project name (`xexam`), the same volume (`xexam_data`) and the same port, so they **cannot run at the same time**. The old version's container is named `xexam`, while the new version's are `xexam-frontend` and `xexam-backend`, so during the upgrade you have to remove the old container as in step 2 above.

## 4. Rollback

You can go back to the Python version at any time, **with no data loss**. The old version's compose file is no longer on the main branch; get it from the tag:

```bash
# 1. Stop the new version (do not use down -v)
docker compose stop

# 2. Get the old version's compose file and start the old version (it pulls ghcr.io/yumin-space/xexam:latest; to pin a version, change :latest to :v0.2.2)
git show python-v0.2:docker-compose.yml > docker-compose.legacy.yml
docker compose -f docker-compose.legacy.yml up -d
```

Note that the image tag in the old compose file is `:latest`, **and `ghcr.io/yumin-space/xexam:latest` is no longer updated after this release** (the new images use the two new names `xexam-frontend` / `xexam-backend`), so it still points to the last Python version. To be safe, write `:v0.2.2` explicitly.

The old version keeps using the same `xexam_data` volume. It can read everything the new version wrote (newly imported questions, new scores), because the new backend does not change the schema, does not change `journal_mode`, does not add tables or columns, does not set `user_version`, and leaves no `-wal` / `-shm` files behind.

If you also want the data to go back to the state before the upgrade (for example, you suspect the new version corrupted it):

```bash
docker compose stop
docker run --rm -v xexam_data:/data -v "$PWD:/backup" alpine tar xzf /backup/xexam-data.tgz -C /data
docker compose -f docker-compose.legacy.yml up -d
```

> The restore overwrites `/data` with the backup, so every change made in the new version after the backup is lost. Do it only when you are sure you need it.

Rolling back without Docker: `git checkout python-v0.2` and run `python server.py` (requires Python 3.8+), with the data directory pointing at the same `xexam.db`.

## 5. What has been rehearsed

The following was done during the migration (while the Python code was still in the repository), **using fictional data only**; no real database was read, printed or modified. The scripts that depended on running the old Python code live were removed along with that code; the conclusions continue to be guarded by the committed snapshot tests.

| Rehearsal | How it was done | Result | What guards it now |
|---|---|---|---|
| Round trip between old and new | Old Python creates the database → backup + `integrity_check` → Rust reads it on a **copy** (byte-for-byte identical to the old version) → Rust imports a second workbook and records scores → old Python reads back (identical to Rust) → old Python writes again → Rust reads again | 19 checks passed, including "the sha256 of the original is unchanged before and after" | `db_golden.rs`: `sample.db` (a database written by the old implementation) is byte-for-byte unchanged after being opened, read and closed; the `sqlite_master` of a newly created database is identical to the old implementation's, character for character |
| Golden-response comparison | 172 responses from the old version compared item by item with the new backend | 0 unexplained differences, 23 recorded | `api_golden.rs` replays them on every CI run |
| Security probes | 340 malicious requests sent to both the old and the new version, in 3 configurations | 0 unexplained differences, 51 recorded (most of them are cases where the new version is stricter) | The probe script has been removed; the guards and upload checks are tested by `api_*.rs`, `parser_hardening.rs` and `nginx_check.py` |
| Parser differential | 49 synthetic workbooks, with the output of the old parser and the Rust parser compared item by item; 85 question-id vectors | All identical | `parser_golden.rs` and `id_vectors.rs` run on every CI run |
| Full stack | `tests/integration/stack_check.py`: bring up the stack, upload / import / submit, the data is still there after the container is deleted and the stack is recreated on the same volume, `docker stop` timing, non-root | 34 checks passed | Runs on every CI run |

## 6. Intentional differences from the old version

The full table, with reasons, is in section 5 of [api.md](api.md). Every entry is recorded in `tests/baseline/golden/allowed_differences_rust.json` and `allowed_probe_differences_rust.json`, and a difference that is not recorded makes the tests fail. The only ones visible to users are:

| Item | Change |
|---|---|
| Architecture | One container becomes two (nginx + backend); the images are named `xexam-frontend` / `xexam-backend`; `XEXAM_BIND` now points at Nginx |
| Stricter upload checks | A DOCTYPE in any encoding / extension is rejected; duplicate ZIP entry names are rejected; the number of entries, the XML depth and the number of nodes are capped |
| Error messages | All other Chinese messages are kept verbatim; in two places that used to echo the text of a library exception, a fixed Chinese message is used instead |
| Grading | Still done in the browser, not moved to the server (the spec recommends moving it; here it is intentionally not moved, for the reasons see Q3 in [migration-plan.md](migration-plan.md)) |
| iPhone | Requires iOS 15.4 or later (build target `safari15`) |
| No longer provided | The backend no longer serves static files (Nginx does); the `Server` response header |

## 7. Upgrading from v0.1

From v0.2 on, the database file is named `xexam.db`. If the data directory contains another `.db` file but no `xexam.db`, the backend's startup log warns you; rename that file to `xexam.db` and start again (the old file is neither deleted nor read). The old `progress.json` is checked once on every startup, and it is migrated only if the database has no study records yet and no migration has happened before; it is migrated only once and never overwrites existing records.

## 8. What has not been verified

- A real Cloudflare Tunnel / Access, a real iPhone, and running on real ARM hardware (arm64 has only been built and started under QEMU).
- A rehearsal with a copy of **your own real database**: I do not have it and should not read it; please do it on your own machine following §2.
