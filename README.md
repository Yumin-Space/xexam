**English** | [日本語](README/README.ja.md) | [简体中文](README/README.zh-CN.md)

<!-- The release badge shows RELEASE_VERSION from VERSION, the last published release. Do not edit it by hand: the release job of .github/workflows/ci.yml writes it (scripts/set-release-version.py) after the images and the GitHub Release are published. GitHub rounds an image's corners only when it has both width and height, so every badge carries its exact size (in px, from shields.io). -->
<p>
  <a href="../../releases/latest"><img width="145" height="28" alt="Release v0.7.5" src="https://img.shields.io/badge/release-v0.7.5-green?style=for-the-badge"></a>
</p>

<p align="center">
  <img width="120" height="120" alt="Xexam logo" src="frontend/public/icon.svg">
</p>

<p align="center">
  <a href="https://hub.docker.com/r/yuminspace/xexam-frontend"><img width="360" height="28" alt="Docker image yuminspace/xexam-frontend" src="https://img.shields.io/badge/Docker%20Image-yuminspace%2Fxexam--frontend-blue?style=for-the-badge"></a>
  <a href="https://hub.docker.com/r/yuminspace/xexam-frontend"><img width="150" height="28" alt="Docker stars" src="https://img.shields.io/docker/stars/yuminspace/xexam-frontend?style=for-the-badge"></a>
  <a href="https://hub.docker.com/r/yuminspace/xexam-frontend"><img width="148" height="28" alt="Docker pulls" src="https://img.shields.io/docker/pulls/yuminspace/xexam-frontend?style=for-the-badge"></a>
</p>
<p align="center">
  <a href="https://hub.docker.com/r/yuminspace/xexam-backend"><img width="352" height="28" alt="Docker image yuminspace/xexam-backend" src="https://img.shields.io/badge/Docker%20Image-yuminspace%2Fxexam--backend-blue?style=for-the-badge"></a>
  <a href="https://hub.docker.com/r/yuminspace/xexam-backend"><img width="150" height="28" alt="Docker stars" src="https://img.shields.io/docker/stars/yuminspace/xexam-backend?style=for-the-badge"></a>
  <a href="https://hub.docker.com/r/yuminspace/xexam-backend"><img width="148" height="28" alt="Docker pulls" src="https://img.shields.io/docker/pulls/yuminspace/xexam-backend?style=for-the-badge"></a>
</p>

# Xexam

Xexam is a self-hosted app for practising exam questions in the browser. You import your own Excel question banks, then take exams or practise with them, and review the questions you got wrong. It works on computers and on phones (iPhone).

![The exam page: a hotspot question in English, the English / 中文 / 日本語 switch and the answer sheet with Submit](docs/en/screenshots/desktop-2-quiz.png)

The repository contains **no questions**. Xexam only provides the tools to import, organize and practise questions; your question banks are your own Excel files and are never committed (`.gitignore` excludes them). The screenshots use made-up demo banks, not content from any real exam.

