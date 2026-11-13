**English** | [日本語](README/README.ja.md) | [简体中文](README/README.zh-CN.md)

<!-- The release badge shows RELEASE_VERSION from VERSION, the last published release. Do not edit it by hand: the release job of .github/workflows/ci.yml writes it (scripts/set-release-version.py) after the images and the GitHub Release are published. GitHub rounds an image's corners only when it has both width and height, so every badge carries its exact size (in px, from shields.io). -->
<p>
  <a href="../../releases/latest"><img width="132" height="28" alt="Release v1.0" src="https://img.shields.io/badge/release-v1.0-green?style=for-the-badge"></a>
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

![The exam page: a hotspot question in English, the English / 中文 / 日本語 switch and the answer sheet with Finish exam](docs/en/screenshots/desktop-2-quiz.png)

The repository contains **no questions**. Xexam only provides the tools to import, organize and practise questions; your question banks are your own Excel files and are never committed (`.gitignore` excludes them). The screenshots use made-up demo banks, not content from any real exam.

| | |
|---|---|
| Current version | **v1.1** (see the [changelog](docs/en/changelog.md) or the repository's [Releases](../../releases) page; in the app: menu → **Xexam v1.1**) |
| Images | Three, on Docker Hub (public, no login needed): `docker.io/yuminspace/xexam-frontend`, `docker.io/yuminspace/xexam-backend` and `docker.io/yuminspace/xexam-ai-service`, with tags `latest` / `v1.1` / `sha-<commit>` (the `sha-` tags are for troubleshooting). Mirror: `ghcr.io/yumin-space/xexam-frontend`, `ghcr.io/yumin-space/xexam-backend` and `ghcr.io/yumin-space/xexam-ai-service` (private, same tags) |
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
- **AI help while you study** (for users with the **AI+ feature**; nothing during an exam): in a practice, **AI hint** (how to approach the question, without giving the answer) and **AI explanation** (the answer and why), in one panel under the question; on the results and in study records, **AI analysis** and **AI summary** of your wrong answers. The answers are saved in **AI Chat**, and nothing in the AI panel leaves the page. Each user has per-minute, per-hour and per-day limits on AI requests, set by an administrator.
- **Phones first**: every page works at phone width without sideways scrolling, with icon buttons that show their name on press and hold, touch targets of at least 44 px, a centred footer line with the project link, and the questions and answers getting most of the space. Desktop pages are centred on wide screens.

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
| Pin to a specific version | Change `:latest` to `:v1.1` for all three images in `docker-compose.yml`, then run `docker compose up -d` |
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

Every upgrade keeps the same port and the same volume `xexam_data`: `docker compose pull`, then `docker compose up -d`. Back up first; the steps for every version, the backup copies and every rollback are in [docs/en/migration.md](docs/en/migration.md).

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
