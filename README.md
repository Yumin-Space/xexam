**English** | [日本語](README/README.ja.md) | [简体中文](README/README.zh-CN.md)

<!-- The release badge shows RELEASE_VERSION from VERSION, the last published release. Do not edit it by hand: the release job of .github/workflows/ci.yml writes it (scripts/set-release-version.py) after the images and the GitHub Release are published. GitHub rounds an image's corners only when it has both width and height, so every badge carries its exact size (in px, from shields.io). -->
<p>
  <a href="../../releases/latest"><img width="145" height="28" alt="Release v0.9.1" src="https://img.shields.io/badge/release-v0.9.1-green?style=for-the-badge"></a>
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
<p align="center">
  <a href="https://hub.docker.com/r/yuminspace/xexam-ai-service"><img width="369" height="28" alt="Docker image yuminspace/xexam-ai-service" src="https://img.shields.io/badge/Docker%20Image-yuminspace%2Fxexam--ai--service-blue?style=for-the-badge"></a>
  <a href="https://hub.docker.com/r/yuminspace/xexam-ai-service"><img width="150" height="28" alt="Docker stars" src="https://img.shields.io/docker/stars/yuminspace/xexam-ai-service?style=for-the-badge"></a>
  <a href="https://hub.docker.com/r/yuminspace/xexam-ai-service"><img width="148" height="28" alt="Docker pulls" src="https://img.shields.io/docker/pulls/yuminspace/xexam-ai-service?style=for-the-badge"></a>
</p>

# Xexam

Xexam is a self-hosted app for practising exam questions in the browser. You import your own Excel question banks, then take exams or practise with them, and review the questions you got wrong. If an administrator connects an AI provider and gives them the **AI+ feature**, users can also ask an AI model in **AI Chat** and get AI help while they study: hints and explanations in practice, an AI analysis of every question on result pages and an AI summary of their wrong answers. It works on computers and on phones (iPhone).

![The exam page: a hotspot question in English, the English / 中文 / 日本語 switch and the answer sheet with Submit](docs/en/screenshots/desktop-2-quiz.png)

The repository contains **no questions**. Xexam only provides the tools to import, organize and practise questions; your question banks are your own Excel files and are never committed (`.gitignore` excludes them). The screenshots use made-up demo banks, not content from any real exam.

| | |
|---|---|
| Current version | **v0.9.1** (see the [changelog](docs/en/changelog.md) or the repository's [Releases](../../releases) page; in the app: menu → **Xexam v0.9.1**) |
| Images | Three, on Docker Hub (public, no login needed): `docker.io/yuminspace/xexam-frontend`, `docker.io/yuminspace/xexam-backend` and `docker.io/yuminspace/xexam-ai-service`, with tags `latest` / `v0.9.1` / `sha-<commit>` (the `sha-` tags are for troubleshooting). Mirror: `ghcr.io/yumin-space/xexam-frontend`, `ghcr.io/yumin-space/xexam-backend` and `ghcr.io/yumin-space/xexam-ai-service` (private, same tags) |
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
- **Several people, one Xexam**: everyone signs in and has their own records and settings. Administrators decide who sees which question bank and who may import banks (bank managers), and can follow the **System log**.
- **AI Chat** (optional, off until an administrator sets it up): administrators connect an OpenAI-compatible API, Anthropic or Cloudflare AI Gateway in **AI settings**, test it and switch the **AI+ feature** on, and give the **AI+ feature** to each user who may use it in **User management**. In **AI settings** they can **Pause** and **Resume** a user's **AI+ feature** without taking it away. Answers stream in (with **Thinking…** while a reasoning model works) and keep being written when the page is closed; the AI answers in the language chosen in **Settings**; chats are saved (and deleted automatically after 90 days by default); **Memory** keeps short notes the AI uses in later chats; administrators can choose **Free** under **AI chat mode** in **Settings** to talk to the model without the study-assistant instructions. Administrators see usage statistics and can view and delete the chats and memories of users with the AI+ feature, which AI Chat tells every user. The AI runs in its own container, `ai-service`, with its own database; the provider keys are stored encrypted.
- **AI help while you study** (for users with the **AI+ feature**; nothing during an exam): in a practice, **AI hint** (how to approach the question, without giving the answer) and **AI explanation** (the answer and why), from the action bar or the bulb and book icons above the question, in a small window that streams the answer; on every practice and exam result page and on a study record's detail page, **AI analysis** of each question; on **Study records** and **Home**, an **AI summary** of the wrong answers of a record (weak knowledge points, what to watch out for, how to approach such questions). Every answer is saved in AI Chat, in the chat of its practice (`<start>-Practice`), its exam (`<start>-Exam`) or its summary (`<record time>-AI summary`). Users whose AI is paused or unavailable see these controls greyed out, marked **Unavailable**, with the reason; users without the **AI+ feature** do not see them at all. They send the question (and, for explanations and summaries, the bank's answer and explanation and your own answer) to the AI provider you set up; the hint request never contains the answer, and a hint that gives it away is hidden.
- **Desktop and phone**: a slim header with a menu, pages centered on wide screens, touch targets of at least 44 px on phones.