| | |
|---|---|
| Current version | **v0.7.5** (see the [changelog](#changelog) or the repository's [Releases](../../releases) page; in the app: menu → **Xexam v0.7.5**) |
| Images | On Docker Hub (public, no login needed): `docker.io/yuminspace/xexam-frontend` and `docker.io/yuminspace/xexam-backend`, with tags `latest` / `v0.7.5` / `sha-<commit>` (the `sha-` tags are for troubleshooting). Mirror: `ghcr.io/yumin-space/xexam-frontend` and `ghcr.io/yumin-space/xexam-backend` (private, same tags) |
| CPU architecture | `linux/amd64`, `linux/arm64` (the right one is picked automatically under the same tag) |
| Port | `8765` (bound to the local machine only by default) |
| License | [MIT](LICENSE) |

---

## Contents

- [What it does](#what-it-does)
- [Quick start](#quick-start)
- [Configuration](#configuration)
- [Upgrading](#upgrading)
- [Documentation](#documentation)
- [License](#license)
- [Changelog](#changelog)

---

## What it does

- **Exams**: nothing is scored until you submit. Then you see your score and each wrong question (or all of them) with your answer, the correct answer, the explanation and the memory aid. You can jump between questions, mark them to revisit, and save and exit to continue later.
- **Practice**: every answer is checked right away and shows the correct answer and the explanation.
- **Your own question banks**: every Excel workbook you import is its own bank, and each worksheet a group of questions you can pick. Workbooks use the six-column layout: the question and the answer in English, and if you like in Chinese and Japanese too. A preview shows what an import will change before anything is saved.
- **Three languages**: the interface in English, Chinese or Japanese, and every question, answer and explanation switchable between **English**, **中文** and **日本語**.
- **Study records and a wrong-answer notebook**: every exam and practice is kept with its answers; questions you got wrong stay in the notebook until you answer them right twice in a row.
- **Several people, one Xexam**: everyone signs in and has their own records and settings. Administrators decide who sees which question bank and who may import banks (bank managers), and can follow the backend's **System log**.
- **Desktop and phone**: a slim header with a menu, pages centered on wide screens, touch targets of at least 44 px on phones.

How each page works, the Excel format and the import rules: [docs/en/usage.md](docs/en/usage.md).

---

## Quick start

You need a machine with **Docker** (including Compose v2), on Windows, macOS or Linux, **x86 or ARM**, and a web browser.

**1. Download [`docker-compose.yml`](docker/docker-compose.yml)** (in the `docker/` folder of this repository) into a new folder, for example `xexam`. It is the only file you need. The easiest way: open the file on GitHub and click **Download raw file**. Or on the command line:

```bash
curl -fLO https://raw.githubusercontent.com/Yumin-Space/xexam/main/docker/docker-compose.yml
```

The repository is private: to download the file with `curl` (or to `git clone` the repository), ask the owner to add your GitHub account as a collaborator, create a classic personal access token with the scope `repo` (GitHub → **Settings** → **Developer settings** → **Personal access tokens** → **Tokens (classic)**) and add `-H "Authorization: token YOUR_TOKEN"` after `-fLO`. Or ask the owner to send you the file. In Windows PowerShell, type `curl.exe` instead of `curl`. The images themselves are public on Docker Hub: pulling them needs no account and no `docker login`.

**2. Start.** In the folder that contains `docker-compose.yml`, run:

```bash
docker compose up -d
```

This pulls the two images and starts **two containers**, `xexam-frontend` (Nginx, the only one reachable from outside) and `xexam-backend` (Rust, on a private network).

**3. Sign in.** Open <http://localhost:8765> in a browser on the same machine and sign in with the username `admin` and the password `admin`. Xexam then asks you to choose your own password: do this right away, because until then anyone who can open the page can use the default password.

**4. Import a question bank and take an exam.** Open the menu (the three-line button at the top right), choose **Question banks** and import your Excel workbook, then start an exam on **Home**: see [Your first exam](docs/en/usage.md#1-your-first-exam). The interface starts in English; switch it to Chinese or Japanese under **Settings** → **Language**.

| Action | Command |
|---|---|
| Update to the latest version | `docker compose pull`, then `docker compose up -d` (read [Upgrading](#upgrading) first) |
| Pin to a specific version | Change `:latest` to `:v0.7.5` for both images in `docker-compose.yml`, then run `docker compose up -d` |
| Check status | `docker compose ps` (both containers should be `healthy`) |
| Stop / start again | `docker compose stop` / `docker compose start` |
| View logs | `docker compose logs -f backend` (administrators can also open **System log** in the menu) |
| See which version and architecture are running | Open <http://localhost:8765/api/info> in a browser |

> ⚠️ **Do not run `docker compose down -v`**: `-v` also deletes the data volume `xexam_data`, which holds your question banks, users, study records and settings.

- **From a phone or another computer**, or **from the internet through Cloudflare Tunnel**: section 4 of [docs/en/deployment.md](docs/en/deployment.md).
- **Building the images from source** (when you change the code): `docker compose -f docker/docker-compose.build.yml up -d --build` in a clone, see section 2 of [docs/en/deployment.md](docs/en/deployment.md).
- **Backups**: stop Xexam and pack the volume; the commands are in section 5 of [docs/en/database.md](docs/en/database.md).

---

## Configuration

The defaults work on one computer without any configuration. To change something, create a `.env` file next to `docker-compose.yml` (in a clone: `docker/.env`; [`.env.example`](docker/.env.example) is a template), then run `docker compose up -d` again. You do not need to edit the compose file itself.

| Name in `.env` | Default | Purpose |
|---|---|---|
| `XEXAM_BIND` | `127.0.0.1:8765` | The address and port to listen on, written as `IP:port`. By default only this machine can connect; `0.0.0.0:8765` = other machines on the network too. ⚠️ Only a port number (such as `8766`) = reachable on all network interfaces |
| `XEXAM_ALLOWED_HOSTS` | empty | Which domain names / IPs may be used to open Xexam, separated by commas, spaces or semicolons, for example `xexam.example.com,192.168.1.20` |
| `XEXAM_ALLOWED_ORIGINS` | empty | Needed only when a reverse proxy rewrites the Host to the origin server's address: your public domain, with `https://` |
| `XEXAM_ALLOW_ANY_HOST` | `0` | `1` = stop checking the Host (only when authentication is already in front) |
| `XEXAM_PARSE_TIMEOUT_SECS` | `30` | Maximum time to parse one Excel file (seconds) |
| `XEXAM_SQLITE_JOURNAL` | unset (WAL) | `delete` keeps SQLite's older DELETE journal, only for a data volume on unusual storage (a network filesystem, or a Windows folder bind-mounted into Docker) |
| `XEXAM_COOKIE_SECURE` | unset | `1` = the session cookie always gets `Secure`; only when Xexam is always reached over HTTPS |
| `XEXAM_LOG` | unset (`info`) | The backend's log filter, for example `debug`; empty means `info` |
| `TZ` | unset (the host's time zone) | Time zone of the times Xexam shows, for example `TZ=UTC` |

Times follow the host's time zone because the compose file mounts the host's `/etc/localtime`. On a host without that file, and on Docker Desktop (Windows, macOS), whose virtual machine usually runs on UTC, read section 3.1 of [docs/en/deployment.md](docs/en/deployment.md). Every option is explained in section 3 of that document.

---

## Upgrading

Every upgrade keeps the same two images, the same port and the same volume `xexam_data`, so your data stays where it is: `docker compose pull`, then `docker compose up -d`. When the database schema changes, the first start of the new version writes a backup copy of the database first, into `/data/backups/` in the volume, and then migrates it in one transaction. If the copy cannot be written (disk full, no permission), the backend refuses to start and leaves the database unchanged; `docker compose logs backend` says why.

### From v0.7 to v0.7.5

Nothing to do: `docker compose pull`, then `docker compose up -d`. The database, `docker-compose.yml` and `.env` stay as they are.

### From v0.6 to v0.7 (and v0.7.5)

> ⚠️ Since v0.7 **only the six-column Excel layout can be imported**, and every question that was imported from the three- or four-column layouts of earlier versions is **retired at every start**. All banks of v0.6 are in those layouts, so after the upgrade nothing can be practised until you update them with six-column workbooks. The banks, the study records and the statistics stay.

1. **Before the upgrade**, prepare a six-column version of each workbook (format: section 12 of [docs/en/usage.md](docs/en/usage.md)), and submit or discard any unfinished exam: its questions are retired, so it cannot be continued afterwards.
2. **Back up and upgrade**:

   ```bash
   # 1. Back up the volume (stop first; the backup file holds your whole database, do not commit it)
   docker compose stop
   docker run --rm -v xexam_data:/data -v "${PWD}:/backup" alpine tar czf /backup/xexam-data-v06.tgz -C /data .

   # 2. Replace docker-compose.yml with the new one (your .env stays)
   curl -fLO https://raw.githubusercontent.com/Yumin-Space/xexam/main/docker/docker-compose.yml

   # 3. Pull and start (do not use down -v), then read the upgrade lines
   docker compose pull
   docker compose up -d
   docker compose logs backend
   ```

   The new `docker-compose.yml` follows the host's time zone, limits the container logs and passes `XEXAM_SQLITE_JOURNAL`, `XEXAM_COOKIE_SECURE` and `XEXAM_LOG` from `.env`.
3. **What the first start does**, before it answers any request: it writes the copy `/data/backups/xexam-schema3-<YYYYmmdd-HHMMSS>.db`, migrates the database to schema v4 and switches it to WAL mode. Every existing user keeps every existing bank, with **Include banks imported later**; every bank belongs to the first administrator; every earlier record is an **Exam**; nobody is a bank manager yet. Then it retires the old-layout questions, with one warning per bank in the log, for example `retired 12 questions of bank "Kitchen basics": they use the old 3- / 4-column layout; import the six-column version of the workbook`.
4. **Update each old bank with its six-column file**: **Question banks** → **Import Excel** → **Update a bank**, pick the bank, upload the file and confirm. The bank keeps its name, its owner and the users who see it. Its questions are now recognised by their English text, so they start without answer history; the old questions stay retired and still show in the old records. Or import the six-column file as a **New bank**.
5. **Afterwards**: decide who is a **Bank manager** and which banks each user sees (a user you add from now on sees no bank until you choose some), and check that <http://localhost:8765/api/info> shows `"version": "0.7.5"`.

Rolling back to v0.6 means restoring the copy from `backups/`; the commands are in section 2.6 of [docs/en/migration.md](docs/en/migration.md).

### From older versions

From v0.5, v0.4 or the Python version v0.2.x the same steps apply, with a few more (sign-in arrives, the image source changed in v0.5.1, the old single container must be removed): see [docs/en/migration.md](docs/en/migration.md), which also has every rollback.

---

## Documentation

| Document | What is in it |
|---|---|
| [Usage](docs/en/usage.md) | Every page, exams and practice, study records, settings, the Excel format, importing, users and sign-in, the System log, the iPhone |
| [Deployment](docs/en/deployment.md) | Starting, configuration, time zone, phone and LAN access, Cloudflare Tunnel, health checks, logs, recovery |
| [Migration and rollback](docs/en/migration.md) | Every upgrade path with its checks and rollback |
| [Database](docs/en/database.md) | The tables, question ids, backup and restore, schema migrations |
| [Security](docs/en/security.md) | Threat model, Host / Origin guard, sign-in and locks, permissions, upload checks, Nginx limits |
| [Architecture](docs/en/architecture.md) | How frontend, Nginx and backend fit together, the directory layout, the request flows |
| [API](docs/en/api.md) ([OpenAPI](docs/en/openapi.yaml)) | Every endpoint, who may call it, its errors |
| [Development](docs/en/development.md) | Building and testing, the screenshot tool, CI and releases |

---

## License

Xexam is released under the [MIT License](LICENSE). Copyright (c) 2026 Yumin-Space.

The license covers the software in this repository. The repository contains no exam questions: the question banks you import are your own files, and you are responsible for making sure that you may use them.

---

## Changelog

Every update must add an entry here, and the **version number must match `CODE_VERSION` in the `VERSION` file in the repository root** (the automated build checks this and refuses to build on a mismatch). The latest image carries both the `latest` tag and the matching `vX.Y` tag.

The **About Xexam** page of the app shows these entries (they are built into the frontend), so keep them in simple Markdown: paragraphs, `- ` bullets, nested `  - ` bullets, `**bold**` and `` `code` ``. Links are shown as plain text there.

### v0.7.5 — 2026-11-11

**Fixes after v0.7: no more "Cannot reach the service" when several people use Xexam at once, real red for everything that deletes, green for every correct answer, and a shorter README with a separate usage guide. The database does not change.**

- **Too many requests**: Nginx now lets 30 API requests per second through from one address, with a burst of 120 (it was 10 with a burst of 30). Behind a tunnel such as Cloudflare Tunnel all users arrive from the tunnel’s one address and share this limit, and the System log page alone reads twice a second. When Nginx still refuses a request (429), the page retries a read twice by itself, then says **Too many requests**, instead of “Cannot reach the service” with a hint to sign in again. Uploads keep their own, smaller limit.
- **Red for deleting**: delete buttons, the confirm button of every delete dialog, error messages, form checks and the errors of the System log use GitHub’s red, in the light and the dark theme. Wrong answers stay orange, and warnings keep their own orange.
- **Green for every correct answer**: the **Correct** badge, the ticks of the result strip, the hotspot **Correct** tag and accuracies of 80 % and more in **Study records** are green like the correct options. On the **Your answer** line each letter of a multiple-choice answer is green or orange on its own, so a partly right answer shows both.
- **No question count in the public info**: `/api/info` no longer says how many questions there are (`bankCount` is gone); the route is public, and signed-in pages count the questions they may see themselves. It still reads the database, so the health check notices a broken one.
- **A shorter README**: installation, configuration, upgrading and this changelog. How to use Xexam, the Excel format and the import rules moved to `docs/en/usage.md`; everything else is in the other documents under `docs/`.
- **Screenshot tool**: `python tools/screenshots/retake.py` retakes every screenshot of the READMEs and docs, in all three languages, from a throw-away instance with made-up demo data, and cleans up after itself.
- **Fixes from the review of v0.7**:
  - an empty `XEXAM_LOG=` in `docker/.env` no longer hides every info and warning line: empty means `info`, and an invalid value falls back to `info` with one warning;
  - users that an older version added to a database of a v0.7 pre-release now get every bank when that database is converted, like the users of the upgrade;
  - on Home, a bank manager whose banks have only retired questions is sent to **Question banks**, where they can update their own banks;
  - the System log page gives up on a read after 10 seconds and tries again later;
  - the **Change** and **Unlock** buttons of User management have accessible names that contain their visible word;
  - the Excel format guide on the Question banks page matches the parser more closely (`HOTSPOT` is recommended, not required).
- Tests: backend 314 → 316, frontend 129 → 139.

### v0.7 — 2026-11-11

**Question banks per user, bank managers, a practice mode, the six-column Excel layout as the only one that can be imported, and a System log for administrators. The database moves to schema v4 automatically, after a backup copy is written, and now runs in WAL mode.**

- **Question banks per user**: in **User management** an administrator ticks, for each user, the existing question banks they see (with **Select all**) and, independently, **Include banks imported later**, which gives them every bank imported from then on. A new user sees no bank until some are chosen; every user from before v0.7 gets every existing bank and **Include banks imported later**, and administrators always see every bank. Taking a bank away keeps the user’s study records as history.
- **Bank managers**: a user with the new **Bank manager** permission (set in **User management**) can import new question banks and update or delete the banks they imported, but not the banks of administrators or other users. Users without it do not see **Question banks** in the menu, and opening the page directly shows **Not allowed**. Administrators see who uploaded each bank; when a user who uploaded banks is deleted, the banks move to the administrator who deletes them.
- **Practice mode**: **Start practice** next to **Start exam** on Home, with the same banks, sheets, count and order. Each question is checked at once (a single-choice question when you choose an option, the others with **Check answer**) and shows right or wrong, the correct answer, the explanation and the memory aid. A practice is saved as a study record of its own kind and counts in the wrong-answer notebook like an exam. **Study records** tags every record as **Exam** or **Practice** and can show only one kind, Home’s **Latest result** shows the latest record of either kind, and **Practise these** in the wrong-answer review now starts a practice.
- **Only the six-column Excel layout can be imported**: A English, B Chinese and C Japanese question; D English, E Chinese and F Japanese answer with explanation and memory aid. English is authoritative: the question type, the options, the hotspot items, the question identity and the answer key come from columns A and D. When the Chinese or Japanese answer differs from the English one, the row is imported with the English answer and the preview lists a warning. Files in the three- and four-column layouts are refused with a message, and every question imported from them earlier is retired at every start; the banks, the study records and the statistics stay. Update such a bank with the six-column version of its workbook: its questions are then recognised by their English text and start without answer history. The guide on the Question banks page explains only the six-column layout.
- **Answers and explanations in three languages**: every question card of the result page, an exam record and the wrong-answer review has its own **English** / **中文** / **日本語** switch for the question, the options, the explanation and the memory aid; the separate Chinese translation toggle is gone. Every question-language switch now says **English** / **中文** / **日本語** in every interface language.
- **Green and orange answers**: in the answers and explanations and in practice, the correct options are solid green and a wrongly chosen option is solid orange, with white text, in the light and the dark theme alike. Graded hotspot statements sit on a neutral background. On Home, **Start exam** is green and **Start practice** orange.
- **System log** (menu, administrators only): the last 2000 lines the backend logged since it started, read every half second, with **Pause**, **Jump to latest**, a level filter and a search. The lines name users by their username, never a typed name that has no account. Rejected requests and failed sign-ins that come in bursts are summed up once a minute, so they cannot push the other lines out. Older lines: `docker compose logs backend`.
- **SQLite WAL mode**: the database now runs in WAL mode (still with `synchronous=FULL`, so a committed change is not lost) and is checkpointed when the backend stops normally. A backup copied while the backend runs must include `xexam.db-wal` and `xexam.db-shm`, or stop the backend first. `XEXAM_SQLITE_JOURNAL=delete` in `docker/.env` keeps the old mode for unusual storage such as a network filesystem.
- **Time zone**: timestamps follow the host’s time zone by default (the compose file mounts the host’s `/etc/localtime`); a `TZ` set in `docker/.env` overrides it.
- **Configuration**: the compose files also pass `XEXAM_COOKIE_SECURE` and `XEXAM_LOG` from `docker/.env`. Log lines and the output of the command-line program are in English.
- **Sign-in and users**: the lock after wrong passwords is kept per account and cannot be worn away by guessing many other usernames; changing a password signs out your other sessions; **Unlock** is also offered while a user still has tries left, and the lock note is shown under the user’s row; User management counts each user’s study records (exams and practice).
- **Version file**: `VERSION` now holds `CODE_VERSION` (the version of the code, bumped together with this changelog) and `RELEASE_VERSION` (the last published release). The release job writes `RELEASE_VERSION` and the release badge at the top of the READMEs only after the images and the GitHub Release are published.
- **Database schema v4**: the new table `user_banks` and the new columns `users.new_banks`, `users.can_manage_banks`, `banks.owner_id` and `attempts.kind`; before the migration a copy is written to `<data dir>/backups/xexam-schema3-<date-time>.db`. A database written by a pre-release build of v0.7 is converted when it is opened, after a copy `xexam-schema4-<date-time>.db`.
- **New API**: `GET /api/logs` (administrators only); `banks`, `newBanks` and `canManageBanks` in `/api/users`, `/api/users/create` and `/api/users/update`; `kind` (`exam` or `practice`) in `/api/attempt`, `/api/records` and `/api/record`. `POST /api/upload` and `POST /api/import/confirm` refuse a three- or four-column workbook with 400. Only administrators see who owns a bank.
- Tests: backend 232 → 314, frontend 59 → 129.

### v0.6 — 2026-11-11

**Accounts and sign-in: every user has their own records, and administrators manage users and question banks. The database moves to schema v3 automatically, after a backup copy is written.**

- **Sign-in**: a standard sign-in page in the calm style of v0.5.1. On the first start (a new install or an upgrade) sign in with `admin` / `admin`; Xexam then asks for your own password, and the sign-in page and the startup log remind you until it is changed.
- **Users and roles**: administrators manage question banks and users; users take exams and see only their own study records, wrong-answer notebook, unfinished exam and settings. The question banks are shared. The data from before v0.6 belongs to the first administrator.
- **User management** (menu, administrators only): add, edit, reset the password of, disable / enable, delete and unlock users, with each user’s role, status (Normal, Disabled, Locked), created, last sign-in and number of exams. Users created or reset by an administrator choose their own password at the next sign-in.
- **Header**: your username (with **Admin** for administrators) left of the menu button; the menu shows only the pages of your role and ends with **Sign out**.
- **Settings**: a new **Account** section with **Change password**.
- **Wrong passwords**: 5 tries, then a lock of 1 minute; then 3 tries per round with locks of 5 minutes, 15 minutes, 30 minutes, 1 hour and 1 day. The sign-in page says how long to wait, and an administrator can unlock a user.
- **Security**: passwords as Argon2id hashes; `HttpOnly`, `SameSite=Strict` session cookies (the database keeps only their SHA-256), valid 30 days, signed out when the password, the role or the status changes. Plain HTTP keeps working (`Secure` only over HTTPS or with `XEXAM_COOKIE_SECURE=1`).
- **Locked out of the administrator account**: `docker compose exec backend xexam-backend reset-password admin` prints a new temporary password.
- **Database schema v3**: new tables `users`, `sessions`, `user_stats` and `user_settings` and the column `attempts.user_id`; before the migration a copy is written to `<data dir>/backups/xexam-schema2-<date-time>.db`.
- **New API**: `/api/auth/state`, `/api/auth/login`, `/api/auth/logout`, `/api/auth/password`, `/api/users`, `/api/users/create`, `/api/users/update`, `/api/users/password`, `/api/users/delete` and `/api/users/unlock`; every other route needs a signed-in account (`/api/info` stays public).
- **Repository layout**: the Japanese and Chinese READMEs moved to `README/`, the compose files and `.env.example` to `docker/` (in a clone, `.env` now goes to `docker/.env`). The README badges are rounded, and the release badge shows the version.
- Tests: backend 181 → 232, frontend 44 → 59.

### v0.5.1 — 2026-11-11

**A calmer, simpler look with colour-blind friendly colours, the menu button at the top right, and a Question banks page as compact as Home.**

- **A calmer, simpler look**: the colours now follow the colour-blind friendly light and dark palettes of GitHub's Primer design system ("Light colorblind" / "Dark colorblind"). Correct answers are blue and wrong answers orange instead of green and red. The light theme is a soft grey-white page with white blocks and navy main buttons; the dark theme is near-black. The pages sit on one calm sheet with fewer frames and borders, rounded corners, soft hover backgrounds, notes shown as tinted callouts and a lighter menu with a soft shadow. The new look itself moves nothing and changes no behaviour.
- **Menu on the right**: the three-line menu button is now at the top right of the header; the Xexam logo stays at the top left. The menu opens below the button and works as before.
- **Compact Question banks page**:
  - **Import Excel**: **New bank** and **Update a bank** are two small buttons next to the title (their tooltips explain them); for an update, **Bank to update** appears under them. One hint line ends in a link to the **Excel format guide**, and one row takes the file: **Choose Excel file**, or drop the file anywhere on the panel.
  - **Preview**: the counts are small chips (counts of zero are hidden), each sheet is one line, and a sheet's unreadable rows are folded under it. As before, the confirm and cancel buttons follow right after the counts and the notes, before the sheets.
  - **Your question banks**: one compact row per bank with its name, small **Update** and **Delete** buttons and one line of facts (file, questions, sheets, last update). The sheets are folded under the arrow on the right of the row.
  - **Import history** shows one line per import (the latest 8), and **Excel format and import guide** stays folded until you open it.
  - On touch screens the small buttons keep a tap area of about 44 px; on a phone, a long bank name pushes the buttons onto their own row.
- **Images on Docker Hub**: every release is now also published to Docker Hub as `yuminspace/xexam-frontend` and `yuminspace/xexam-backend` (public, `amd64` and `arm64` under the same tags). `docker-compose.yml` pulls from there, so `docker login` is no longer needed. The same tags stay on `ghcr.io/yumin-space` as a private mirror. If you run v0.5 with the old `docker-compose.yml`, replace it with the new one (the data volume stays). The READMEs now start with the Docker Hub badges.
- The READMEs and the docs describe the new layout, and the screenshots were retaken.

### v0.5 — 2026-11-10

**A new interface with several question banks, study records and settings, in English, Chinese and Japanese. The database moves to schema v2 automatically, after a backup copy is written.**

- **New layout**: every page except the exam has a slim header with a three-line menu button. The button opens a small menu with Home, Question banks, Study records and Settings; it closes again after a choice, a click outside it or Esc. On wide screens the page content is centered (at most 1120 px wide). The exam itself is a focused full-screen page.
- **About page**: the last menu entry, Xexam v0.5 (under a divider), opens it. It shows the version, the server's CPU architecture, the license and what is new in each version: this changelog, in the interface language, with the newest version open and the earlier ones folded.
- **Compact home page**: an unfinished exam is a one-line banner with Continue and Discard. Two small buttons in the top-right corner of the Start an exam panel choose where the questions come from: All and Wrong answers, each with its number of questions (Wrong answers is disabled, with a one-line reason, while there is nothing to review in the selected sheets). Each question bank is one card with its sheets as compact rows inside; the bank's checkbox selects all its sheets, and the sheets can be folded away (with more than two banks they start folded).
- **Random pick**: when the number of questions is smaller than the selection, the questions are now picked at random from the whole selection (before, without shuffling, the first ones in bank order were taken). Shuffle question order only decides whether they are asked in random or bank order.
- **Several question banks**: every imported workbook is its own bank. Import a file as a new bank, or update / replace an existing bank; questions missing from the file are retired only in that bank. The bank list shows each bank's sheets and counts. Deleting a bank asks for confirmation, with an option to also delete that bank's study records.
- **Study records page**: the list of exams with filters (question bank, date range, accuracy); the detail of each exam with your answer to every question, the correct answer and the explanation (for exams taken from v0.5 on); wrong-answer review and practice; retry the wrong answers or retake an exam; delete one record (the wrong-answer notebook is not changed) or all records (with an option to also clear the notebook). Xexam keeps the latest 500 exams (60 before).
- **Settings page**: theme (follow the system / light / dark), interface language (English by default, 中文, 日本語), auto-advance after answering (single choice, and a hotspot question once every item is answered; never multiple choice), reset to defaults. The settings and the question language are stored in the data directory on the server, so they survive restarts, image updates, clearing the browser cache and switching to another browser or device. An unfinished exam is still saved only in the current browser.
- **The whole interface in English, Chinese and Japanese**, including the exam page and the error messages from the server (the backend still sends them in Chinese; the page translates them).
- **Phone friendly**: the Start exam button stays at the bottom of the screen on the home page, lists are more compact, the answer sheet shows the answered count and Submit above the question tiles, and touch targets are at least 44 px.
- **Database schema v2**: new tables `banks`, `attempt_items` and `settings`, new columns `questions.bank_id`, `attempts.banks` and `imports.bank_id`. On the first start of v0.5 the existing database is migrated automatically; before that, a copy is written to `<data dir>/backups/xexam-schema1-<date-time>.db` (if the copy cannot be written, the backend does not start and changes nothing). Existing data becomes one question bank; question ids, the wrong-answer notebook and the score history are kept. Rolling back = stop, restore that copy, run v0.4. Never run v0.4 on a database that has several banks. Steps: [docs/en/migration.md](docs/en/migration.md).
- **New API**: `GET /api/banks`, `POST /api/banks/delete`, `GET /api/records`, `GET /api/record?id=`, `POST /api/records/delete`, `GET` / `POST /api/settings`, `POST /api/settings/reset`, and the optional header `X-Bank-Target` on `POST /api/upload` (without it, an upload behaves exactly as in v0.4). Each question in `GET /api/bank` gains one more key, `bank`. See [docs/en/api.md](docs/en/api.md).
- **Docs split by language**: `docs/en`, `docs/ja` and `docs/zh-CN`; the Chinese-only historical documents moved to `docs/zh-CN/history`. Each language folder has its own screenshots, with the interface in that language. CI checks this layout and every relative link.
- **CI also runs on pushes to the `dev` branch** (checks and tests only, never publishes).
- The Excel examples in the READMEs, the docs and the four-column parser tests were replaced by made-up examples; the legacy test snapshot in `tests/baseline/` is unchanged. In the older changelog entries, only the links (to the new doc folders) and one list of example words were changed.
- **MIT license** added (`LICENSE`).
- Tests: backend 151 → 181 (migration and backup, banks, records, settings, the guards on the new routes), frontend 27 → 44 (interface texts, translation of server messages, the changelog on the About page, random pick).

### v0.4 — 2026-11-10

- **Three-language side-by-side: English / Chinese / Japanese.** Excel gains a new format: column A is English, column B is Chinese, column C is Japanese (the question itself), and column D holds the answer, explanation and mnemonic. Columns A, B and C are the same question in three languages and correspond one to one; the answer is written only once. The language buttons on the quiz page become `英语 / 中文 / 日语` ("English / Chinese / Japanese"), and the wrong-answer review on the result page is also shown in the language you choose.
- **Hotspot options follow the language too**: write the options at the end of each sub-item in that language (Japanese `はい / いいえ`, Chinese `是 / 否` ("yes / no"), English `Yes / No`; the same applies to three-way choices such as the execution layer), and the option buttons switch together with the language. If a column has no options written, that language is still available and the option buttons show the Japanese ones.
- **Hotspot answers are accepted when written with option words from any language** (`調理`, `烹饪`, `Cooking`, and they can be mixed). The options of the three languages correspond one to one by position; a column whose option count does not match does not take part, and nothing is guessed.
- **The old three-column format (A Japanese, B Chinese, C answer) needs no changes and can still be uploaded**, just without English. The program detects the format from the content automatically; in the four-column format the question id still comes from the Japanese text in column C, so the same question has the same id in both formats and learning records are not affected.
- The footer text on the home page now reads: Xexam only provides import, organization and practice of questions and answers; the software itself does not provide or include exam questions, answers or related materials; users must ensure that the sources of the materials they import and use are lawful and that they hold the corresponding usage rights.
- **The README is split into three languages**: `README.md` (English, the default), `README.ja.md` and `README.zh-CN.md`, linked to each other from a language bar at the top; each keeps its own changelog, and the GitHub Release notes come from the English one.
- **The documents in `docs/` come in three languages too**: the seven current ones (architecture, API, database, security, deployment, migration, development) are `X.md` (English), `X.ja.md` (Japanese) and `X.zh-CN.md` (Chinese), with a language bar on top; `openapi.yaml` is in English. The Phase 0 audit and planning documents are historical and stay Chinese-only. CI checks that all three versions exist and that every relative link resolves.
- **New screenshots** taken with the v0.4 interface and a fictional four-column demo bank: the question in English, Chinese and Japanese, with the hotspot option buttons following the language. (The interface itself, buttons and labels, is still in Chinese only.)
- **Language convention**: code comments and the output of programs, tools and CI are in English; the UI and the READMEs use their own language. No code was changed by this (checked mechanically against the previous commit).
- Tests: backend 136 → 151 (four-column format, options and answers in three languages, fallback when they do not match), frontend 19 → 27 (language rules).

### v0.3 — 2026-11-10

**The whole program was rewritten in Rust + TypeScript + Nginx. The database file and schema are unchanged, the API is backward compatible, and the old Python implementation has been removed from the repository.**

- **Backend rewritten in Rust** (Axum 0.8 + Tokio + rusqlite, toolchain pinned to 1.99.1): REST API, XLSX parsing, import (preview without writing to the database → one-time confirmation token → one-transaction write), score and wrong-answer statistics, Host/Origin guard, and the `xexam-backend import / healthcheck / migrate-progress` subcommands. Behavior matches the Python version one to one: all 172 legacy responses were compared item by item with no unexplained differences; the parse output of 49 synthetic workbooks and all 85 question id vectors are identical, so question ids are byte-for-byte the same as in the old version and learning records do not become detached.
- **Frontend rewritten in TypeScript** (strict) + **Vite**, plain DOM, no framework; there are no inline styles, so the CSP can be very strict. The build target is `safari15`, so iPhone needs iOS 15.4 or later.
- **Two images, two containers**: `xexam-frontend` (Nginx + static frontend files, the only one exposed) and `xexam-backend` (Rust, on the `internal` private network with no published port). Non-root, `cap_drop: ALL`, read-only root filesystem; Host passed through unchanged, `X-Forwarded-*` cleared; 20 MB limit, security headers, rate limiting, timeouts; a JSON 502 is returned when the backend is unreachable. Both images ship for `amd64` and `arm64`.
- **Stricter upload checks (intentional differences)**: DOCTYPE and entity declarations are rejected in any encoding / extension; duplicate ZIP entry names are rejected; there are limits on the number of ZIP entries, XML depth / node count / attributes per element, and total cell bytes; parsing has a cooperative timeout (the parser stops by itself when time is up) and at most 2 parses run at the same time; error messages no longer echo the library's exception text; unsupported HTTP methods return 405 (previously 501). See [docs/en/api.md](docs/en/api.md) for the full table of differences.
- **One pipeline** (`.github/workflows/ci.yml`): check version and changelog → four check groups (frontend / backend / Nginx and full stack / arm64) → only if all pass, publish the two images and create the Release.
- **Tests**: 136 backend tests (against the committed legacy snapshot: 172 golden responses, 49 parse fixtures, 85 id vectors), 19 frontend tests; in `tests/integration/`, 38 Nginx checks, 34 full-stack checks and 10 `progress.json` migration checks.
- **Documentation**: architecture, API (including OpenAPI), database, security, deployment, migration and development under `docs/`, plus the pre-migration audit, risks and plan.
- Grading is still done in the browser (the server only receives `{id, correct}`, i.e. the question id and whether it was answered correctly); the reasoning is in [docs/zh-CN/history/migration-plan.md](docs/zh-CN/history/migration-plan.md).
- ⚠️ **Upgrade note**: the old and new containers have the same names and ports and **cannot run at the same time**, so run `docker compose stop` before upgrading; the data volume `xexam_data` is the same one, so no data migration is needed. The old single image `ghcr.io/yumin-space/xexam` is no longer updated (the last version was `v0.2.2`), and `docker-compose.yml` now pulls both `xexam-frontend` and `xexam-backend` at once. For rollback steps see [docs/en/migration.md](docs/en/migration.md).
- **What has not been verified (stated honestly)**: a real Cloudflare Tunnel, a real iPhone, running on real ARM hardware (it was only built and started under QEMU), and a dry run with a copy of your own real database.

### v0.2.2 — 2026-11-10

- Added `tests/baseline/`: the behavior baseline of the old Python version (172 real responses, 50 XLSX fixtures with their parse output, 85 question id vectors, a sample database, security probes, and Host/Origin check and tunnel simulation scripts), against which the later Rust rewrite is compared item by item. The tools no longer hardcode any local path; use `XEXAM_LEGACY_ROOT` to point at the code under test.
- Corrected the statement about the v0.2 Release in the previous changelog.

### v0.2.1 — 2026-11-10

- **A Release is now created automatically on GitHub's Releases page after every successful build** (previously only images were pushed and the Releases page stayed empty): the pipeline gains a 4th job, `release`, which runs after the images have been published and verified; its notes are taken from this version's section of this changelog, and it automatically creates the git tag `v<version>`.
- The v0.2 Release was created manually once (it corresponds to the v0.2 commit `1ee64c8` and is not marked Latest); from v0.2.1 on, the pipeline creates it automatically.
- Added `scripts/release-notes.py`: extracts the changelog of a given version from the README; it can also run locally to preview what the Release page will show.

### v0.2 — 2026-11-10

- **Can be reached from the internet through your own domain** (Cloudflare Tunnel + Cloudflare Access):
  - Allowed domains go into `XEXAM_ALLOWED_HOSTS`; the `Origin` of write requests is now compared by hostname only, so uploads and submitting a quiz are no longer blocked with 403 when accessed through an **HTTPS** domain.
  - When a proxy rewrites `Host`, `XEXAM_ALLOWED_ORIGINS` can be used to allow the public domain separately; when authentication is already in front, `XEXAM_ALLOW_ANY_HOST=1` can be set.
  - When a request is rejected, the message explains which check failed and how to allow it; when the login expires, the page prompts you to refresh and log in again.
- Port binding is now configurable (`XEXAM_BIND`); the default is still localhost only.
- After receiving the stop signal from `docker stop`, the program exits cleanly (previously it waited 10 seconds and was killed), and the database is closed cleanly.
- CI: split into three steps, "check version and changelog → test each of the two architectures → publish" (from v0.2.1 a fourth step is added: create the Release); the smoke test moved into `scripts/smoke-test.sh`, which can also be run directly on a machine with Docker installed; after publishing, the image manifest is verified by digest.
- `/api/info` gains `arch` (the current CPU architecture).
- Fixes after the audit (each one has a corresponding real test):
  - Server: requests rejected early (403/415) have their body read before the reply, so the browser sees the real reason instead of `连不上服务` ("cannot connect to the service"); a rejected request leaves one line in the log (visible with `docker compose logs`); malformed JSON (a number where a list is expected, `Infinity`, over-deep nesting, a non-ASCII confirmation token) returns 400/409 instead of dropping the connection; the `Content-Type` of a POST must really be `application/json`; `XEXAM_ALLOWED_HOSTS` separated by spaces or semicolons is also recognized.
  - Database: if `COMMIT` fails it rolls back instead of leaving the connection stuck in a transaction; a malformed old `progress.json` no longer keeps the program from starting; when the data directory contains only some other `.db` file, the startup log gives a warning (that file is not modified).
  - Pages: when submitting a quiz, the saved progress is cleared only after the score has been submitted successfully; on failure it is kept and a prompt `刷新页面重新登录后再交卷` ("Refresh the page, log in again, then submit") is shown; before starting a new exam, if there is an unsubmitted exam, it asks for confirmation first; the percentage on the result page uses the same rounding as the learning record and no longer differs by 1; the notification bar is no longer covered by the bottom paging bar; long English strings no longer cause a horizontal scrollbar; old Safari without `<dialog>` uses the system confirmation box; holding down a letter key no longer toggles the answer repeatedly.
  - Images and CI: the health check bypasses the proxy and follows `XEXAM_PORT`; `.dockerignore` now applies at any directory level; `.gitignore` gains backup archives and `.env.*`; publishing happens only from `main`, and queued runs on `main` are not cancelled; the in-container paths in the smoke test also work in Git Bash on Windows.
  - This version was really built locally with Docker (amd64) and passed the same smoke test that CI uses: non-root, Host/Origin blocking, data still there after deleting the container, `docker stop` in 1 second. arm64 and publishing still depend on the GitHub Actions results.
- Names unified to **xexam**: environment variables `XEXAM_DATA` / `XEXAM_HOST` / `XEXAM_PORT` / `XEXAM_ALLOWED_HOSTS` / `XEXAM_ALLOWED_ORIGINS` / `XEXAM_ALLOW_ANY_HOST`, database file `xexam.db`, key names saved in the browser `xexam.*`; the docs also no longer hardcode your own Excel file name.
- ⚠️ **When upgrading from v0.1:** the database file name changed, and the new version only reads `xexam.db`. If you already imported a question bank with an earlier version, stop the program first and rename that `.db` file in the data directory to `xexam.db` before starting, otherwise you will see an empty question bank (the old file is not deleted, it is just not read). The key names used for the saved exam and the language choice in the browser also changed, so submit your quiz before upgrading; the language will revert to Japanese. If you set the old environment variable names in a compose file you modified yourself, change them to the `XEXAM_*` names above, otherwise they will be ignored.
- `docker-compose.yml` no longer has `pull_policy: always`: `docker compose up -d` no longer checks online every time; to update, use `docker compose pull && docker compose up -d`.
- `.env` (local configuration) was added to `.gitignore` and `.dockerignore`, so it is not committed and does not end up in the image.

### v0.1 — 2026-11-10

First version.

- Quiz in the browser: single choice, multiple choice and hotspot questions; **scored only after submitting**, and wrong answers show the answer, explanation and mnemonic.
- Japanese / Chinese side-by-side switching; answer sheet; flagging; save and exit, then resume.
- Wrong-answer notebook: a wrongly answered question is removed after 2 consecutive correct answers; score history.
- The question bank and learning records are now stored in **SQLite** (Docker named volume `xexam_data`) and no longer depend on Excel.
- The admin page **uploads Excel to update the question bank**: preview first (added / updated / restored / retired / unreadable), and nothing is written until you confirm; retired questions keep their answer records and are restored automatically when they are uploaded again.
- The old version's `progress.json` is migrated automatically on first start.
- **iPhone** support: touch targets of at least 44 px, safe areas, a fixed bottom paging bar, and it can be added to the home screen.
- The Docker images support both **x86 (amd64) and ARM (arm64)**, built and tested by GitHub Actions.
- ⚠️ The "publish" step of v0.1 did not succeed, so there is **no** `v0.1` tag on `ghcr.io/yumin-space/xexam`; images are published from v0.2 on (`latest` / `v0.2`).
- The app was renamed to **Xexam**.

<!-- Write new versions above this line, format: ### vX.Y — date -->