How each page works, the AI help in practice and results, AI Chat and AI settings, the Excel format and the import rules: [docs/en/usage.md](docs/en/usage.md).

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

This pulls the three images and starts **three containers**: `xexam-frontend` (Nginx, the only one reachable from outside), `xexam-backend` (Rust, on a private network without internet access) and `ai-service` (Rust, the AI service; the only container that can reach the internet, which it does only to call the AI provider an administrator sets up). It reports to the backend every 25 seconds, so Xexam knows whether the AI is available. Xexam works without `ai-service` too; only AI Chat is then missing, and **AI settings** says that ai-service is not running.

**3. Sign in.** Open <http://localhost:8765> in a browser on the same machine and sign in with the username `admin` and the password `admin`. Xexam then asks you to choose your own password: do this right away, because until then anyone who can open the page can use the default password.

**4. Import a question bank and take an exam.** Open the menu (the three-line button at the top right), choose **Question banks** and import your Excel workbook, then start an exam on **Home**: see [Your first exam](docs/en/usage.md#1-your-first-exam). The interface starts in English; switch it to Chinese or Japanese under **Settings** → **Language**.

**5. AI Chat (optional).** AI is off until you set it up: menu → **AI settings** (it shows whether ai-service is connected), choose the provider, enter its address and key, pick a model from the list, **Test**, then switch on the **AI+ feature**. Administrators can use AI Chat at once; other users after you turn on the **AI+ feature** for them in **User management** (the switch appears there only while ai-service is connected and the AI+ feature is on). The steps are in [section 17 of docs/en/usage.md](docs/en/usage.md#17-ai-settings).

| Action | Command |
|---|---|
| Update to the latest version | `docker compose pull`, then `docker compose up -d` (read [Upgrading](#upgrading) first) |
| Pin to a specific version | Change `:latest` to `:v0.9.1` for all three images in `docker-compose.yml`, then run `docker compose up -d` |
| Check status | `docker compose ps` (all three containers should be `healthy`) |
| Stop / start again | `docker compose stop` / `docker compose start` |
| View logs | `docker compose logs -f backend` and `docker compose logs -f ai-service` (administrators can also open **System log** in the menu; it shows the AI service's lines too) |
| See which version and architecture are running | Open <http://localhost:8765/api/info> in a browser |

> ⚠️ **Do not run `docker compose down -v`**: `-v` also deletes the two data volumes, `xexam_data` (your question banks, users, study records and settings) and `xexam_ai_data` (the AI settings, the encrypted keys and their key file, the chats and memories).

- **From a phone or another computer**, or **from the internet through Cloudflare Tunnel**: section 4 of [docs/en/deployment.md](docs/en/deployment.md).
- **Building the images from source** (when you change the code): `docker compose -f docker/docker-compose.build.yml up -d --build` in a clone, see section 2 of [docs/en/deployment.md](docs/en/deployment.md).
- **Backups**: stop Xexam and pack the two data volumes; the commands are in section 5 of [docs/en/database.md](docs/en/database.md). The AI volume also holds the key file that decrypts the saved AI keys, so keep its backup as safe as the keys themselves.

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
| `TZ` | unset (the host's time zone) | Time zone of the times Xexam shows and of the days in the AI usage statistics, for example `TZ=UTC` |
| `XEXAM_AI_SECRET_KEY` | unset (a key file in the AI volume) | The key that encrypts the saved AI provider keys: 32 bytes as 64 hex digits or as base64, for example from `openssl rand -base64 32`. Set it to keep the key apart from the AI volume, so that a copy of that volume alone cannot decrypt the keys. If it changes or is lost, the AI keys must be entered again in **AI settings** |
| `XEXAM_INTERNAL_TOKEN` | unset (a random token in the link volume) | The token the backend and the AI service share; the compose file passes the same value to both. At least 32 printable ASCII characters without spaces, for example from `openssl rand -hex 32`. It also authenticates the AI service's own calls to the backend, such as its state report. A value that breaks this rule switches AI Chat off: **AI settings** then says that ai-service cannot connect, and a log line says why |
| `XEXAM_AI_LOG` | unset (`info`) | The AI service's log filter, like `XEXAM_LOG` |
| `XEXAM_AI_SQLITE_JOURNAL` | unset (WAL) | `delete` for the AI database, like `XEXAM_SQLITE_JOURNAL` |

Times follow the host's time zone because the compose file mounts the host's `/etc/localtime` into the backend and the AI service. On a host without that file, and on Docker Desktop (Windows, macOS), whose virtual machine usually runs on UTC, read section 3.1 of [docs/en/deployment.md](docs/en/deployment.md). Every option is explained in section 3 of that document. The AI provider, its address, the keys and the model are not set here but in the app, in **AI settings**.

---

## Upgrading

Every upgrade keeps the same port and the same volume `xexam_data`, so your data stays where it is: `docker compose pull`, then `docker compose up -d`. v0.8 added a third image and two volumes, so coming from v0.7.5 or older you also need the new `docker-compose.yml`; v0.9 needs no new compose file. When a database schema changes, the first start of the new version writes a backup copy of the database first, into `/data/backups/` in the volume (the AI service does the same for its own database, in its own volume), and then migrates it in one transaction. If the copy cannot be written (disk full, no permission), the service refuses to start and leaves the database unchanged; `docker compose logs backend` and `docker compose logs ai-service` say why. A backup copy is about as large as the database, so keep that much free space before upgrading; backup copies are never deleted automatically.

The sections below are written for the version they lead to. Pulling the current images (v0.9.1) from an older version does every step in one go: one backup named after the schema the file had, and the database ends at schema 6 (the log then says `schema 6` where a section says `schema 5`; `/api/info` shows `"version": "0.9.1"`).

### From v0.9 to v0.9.1

Nothing to do: `docker compose pull`, then `docker compose up -d`. There is no database change and no new setting; the three images, the volumes, `docker-compose.yml` and `.env` stay as they are. The changes are listed in the [changelog](docs/en/changelog.md) (it lives in `docs/<lang>/changelog.md` now).

### From v0.8 to v0.9

v0.9 adds the AI help for studying (see the [changelog](docs/en/changelog.md)). The three images, the volumes and the `docker-compose.yml` stay as they are: **no new compose file is needed**. Back up both data volumes first, then pull and start.

```bash
# 1. Back up both volumes (stop first; the backup files hold your whole databases, do not commit them)
docker compose stop
docker run --rm -v xexam_data:/data -v "${PWD}:/backup" alpine tar czf /backup/xexam-data-v08.tgz -C /data .
docker run --rm -v xexam_ai_data:/data -v "${PWD}:/backup" alpine tar czf /backup/xexam-ai-data-v08.tgz -C /data .

# 2. Pull the three images and start (do not use down -v), then read the upgrade lines
docker compose pull
docker compose up -d
docker compose logs backend
docker compose logs ai-service
```

- **The backend's first start** writes the copy `/data/backups/xexam-schema5-<YYYYmmdd-HHMMSS>.db` and moves the database to schema v6: two new columns on `attempts`, `session_key` and `started_at`, empty for every record you already have. Nothing else changes. The log says `Upgraded the database to schema 6`.
- **The AI service's first start** writes the copy `/data/backups/xexam-ai-schema2-<YYYYMMDD-HHMMSS>.db` in the AI volume and moves its database to schema 3 (study chats and their messages, and a `purpose` on the usage rows). Your chats, memories, settings and keys stay. The log says `Upgraded the AI database to schema 3`.
- **Nothing to set up**: the **AI+ feature** works as before. Everyone who can use AI now sees the new controls; users without the **AI+ feature** see nothing new. Records made before v0.9 have no session key, so their first AI analysis starts a chat of its own (section 16 of [docs/en/usage.md](docs/en/usage.md)).
- Check that <http://localhost:8765/api/info> shows `"version": "0.9"` and that `docker compose ps` lists three healthy containers.

Rolling back to v0.8: the main database needs no restore (v0.8 opens the schema-6 database as it is, and records saved afterwards simply have no key), but v0.8's AI service refuses an AI database of schema 3, so its copy `xexam-ai-schema2-….db` must be restored, which loses every AI chat, setting and usage row made since the upgrade. See section 0 of [docs/en/migration.md](docs/en/migration.md).

### From v0.7.5 (or v0.7) to v0.8

v0.8 adds the AI service: a third image, `xexam-ai-service` (the container and the Compose service are called `ai-service`); two new volumes, `xexam_ai_data` (the AI database and its key file) and `xexam_link` (only the token the backend and the AI service share); and the network `egress` (internet access for the AI service only). **Download the new `docker-compose.yml`**. With the old one everything works except AI, and **AI settings** says that ai-service is not running.

```bash
# 1. Back up the volume (stop first; the backup file holds your whole database, do not commit it)
docker compose stop
docker run --rm -v xexam_data:/data -v "${PWD}:/backup" alpine tar czf /backup/xexam-data-v075.tgz -C /data .

# 2. Replace docker-compose.yml with the new one (your .env stays; carry over your own changes to the old file)
curl -fLO https://raw.githubusercontent.com/Yumin-Space/xexam/main/docker/docker-compose.yml

# 3. Pull the three images and start (do not use down -v), then read the upgrade line
docker compose pull
docker compose up -d
docker compose logs backend
```

- **The first start** writes the copy `/data/backups/xexam-schema4-<YYYYmmdd-HHMMSS>.db` and moves the database to schema v5: two new columns on `users`, the **AI+** switch (off for everyone) and an identity stamp. Nothing else changes; banks, records, users and settings stay. The log says `Upgraded the database to schema 5`.
- **AI is off** until an administrator sets it up in **AI settings** (section 17 of [docs/en/usage.md](docs/en/usage.md)); there is nothing to migrate for AI. Administrators can use AI Chat as soon as the **AI+ feature** is on; every other user needs the **AI+ feature** in **User management**, which can be switched on only while ai-service is connected and the **AI+ feature** is on. The AI database `xexam-ai.db` is created by ai-service at its first start; if it comes from a pre-release build of v0.8, the columns it lacks are added automatically.
- **What to back up from now on**: both data volumes, `xexam_data` and `xexam_ai_data`. The AI volume holds the key file `secret.key` next to the AI database, so its backup can decrypt the saved keys; set `XEXAM_AI_SECRET_KEY` to keep the key apart. `xexam_link` needs no backup: the backend creates a new token when it is empty.
- Check that <http://localhost:8765/api/info> shows the version (`"0.8"`, or `"0.9.1"` with the current images) and that `docker compose ps` lists three healthy containers (`xexam-frontend`, `xexam-backend` and `ai-service`).

Rolling back to v0.7.5 means restoring the copy from `backups/` and the old `docker-compose.yml`; see section 1 of [docs/en/migration.md](docs/en/migration.md).

### From v0.6 to v0.7 and later

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

   The new `docker-compose.yml` follows the host's time zone, limits the container logs, passes `XEXAM_SQLITE_JOURNAL`, `XEXAM_COOKIE_SECURE` and `XEXAM_LOG` from `.env`, and adds the AI service of v0.8 with its volumes.
3. **What the first start does**, before it answers any request: it writes the copy `/data/backups/xexam-schema3-<YYYYmmdd-HHMMSS>.db`, migrates the database to schema v5 (v0.7 and v0.7.5 stop at v4) and switches it to WAL mode. Every existing user keeps every existing bank, with **Include banks imported later**; every bank belongs to the first administrator; every earlier record is an **Exam**; nobody is a bank manager yet. Then it retires the old-layout questions, with one warning per bank in the log, for example `retired 12 questions of bank "Kitchen basics": they use the old 3- / 4-column layout; import the six-column version of the workbook`.
4. **Update each old bank with its six-column file**: **Question banks** → **Import Excel** → **Update a bank**, pick the bank, upload the file and confirm. The bank keeps its name, its owner and the users who see it. Its questions are now recognised by their English text, so they start without answer history; the old questions stay retired and still show in the old records. Or import the six-column file as a **New bank**.
5. **Afterwards**: decide who is a **Bank manager** and which banks each user sees (a user you add from now on sees no bank until you choose some), and check that <http://localhost:8765/api/info> shows `"version": "0.9.1"`. AI is off until you set it up (see the previous section).

Rolling back to v0.6 means restoring the copy from `backups/`; the commands are in section 2.6 of [docs/en/migration.md](docs/en/migration.md).

### From older versions

From v0.5, v0.4 or the Python version v0.2.x the same steps apply, with a few more (sign-in arrives, the image source changed in v0.5.1, the old single container must be removed): see [docs/en/migration.md](docs/en/migration.md), which also has every rollback.

---

## Documentation

| Document | What is in it |
|---|---|
| [Usage](docs/en/usage.md) | Every page, exams and practice, study records, settings, the Excel format, importing, users and sign-in, the System log, AI Chat and AI settings, the iPhone |
| [Deployment](docs/en/deployment.md) | Starting, configuration, time zone, phone and LAN access, Cloudflare Tunnel, AI providers, backups, health checks, logs, recovery |
| [Migration and rollback](docs/en/migration.md) | Every upgrade path with its checks and rollback |
| [Database](docs/en/database.md) | The tables of the main database and of the AI database, question ids, backup and restore, schema migrations |
| [Security](docs/en/security.md) | Threat model, Host / Origin guard, sign-in and locks, permissions, upload checks, Nginx limits; the AI service: identity, encrypted keys, what administrators can see |
| [Architecture](docs/en/architecture.md) | How frontend, Nginx, backend and the AI service fit together, the data boundary, the directory layout, the request flows |
| [API](docs/en/api.md) ([OpenAPI](docs/en/openapi.yaml)) | Every endpoint, the AI routes and their event stream, who may call them, the errors |
| [Development](docs/en/development.md) | Building and testing (backend, AI service, frontend), the screenshot and icon tools, CI and releases |

---

## License

Xexam is released under the [MIT License](LICENSE). Copyright (c) 2026 Yumin-Space.

The license covers the software in this repository. The repository contains no exam questions: the question banks you import are your own files, and you are responsible for making sure that you may use them.

---

## Changelog

The changes in each version, newest first, are in [docs/en/changelog.md](docs/en/changelog.md).
