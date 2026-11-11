**English** | [日本語](README/README.ja.md) | [简体中文](README/README.zh-CN.md)

<!-- The release badge shows RELEASE_VERSION from VERSION, the last published release. Do not edit it by hand: the release job of .github/workflows/ci.yml writes it (scripts/set-release-version.py) after the images and the GitHub Release are published. GitHub rounds an image's corners only when it has both width and height, so every badge carries its exact size (in px, from shields.io). -->
<p>
  <a href="../../releases/latest"><img width="132" height="28" alt="Release v0.7" src="https://img.shields.io/badge/release-v0.7-green?style=for-the-badge"></a>
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

Xexam is a self-hosted app for practising exam questions in the browser. You import your own Excel question banks, then take exams or practise with them, and review the questions you got wrong.

- **Exam**: nothing is scored until you submit. Then you see your score and each wrong question (or all of them) with your answer, the correct answer, the explanation and the memory aid.
- **Practice**: every answer is checked right away and shows the correct answer and the explanation.
- **Several people, one Xexam**: everyone signs in and has their own study records. Administrators decide who sees which question bank and who may import banks.

It works on computers and on phones (iPhone).

![The exam page: a hotspot question in English, the English / 中文 / 日本語 switch and the answer sheet with Submit](docs/en/screenshots/desktop-2-quiz.png)

The repository contains **no questions**. Xexam only provides the tools to import, organize and practise questions; your question banks are your own Excel files and are never committed (`.gitignore` excludes them). The screenshots use two **made-up demo banks**, "General knowledge" and "Kitchen basics", not content from any real exam. Both are in the six-column layout, so every question, answer and explanation exists in English, Chinese and Japanese. The screenshots show the English interface; those of the Japanese and the Chinese interface are in `docs/ja/screenshots/` and `docs/zh-CN/screenshots/`.

| | |
|---|---|
| Current version | **v0.7** (see the [changelog](#changelog) or the repository's [Releases](../../releases) page; in the app: menu → **Xexam v0.7**) |
| Images | On Docker Hub (public, no login needed): `docker.io/yuminspace/xexam-frontend` and `docker.io/yuminspace/xexam-backend`, with tags `latest` / `v0.7` / `sha-<commit>` (the `sha-` tags are for troubleshooting). Mirror: `ghcr.io/yumin-space/xexam-frontend` and `ghcr.io/yumin-space/xexam-backend` (private, same tags) |
| CPU architecture | `linux/amd64`, `linux/arm64` (the right one is picked automatically under the same tag) |
| Port | `8765` (bound to the local machine only by default) |
| License | [MIT](LICENSE) |

Documentation: [Architecture](docs/en/architecture.md) · [API](docs/en/api.md) ([OpenAPI](docs/en/openapi.yaml)) · [Database](docs/en/database.md) · [Security](docs/en/security.md) · [Deployment](docs/en/deployment.md) · [Migration and rollback](docs/en/migration.md) · [Development](docs/en/development.md)

---

## Contents

- [What it does](#what-it-does)
- [Quick start (installation and deployment)](#quick-start-installation-and-deployment)
- [Configuration](#configuration)
- [Opening Xexam from your phone or another computer](#opening-xexam-from-your-phone-or-another-computer)
- [Using Xexam](#using-xexam)
- [Excel question bank format](#excel-question-bank-format)
- [Importing question banks](#importing-question-banks)
- [Upgrading](#upgrading)
- [Data, backup and restore](#data-backup-and-restore)
- [Access from the internet via Cloudflare Tunnel](#access-from-the-internet-via-cloudflare-tunnel)
- [Architecture](#architecture)
- [Security notes](#security-notes)
- [Automated build and release](#automated-build-and-release)
- [Development](#development)
- [Directory structure](#directory-structure)
- [License](#license)
- [Changelog](#changelog)

---

## What it does

- **Your own question banks.** Every Excel workbook (`.xlsx`) you import becomes its own question bank, and each worksheet in it is a group of questions you can pick. Import a workbook as a new bank, or update an existing bank with a newer version of its file. A preview shows what will change before anything is saved. Three layouts are recognised by themselves: 3 columns (one language), 4 columns (Japanese with English and Chinese) and 6 columns (questions, answers and explanations in English, Chinese and Japanese).
- **Exams and practice.** Pick banks and sheets, all questions or only the wrong-answer notebook, the number of questions (fewer than you selected = picked at random) and whether to shuffle the order. Single-choice, multiple-choice and hotspot questions (a list of statements, each answered with one choice such as Yes / No).
  - In an **exam** there is no right/wrong feedback until you submit; you can jump between questions, mark questions to revisit, and save and exit to continue later.
  - In a **practice** each answer is checked at once and shows the correct answer, the explanation and the memory aid.
- **Three languages.** The whole interface (menus, buttons and messages, the server's error messages included) is available in English, Chinese and Japanese; you choose it in **Settings**. Questions switch between **English**, **中文** and **日本語** at any time, as far as the workbook has them; your answers stay as they are.
- **Results and study records.** After an exam or a practice: the score, then each question with your answer, the correct answer, the explanation and the memory aid. Everything is kept in **Study records** (your latest 500 records), tagged **Exam** or **Practice**, with filters, the detail of each record, and buttons to retry the wrong answers or take it again.
- **Wrong-answer notebook.** Questions you answer wrong (in an exam or a practice) are collected automatically and leave the notebook after 2 correct answers in a row. You can review them or practise only them.
- **Accounts and permissions.** Everyone signs in. Administrators manage users and every question bank and choose which banks each user sees. A user with the **Bank manager** permission may import their own banks. Each user has their own study records, notebook and settings.
- **Settings stored on the server.** Theme, interface language, auto-advance and the question language are saved with your account, so they are the same in every browser and on every device, and they survive restarts and image updates.
- **System log.** Administrators can follow what the backend logs, live, in the menu under **System log**.
- **Desktop and phone.** A slim header with a menu button, pages centered on wide screens, and touch targets of at least 44 px on phones.

---

## Quick start (installation and deployment)

### What you need

- A machine with **Docker** (including Compose v2) installed: Windows / macOS / Linux all work, **x86 or ARM**.
- A web browser.

### Option A: pull the prebuilt images (recommended)

You only need **one file** from the repository, [`docker-compose.yml`](docker/docker-compose.yml) in the `docker/` folder; no other code is required.

**If you are not the owner:** the two images are public on Docker Hub, so pulling them needs no account and no `docker login`. Only the repository is private: to download `docker-compose.yml` from it with `curl` (or to `git clone` it), ask the owner to add your GitHub account as a collaborator on the repository, then create one token under GitHub → **Settings** → **Developer settings** → **Personal access tokens** → **Tokens (classic)** with the scope `repo`. Use it as `YOUR_TOKEN` for `curl` and as the password for `git clone`. Or simply ask the owner to send you the file.

**1. Download `docker-compose.yml`** into a new folder (for example `xexam`). The easiest way: open the file on GitHub and click **Download raw file**. Or on the command line:

```bash
curl -fLO https://raw.githubusercontent.com/Yumin-Space/xexam/main/docker/docker-compose.yml
```

With `curl`, add `-H "Authorization: token YOUR_TOKEN"` after `-fLO`. In Windows PowerShell, type `curl.exe` instead of `curl` (there, `curl` is a different command).

**2. No login needed.** `docker-compose.yml` pulls both images from Docker Hub (`docker.io/yuminspace/xexam-backend` and `docker.io/yuminspace/xexam-frontend`), where they are public, so go straight to step 3. Only if you have access to the private mirror on GitHub's container registry and want to use it instead (the owner and collaborators): log in once, then change the two `image:` lines in `docker-compose.yml` to `ghcr.io/yumin-space/xexam-backend:latest` and `ghcr.io/yumin-space/xexam-frontend:latest`.

```bash
docker login ghcr.io -u YOUR_GITHUB_USERNAME
# Only for the GHCR mirror. For the password, enter a GitHub token with the read:packages scope
# (GitHub → Settings → Developer settings → Personal access tokens → Tokens (classic))
```

For the mirror, without read access or with a fine-grained token instead of a classic one, `docker login` or the next step is refused (`denied` or `unauthorized`). Docker Hub needs none of this.

**3. Start.** In the folder that contains `docker-compose.yml`, run:

```bash
docker compose up -d
```

This pulls the two images from Docker Hub and starts **two containers** (`xexam-frontend`, `xexam-backend`). Then open <http://localhost:8765> in a browser on the same machine. Sign in with the username `admin` and the password `admin`; Xexam then asks you to choose your own password. Do this right away: until then, anyone who can open the page can use the default password.

Docker runs on another machine (a server, a NAS, a virtual machine)? See [Opening Xexam from your phone or another computer](#opening-xexam-from-your-phone-or-another-computer).

The interface starts in English. To switch it to Chinese or Japanese, open the menu (the three-line button at the top right), choose **Settings** and pick the language under **Language**. Then follow [Your first exam](#your-first-exam) to import your Excel file and take an exam.

> Coming from v0.6? Read [From v0.6 to v0.7](#from-v06-to-v07) first: the first start of v0.7 moves the database to schema v4 (after writing a backup copy). Coming from v0.4? Read [Upgrading from v0.4](#upgrading-from-v04).

| Action | Command |
|---|---|
| Update to the latest version | `docker compose pull`, then `docker compose up -d`. If your `docker-compose.yml` still names `ghcr.io` (files from before v0.5.1), replace it with the new one first; the data volume stays |
| Pin to a specific version | Change `:latest` to `:v0.7` for both images in `docker-compose.yml`, then run `docker compose up -d` |
| Check status | `docker compose ps` (both containers should be `healthy`) |
| Stop / start again | `docker compose stop` / `docker compose start` |
| View logs | `docker compose logs -f` (administrators can also open **System log** in the menu) |
| See which version and architecture are running | Open <http://localhost:8765/api/info> in a browser |

> ⚠️ **Do not run `docker compose down -v`**: `-v` also deletes the data volume `xexam_data`, which holds your question banks, users, study records and settings.

The defaults work on one computer; all options are listed under [Configuration](#configuration).

### Option B: build from source (for when you change the code)

You need the full repository code:

```bash
git clone https://github.com/Yumin-Space/xexam.git
cd xexam
docker compose -f docker/docker-compose.build.yml up -d --build
```

While the repository is private, `git clone` asks you to sign in to GitHub (when the command line asks for a password, enter a GitHub token that can read the repository).

The first build takes a few minutes (it compiles Rust and the frontend). It uses the **same volume** (`xexam_data`) and the same container names as Option A, so the data is shared and the two **cannot run at the same time**; run `docker compose stop` before switching. In a clone, the compose files are in `docker/` and so is `.env` (`docker/.env`): run the other `docker compose` commands of this README from the repository root with `-f docker/docker-compose.build.yml` added (for example `docker compose -f docker/docker-compose.build.yml logs -f`). To update a source build, run `git pull` and then the build command above again. An older clone with `.env` at the repository root: move it to `docker/.env`, because Compose no longer reads it there.

---

## Configuration

The defaults work on one computer without any configuration. To change something, create a `.env` file next to `docker-compose.yml` (in a clone: `docker/.env`; see [`.env.example`](docker/.env.example) for a template; you do not need to edit the compose file itself). After changing `.env`, run `docker compose up -d` again.

| Name in `.env` | Default | Purpose |
|---|---|---|
| `XEXAM_BIND` | `127.0.0.1:8765` | The address and port to listen on (Nginx); it must be written in the form `IP:port`. By default only this machine can connect; `0.0.0.0:8765` = other machines on the LAN can connect too. ⚠️ Writing only a port number (such as `8766`) = reachable on all network interfaces |
| `XEXAM_ALLOWED_HOSTS` | empty | Which domain names / IPs may be used to access it, separated by commas, spaces or semicolons, for example `xexam.example.com,192.168.1.20` |
| `XEXAM_ALLOWED_ORIGINS` | empty | Needed only when a reverse proxy rewrites the Host to the origin server's address: enter your public domain (including `https://`) |
| `XEXAM_ALLOW_ANY_HOST` | `0` | Set to `1` = stop checking the Host (use it only when authentication is already in front) |
| `XEXAM_PARSE_TIMEOUT_SECS` | `30` | Maximum time to parse one Excel file (seconds) |
| `XEXAM_SQLITE_JOURNAL` | empty (WAL) | The SQLite journal mode. Empty or any other value = WAL, the default since v0.7. `delete` = the older DELETE mode, only for a data volume on unusual storage: a network filesystem, or a Windows folder bind-mounted into Docker |

Times shown in Xexam (study records, sign-ins, the System log) follow the host's time zone: the compose file mounts the host's `/etc/localtime`. To use another zone, set it in `.env`, for example `TZ=UTC`. On a host without `/etc/localtime` the backend container does not start: remove that mount from `docker-compose.yml` (its comment says which lines) and set `TZ` in `.env` instead. On Docker Desktop (Windows, macOS) the mounted file belongs to Docker's own virtual machine, which usually runs on UTC, not to your computer: set `TZ` in `.env` to the zone you want.

More details: section 3 of [docs/en/deployment.md](docs/en/deployment.md).

---

## Opening Xexam from your phone or another computer

**Xexam has its own sign-in since v0.6** (see [Users and sign-in](#users-and-sign-in)), and by default it binds only to the local machine (`127.0.0.1`): only the machine that runs Docker can open <http://localhost:8765>. There are two ways to open it from another device.

### From another computer through SSH (no configuration)

If Xexam runs on a server, NAS or virtual machine that you can log in to with SSH, run this on your own computer:

```bash
ssh -L 8765:127.0.0.1:8765 user@server
```

While this SSH connection is open, open <http://localhost:8765> in the browser on your own computer. Nothing changes on the server, and no port is opened.

### Directly from a phone or a computer on your network

Do this **only on a network you trust** (such as your home Wi-Fi), not in a café or on an office network:

1. Find the LAN IP address of the machine that runs Docker (usually `192.168.x.x` or `10.x.x.x`):
   - Windows: run `ipconfig` and look at the "IPv4 Address" line;
   - macOS: run `ipconfig getifaddr en0`;
   - Linux: run `hostname -I`; it is usually the first address.
2. Create a `.env` file next to `docker-compose.yml` with two lines: `XEXAM_BIND=0.0.0.0:8765` and `XEXAM_ALLOWED_HOSTS=` followed by that IP address (for example `XEXAM_ALLOWED_HOSTS=192.168.1.20`).
3. Run `docker compose up -d`, then open that IP address with port 8765 (for example `http://192.168.1.20:8765`) in the browser of the phone (Safari on an iPhone) or of the other computer.

To use it away from home (not on your home Wi-Fi), see [Access from the internet via Cloudflare Tunnel](#access-from-the-internet-via-cloudflare-tunnel) below; do not open the port directly to the internet.

---

## Using Xexam

### Your first exam

1. Open the menu (the three-line button at the top right) and choose **Question banks** (administrators and bank managers see it). Under **Import Excel**, leave **New bank** selected, click **Choose Excel file** and pick your workbook (see [Excel question bank format](#excel-question-bank-format)). Check the preview, then click **Create the question bank**.
2. Click **Start an exam** in the message that confirms the import (or choose **Home** in the menu). In the **Start an exam or practice** panel every sheet is ticked at first: untick the ones you do not want, then click **Start exam** in the bar at the bottom of the panel (**Count** and **Shuffle** in the same bar set how many questions are asked and in which order). **Start practice** next to it starts a [practice](#practice) with the same questions instead.
3. Answer the questions, then click **Submit**. The result page shows the score and, for each wrong question, the correct answer and the explanation (**All questions (n)** shows every question).
4. To see the exam again later: open the menu, choose **Study records** and click the time of the exam.

A new bank is visible to you and to the administrators. Other users see it once they have **All question banks, including ones imported later**, or once an administrator ticks it for them in [User management](#users-and-sign-in).

### The layout

- Every page except the exam has a slim header: the Xexam logo at the top left (it takes you to Home); at the top right your username (administrators with an **Administrator** tag, **Admin** on a phone) and the three-line menu button.
- The menu shows only the pages of your role:
  - **Home**, **Study records** and **Settings** for everyone;
  - **Question banks** for administrators and bank managers;
  - **User management** and **System log** for administrators.
- Under a divider follow **Xexam v0.7**, which opens the [About page](#about-xexam), then your username with your role (**Administrator** or **User**, plus **Bank manager**) and **Sign out**.
- The menu closes after a choice, a click outside it or the Escape key.
- On wide screens the page content is centered and at most 1120 px wide.
- The exam and the practice are focused full-screen pages without the header.

### Question banks

Here administrators and bank managers import Excel workbooks. Users without the permission do not see the page in the menu; opening it directly shows **Not allowed** with **Back to Home**.

- **Import Excel**: pick **New bank** or **Update a bank** (two small buttons next to the title; their tooltips explain them; for an update you choose the bank under **Bank to update**), then click **Choose Excel file** or drop the file anywhere on the panel. The hint line ends in a link to the **Excel format guide**.
- **The bank list**, one compact row per bank: its file, number of questions and sheets and last update, with **Update** and **Delete**. Its sheets are folded under the arrow on the right of the row.
  - An **administrator** sees **All question banks**, each with who imported it (**uploaded by sam**, **uploaded by you**), and may update or delete any of them.
  - A **bank manager** sees **Your question banks**: only the banks they imported, which they may update or delete. Banks an administrator gave them to study are on Home.
- **Import history**: one line per import (an administrator sees all imports, a bank manager those of their own banks).
- **Excel format and import guide**, folded until you open it, is the short version of [Excel question bank format](#excel-question-bank-format).

The steps and rules of an import are in [Importing question banks](#importing-question-banks).

![Question banks page as an administrator: the preview of an update of "Kitchen basics" with a warning about a differing Japanese answer, and the bank list with who uploaded each bank](docs/en/screenshots/desktop-5-banks.png)

The screenshot shows the preview of an update of "Kitchen basics" with a newer file, `Kitchen basics (v2).xlsx`, before anything is saved: the chips (1 added, 1 updated, 1 retired), the question that would be retired, and the warning that the Japanese answer of one row differs from the English one (the row is imported with the English answer). Below it is the bank list under **All question banks**: "Kitchen basics" was uploaded by the bank manager sam.

### Home

Before there is anything to practise, the **Start an exam or practice** panel shows a short note instead:

- an administrator, or a bank manager without any bank, gets **Go to Question banks**;
- a user who has not been given a question bank yet reads: “No question bank has been made available to you yet. Ask an administrator to give you access to one; it appears here as soon as they do.”

After that, Home has:

- **Unfinished exam** or **Unfinished practice**: a compact banner at the top for what you left with **Save and exit**: its title with **Saved in this browser only**, then its question banks, how many questions are answered (in a practice: checked) and the time used. **Continue** picks it up where you stopped; **Discard** throws it away. Your other devices do not see it. Starting something new while one is saved asks first.
- **Start an exam or practice**:
  - In the top-right corner of the panel you choose where the questions come from: **All** (every question in the selected sheets) or **Wrong answers** (only the questions you still need to review). Each shows its number of questions. When the notebook has nothing to review in the selected sheets, **Wrong answers** cannot be chosen and one line under the title says why; while **Wrong answers** is chosen, that line says how a question leaves the notebook. On a phone the two buttons stay next to the title; in English and Japanese they show only their icon and number, in Chinese they keep their short words.
  - **Banks and sheets**: each bank is one card with its sheets as rows inside.
    - **Select all** and **Clear** are on the right of the heading row.
    - The bank's checkbox selects or clears all its sheets, and you can also tick single sheets. While only some are selected, the checkbox shows − and the card says, for example, **1 of 2 sheets selected** (also while the bank is folded).
    - The arrow button on the right of a bank's row folds or unfolds its sheets. With at most two banks they start unfolded, with more they start folded; your choice is kept until you reload the page.
    - A badge such as **2 to review** shows how many notebook questions a bank or sheet has.
  - The bar at the bottom of the panel: **Start exam** and **Start practice**, the scope (for example **12 questions in scope**, or **11 of 12 in scope, picked at random**) and two options, **Count** (the number of questions) and **Shuffle** (the question order). Both buttons use the same scope. The bar stays at the bottom of the screen while the panel is in view.
  - **Count**: when it is smaller than the scope, the questions are picked at random from the whole scope. **Shuffle** only decides whether they are asked in random order or in bank order.
- **Latest result**: the score of your latest exam or practice with its **Exam** / **Practice** tag, **View details** and **All study records**.
- **Overview**: how many question banks, usable sheets, active questions and notebook questions you have, with **Review wrong answers** (when the notebook has questions) and, for administrators and bank managers, **Question banks**.

![Home page on a desktop](docs/en/screenshots/desktop-1-home.png)

The unfinished exam as a compact banner; **All** and **Wrong answers** in the corner of **Start an exam or practice**; **Select all** and **Clear** on the row of **Banks and sheets**; the two demo banks as unfolded cards with their sheets; the bar with **Start exam**, **Start practice**, the scope, **Count** and **Shuffle**; on the right, the latest result and the overview.

### The exam page

The screenshot at the top of this page shows it.

- At the top: **Question 4 of 12** (for example), the time used, and **Save and exit**, which keeps your answers and the time used so you can continue later from Home in the same browser.
- **No right/wrong feedback and no score while you answer.**
- **English** / **中文** / **日本語** switches the question text. Switching changes only the text: your answers, the option letters, the timer and the grading stay the same. The option buttons of hotspot questions switch too.
  - Each question has a base language that is always there: **English** for questions from the six-column layout, **日本語** for the 3- and 4-column layouts (in the 3-column layout this button shows column A, whatever language it is in).
  - A language that a question does not have, or whose text does not line up with the base (other options or items), is greyed out for that question; tapping it says why.
  - Your choice is saved as a setting and is also where the answers and explanations start after you submit.
- **Previous** and **Next** move between questions (on the last question, **Next** becomes **Submit**). **Mark to revisit** marks a question you want to come back to.
- **Answer sheet**: one tile per question. Solid = answered, half-filled = a hotspot question partly answered, hollow = not answered, small yellow square = marked to revisit. Click a tile to jump there. Below the tiles: **Answered 3 of 12** and **Submit**.
- **Submit** asks for confirmation; unanswered questions count as wrong.
- Grading: a single-choice or multiple-choice question is correct only when you chose exactly the correct options; a hotspot question is correct only when every item is right.
- Keyboard (desktop): `←` `→` previous / next question, `A`–`H` choose an answer (choice questions), `M` mark to revisit.

### Practice

A practice uses the same page as the exam, but checks every answer right away. Start it with **Start practice** on Home, next to **Start exam** and with the same banks, sheets, **Count** and **Shuffle**.

- A **Practice** tag sits next to **Question 4 of 12**.
- **How an answer is checked**:
  - a single-choice question is checked as soon as you pick an option;
  - a multiple-choice or hotspot question has **Check answer**, which works once something is chosen. For a hotspot question one item is enough; items left open count as wrong.
- **After checking**, the question is locked and shows:
  - **Correct** or **Wrong**;
  - the options marked as in the answers and explanations: the correct ones solid green, a wrongly chosen one solid red;
  - your answer and the correct answer;
  - the explanation and the memory aid, in the language of the page's **English** / **中文** / **日本語** switch.
- Auto-advance never applies in a practice: you move on with **Next**.
- The answer sheet fills checked questions green (correct) or red (wrong), and says, for example, **Checked 4 of 12 · 3 correct**.
- **Finish practice** (below the answer sheet, and on the last question) asks only when some questions are not checked yet. Those count as not answered (wrong), even when an answer is chosen.
- **Save and exit** works as in an exam; Home then shows **Unfinished practice** with how many questions are checked.
- A finished practice opens the **Practice result** page, is saved in **Study records** as a **Practice** record, and counts in the wrong-answer notebook like an exam.

![The practice page after Check answer on a multiple-choice question answered wrongly: the Practice tag, the verdict, the chosen wrong option solid red, the correct options solid green, the explanation and the memory aid](docs/en/screenshots/desktop-11-practice.png)

A multiple-choice question after **Check answer**: the **Practice** tag, the verdict **Wrong**, the wrongly chosen option in solid red, the correct options in solid green, then the explanation and the memory aid.

### The result page

The score, the number right and wrong, and a strip with one box per question. A practice shows **Practice result** as its title.

**Answers and explanations** shows **Wrong only (n)** or **All questions (n)**. Each question card shows:

- your choice and the correct answer: correct options solid green, a wrongly chosen option solid red, each with its **Correct answer** / **Your choice** tag (hotspot statements sit on a neutral background; only their choices are coloured);
- its own **English** / **中文** / **日本語** switch. It starts at your question language and changes only that card: the question, the options, the explanation, the memory aid and the note. Languages the question does not have are greyed out;
- the explanation and the memory aid. A six-column question has them in each language; if one language lacks them, another language's text is shown. A 3- or 4-column question has one explanation, shown whatever the language;
- how often you have answered the question so far.

If questions were deleted (with their bank) while you took the exam, a note says so: the score and the saved record cover only the other questions.

Buttons: **Retry wrong answers only**, **Take it again** (**Retake practice** after a practice), **View in study records** and **Back to home**. Retrying and retaking start the same kind as the result: an exam again as an exam, a practice as a practice.

![Result page on a desktop](docs/en/screenshots/desktop-3-result.png)

An exam with two wrong answers, **Wrong only** open: a choice question with the chosen option in solid red and the correct one in solid green, its own **English** / **中文** / **日本語** switch, and the explanation and the memory aid in the interface language.

### Study records

- **Overview**: the number of records (**Records**, or **Exams** / **Practice sessions** when the type filter says so), **Average accuracy**, **Best accuracy** and small bars for the latest results.
- **Filters**: **Type** (**All types**, **Exams**, **Practice**), **Question bank**, a date range with **From** and **To**, and **Accuracy** (**Any accuracy**, **Below 60%**, **60–79%**, **80% and above**). **Clear filters** resets them.
- **Records**: one row per exam or practice (on a phone: one card) with its **Exam** / **Practice** tag, the time, the question banks, how many questions were answered, the time used, the number correct and the accuracy. Click the time to open the record; **Delete** removes it.
- **Exam record** / **Practice record** (one record): the score and the facts (with **Type**), then every question with your answer, the correct answer, the explanation and the memory aid, each card with its own language switch. **Wrong only (n)** / **All questions (n)** switches between the wrong questions and all of them. **Retry wrong answers** starts the questions you got wrong; **Retake this exam** (**Retake this practice**) starts the same questions again, both as the same kind as the record; **Delete this record** removes it. Exams taken before v0.5 show only the score, because v0.4 did not store the answers.
- **Review wrong answers** (top right) opens **Wrong-answer review**: every question in your wrong-answer notebook, with how often you answered it and how many more correct answers in a row take it out. You can narrow it to one question bank, and **Practise these** starts a practice with them.
- Deleting: deleting one record keeps the per-question statistics, so the wrong-answer notebook does not change. **Delete all** deletes every record (exams and practice); tick **Also clear the wrong-answer notebook and per-question stats** to clear those too. Every delete asks for confirmation and cannot be undone.
- Xexam keeps your latest 500 records (exams and practice); older records are removed automatically.

![Study records on a desktop: overview, filters with the type filter, and the list of exam and practice records](docs/en/screenshots/desktop-4-records.png)

### Wrong-answer notebook

Questions you answer wrong, in an exam or in a practice, go into your wrong-answer notebook and leave it after you answer them correctly **2 times in a row** (in either). Use it from Home (**Wrong answers** in the top-right corner of **Start an exam or practice**), from **Study records** → **Review wrong answers** → **Practise these**, or right after an exam with **Retry wrong answers only**.

### Settings

- **Account**: your **Username** and **Role** (with **Bank manager** if you have that permission), and **Change password**. A bank manager also reads what the permission allows.
- **Appearance** (the theme): **System** (the default; follows the light or dark setting of your device), **Light** or **Dark**.
- **Language** (the interface language): **English** (the default), **中文** or **日本語**. It changes the menus, buttons and messages everywhere, the exam page and the error messages from the server included. The question text has its own switch on the exam page.
- **Answering** → **Auto-advance to the next question** (off by default, exams only): after you answer a single-choice question, or every item of a hotspot question, the next question opens by itself. Multiple-choice questions never move on by themselves. While it is on, the exam page shows **Auto-advance on** at the top.
- **Reset to defaults**: theme, interface language and auto-advance go back to their defaults. The question language is kept.

Settings belong to your account and are saved on the Xexam server, together with the question language you last chose. So they survive restarts and image updates, clearing the browser cache, and switching to another browser or device; every user has their own. Only an unfinished exam or practice stays in the current browser.

<table>
<tr>
<td width="50%"><img src="docs/en/screenshots/desktop-6-settings.png" alt="Settings page on a desktop"></td>
<td width="50%"><img src="docs/en/screenshots/desktop-7-dark.png" alt="Exam page in the dark theme"></td>
</tr>
<tr>
<td>Settings: <b>Account</b>, <b>Appearance</b>, <b>Language</b> and <b>Answering</b>.</td>
<td>The <b>Dark</b> theme, here on the exam page.</td>
</tr>
</table>

### About Xexam

The version entry of the menu, **Xexam v0.7**, opens **About Xexam**: the **Version**, the CPU architecture of the **Server**, the **License**, and **What’s new**, which is the [changelog](#changelog) of this README in the interface language. The newest version is open; the earlier ones are folded under **Earlier versions**.

![About Xexam on a desktop, with What’s new showing v0.7](docs/en/screenshots/desktop-8-about.png)

### Users and sign-in

Since v0.6 everyone signs in. There are three kinds of account:

- **Administrator**: everything, including every question bank, **User management** and the **System log**.
- **User** with the **Bank manager** permission: imports new question banks and updates or deletes the banks they imported, but not anybody else's.
- **User**: takes exams and practises with the question banks an administrator gave them.

Each user has their own study records, wrong-answer notebook, unfinished exam and settings.

![User management: administrators, a bank manager who sees all question banks, a user with one question bank and a disabled user](docs/en/screenshots/desktop-10-users.png)

User management with two administrators (admin and lee), sam (**Bank manager**, all question banks), alex (one question bank, General knowledge) and kim (disabled).

- **First start** (a new install, or the first start after upgrading from v0.5 or older): sign in with `admin` / `admin` (the sign-in page reminds you of it). Xexam then asks for a new password (8–128 characters). The study records, notebook and settings from before v0.6 belong to this administrator.
- **User management** (menu, administrators only): one row per user with role, status (**Normal**, **Disabled**, **Locked · … left**, **3 tries left**, **Must choose a new password**), **Bank manager** when the user has that permission, created, last sign-in and the number of study records (exams and practice). Actions: **Edit**, **Reset password**, **Disable** / **Enable**, **Delete** and, when needed, **Unlock**. A user created or reset by an administrator chooses their own password at the next sign-in. You cannot disable, demote or delete yourself, and the last enabled administrator cannot be removed. The list refreshes itself while it is open, so a lock that starts shows up.
- **Which question banks a user sees**: each user's row says **Question banks: all, including ones imported later**, **Question banks: 1 · General knowledge** or that there are none yet; a bank manager's own uploads are listed below it. **Change** opens **Question banks of alex** (for example): choose **All question banks, including ones imported later**, or tick single banks (each with who uploaded it). The change applies at once and the user stays signed in. Administrators always see every bank. Taking a bank away keeps the user's study records as history.
- **Add user**: username, initial password, role, the question bank checklist (nothing ticked: a new user sees no question bank until you choose some) and **Bank manager**. A new account is enabled. The last two are greyed out for an administrator, who sees and manages everything anyway. **Edit** changes the username, the role, **Bank manager** and **Account enabled**. Taking the **Bank manager** permission away applies at the user's next page change.
- **Deleting a user** also deletes their study records (the dialog says how many), wrong-answer notebook and settings. Question banks they imported are not deleted: they move to you, the administrator who deletes the user.
- **Wrong passwords**: 5 wrong passwords for an account within 15 minutes lock its sign-in for 1 minute; after that, every 3 more lock it again, for 5 minutes, 15 minutes, 30 minutes, 1 hour and then 1 day. The lock belongs to the account, not to the typed name, and guessing other usernames cannot wear it away. Browsers that signed in to the account before are counted separately, so guesses from elsewhere do not lock them out. A successful sign-in, **Unlock**, a password reset by an administrator, a restart of the backend, or 24 hours without a new mistake start over with 5 tries. The sign-in page says how long to wait.
  - In **User management** a short note under the row says which browsers are locked, until when, and how many locks in a row there were. **Unlock** gives 5 tries again at once, without asking; it is also offered while the user still has tries left.
- **Change your password**: **Settings** → **Account** → **Change password**; your other signed-in browsers are signed out.
- **Forgot the administrator password**: `docker compose exec backend xexam-backend reset-password admin` prints a new temporary password, which must be replaced at the next sign-in. It works while Xexam runs. To clear a lock at once, restart the backend: `docker compose restart backend`.

<table>
<tr>
<td width="50%"><img src="docs/en/screenshots/desktop-9-signin.png" alt="The sign-in page"></td>
<td width="50%"><img src="docs/en/screenshots/desktop-12-user-banks.png" alt="The dialog Question banks of alex: All question banks, including ones imported later, or a checklist of banks with who uploaded each, General knowledge ticked"></td>
</tr>
<tr>
<td>The sign-in page.</td>
<td><b>Question banks of alex</b>: <b>All question banks, including ones imported later</b>, or single banks with who uploaded each; here only General knowledge.</td>
</tr>
</table>

### System log

**System log** (menu, administrators only; everybody else gets **Not allowed**) shows what the backend has logged since it last started, newest at the bottom: startup, sign-ins, changes to users, rejected requests or uploads and so on.

- The page reads new lines every 1.5 seconds while it is open and follows them as they arrive. Scroll up to read, and **Jump to latest** takes you back down.
- **Pause** stops reading new lines for a while; **Resume** goes on.
- **Level** shows **All** lines, only **Warnings** (and errors) or only **Errors**; **Search the log** filters by text.
- Only the last 2000 lines are kept, in memory only: the list is empty again after a restart. For older lines, run `docker compose logs backend` on the server.
- The lines name users by their id and never contain passwords, session tokens, cookie values or question text. Rejected requests and failed sign-ins that come in bursts are summed up once a minute, so nobody can push the other lines out without signing in.

![The System log page: startup, sign-in and failed sign-in lines with the level filter, the search and Pause](docs/en/screenshots/desktop-13-logs.png)

The System log with lines from the startup, sign-ins, failed sign-ins and a sign-in lock with their summaries, the **Level** filter, the search field and **Pause**.

### On an iPhone

- Open the service address in Safari on your iPhone (first follow [Opening Xexam from your phone or another computer](#opening-xexam-from-your-phone-or-another-computer)). It needs **iOS 15.4 or later**.
- Tap the Share button → **Add to Home Screen**. After that it opens full-screen like a standalone app, with an Xexam icon in the style of an answer sheet.
- The three-line button at the top right opens the same small menu as on a computer.
- On Home, **All** and **Wrong answers** stay next to the **Start an exam or practice** title (in English and Japanese with only their icon and number). The bar at the bottom of the screen has two rows: the scope, **Count** and **Shuffle** on the first; **Start exam** (the wider one) and **Start practice** on the second.
- On the exam and practice page, **Previous**, **Next** and **Mark to revisit** stay at the bottom, and the answer sheet sits below the question with **Answered x of n** and **Submit** (or **Finish practice**) above the tiles.
- Study records are shown as compact cards.
- Buttons and other touch targets are at least 44 px; the safe areas for the notch and the bottom bar are handled.

Screenshots on an iPhone (390 px wide):

<table>
<tr>
<td width="25%"><img src="docs/en/screenshots/iphone-1-home.png" alt="Home page on an iPhone"></td>
<td width="25%"><img src="docs/en/screenshots/iphone-2-quiz.png" alt="Exam page on an iPhone"></td>
<td width="25%"><img src="docs/en/screenshots/iphone-3-menu.png" alt="The menu of an administrator open on an iPhone"></td>
<td width="25%"><img src="docs/en/screenshots/iphone-4-records.png" alt="Study records on an iPhone"></td>
</tr>
<tr>
<td>Home: the scope, <b>Count</b> and <b>Shuffle</b>, with <b>Start exam</b> and <b>Start practice</b> below them, at the bottom of the screen.</td>
<td>Exam: <b>Previous</b>, <b>Next</b> and <b>Mark to revisit</b> stay at the bottom.</td>
<td>The menu of an administrator, opened from the three-line button.</td>
<td>Study records: each record is a compact card.</td>
</tr>
</table>

---

## Excel question bank format

One workbook (`.xlsx`) is one question bank. Each sheet is a group of questions, one question per row. Xexam recognises three layouts by their content, sheet by sheet, so there is nothing to set:

| Layout | Columns | Base language |
|---|---|---|
| [3 columns](#3-column-layout-the-simplest) | A question, B Chinese translation, C answer and explanation | The language of column A (shown under **日本語**) |
| [4 columns](#4-column-layout-with-english) | A English, B Chinese, C Japanese question, D answer and explanation | Japanese (column C) |
| [6 columns](#6-column-layout-three-languages) | A English, B Chinese, C Japanese question; D English, E Chinese, F Japanese answer and explanation | English (columns A and D) |

The base language is the one the question is read from: its type, options, items and answer, and how Xexam recognises it again at the next import.

### 3-column layout (the simplest)

Here is one complete multiple-choice question; it fills one row of a sheet (it is the same example as in **Excel format and import guide** on the **Question banks** page):

| A | B | C |
|---|---|---|
| Which two of the following are fruits?<br>A. Carrot<br>B. Potato<br>C. Apple<br>D. Onion<br>E. Banana | (empty) | 【答案】C、E<br>【解析·怎么选】Apples and bananas are fruits; the others are vegetables.<br>【记忆口诀】Sweet ones are fruit. |

- **Column A**: the question.
- **Column B**: the Chinese translation (optional; leave it empty if there is none).
- **Column C**: first the answer after `【答案】` ("answer"); then, if you like, the explanation after `【解析·怎么选】` ("explanation: how to choose") and a memory aid after `【记忆口诀】` ("memory aid"). In the 3- and 4-column layouts these headings are written with these Chinese words, or in Japanese (`【正解】`, `【解説】`). English headings work only in the 6-column layout.

To start a new line inside a cell, press Alt+Enter (Windows) or Control+Option+Return (Mac). A first row with a heading such as `Question` or `题目` in column A is skipped.

**If your questions are in only one language** (English, Chinese or Japanese), use this layout: the question in column A, column B empty. You do not need any Japanese. On the exam page these questions always show the text of column A, whichever question language is chosen (their **English** and **中文** buttons are greyed out).

**Writing a choice question**

- Write the options under the question text, one per line, in alphabetical order starting with `A.`, at least two: `A. text`.
- One letter in the answer makes a single-choice question, several letters a multiple-choice question: `【答案】C` or `【答案】C、E`.

**Writing a hotspot question** (a list of statements, each answered with one choice)

- The first line of column A is `HOTSPOT` (or `ホットスポット`); the question text follows on the next line.
- Write each item on its own line as `1. text　Yes / No`: after the text comes a **full-width space** (`　`) or a tab, then the item's choices separated by ` / ` (a slash with a space on each side). Any words work as choices, for example `Preparation / Cooking / Serving`.
- The answer gives the choice for every item: `【答案】1 Yes；2 No`. Write one of that item's choice words; for yes / no items, `是`/`否` and `はい`/`いいえ` work too. If an answer is not one of the item's choices, the question is listed in the preview as unreadable, with the reason.

For example, this hotspot question (columns A and C; column B empty; the gap before `Yes / No` is a full-width space):

```
HOTSPOT
For each statement, choose Yes or No.
1. The sun rises in the east.　Yes / No
2. The moon is a star.　Yes / No
```

```
【答案】1 Yes；2 No
【解析·怎么选】The sun rises in the east. The moon only reflects sunlight; it is not a star.
```

### 4-column layout (with English)

This is the 3-column layout moved one column to the right, with English in a new column A. It is meant for workbooks whose questions are written in Japanese in column C; Xexam recognises the layout by the Japanese in column C and the answers in column D.

| Column | Contents |
|---|---|
| **A** | The **English** question (may be left blank). For a hotspot question, the first line is `HOTSPOT` and each item ends with English choices such as `Yes / No` |
| **B** | The **Chinese** translation (may be left blank). The `【中文翻译】` ("Chinese translation") heading is optional |
| **C** | The **Japanese** question. The question, the options and the answer are taken from this column: the question id, the option letters and the hotspot items all come from here. A hotspot question starts with `ホットスポット` |
| **D** | The answer and the explanation, written like column C of the 3-column layout |

Columns A, B and C are **the same question in three languages**, and their structure must **correspond exactly, one to one**: the same question, the same option letters, and the same number and order of items. Item 1 of column A = item 1 of column B = item 1 of column C, and option A = option A = option A. The answer is written only once (column D) and is shared by all three languages.

**The choices of hotspot questions are written in each language's own column**: at the end of each item, write the choices in that column's language; the j-th choice in each of the three languages has the same meaning. For example, for the same item:

| Column | How the item is written |
|---|---|
| A English | `1. Wash and cut the vegetables.　Preparation / Cooking / Serving` |
| B Chinese | `1. 清洗并切好蔬菜。　准备 / 烹饪 / 上菜` ("1. Wash and cut the vegetables. Preparation / Cooking / Serving") |
| C Japanese | `1. 野菜を洗って切る。　下ごしらえ / 調理 / 盛り付け` (same meaning as above) |

The answer for an item can then be written with its choice word in any of the three languages (for example `Cooking`, `調理` or `烹饪`), because the choices correspond one to one by position. When you switch language on the exam page, the question text, the options and the choice buttons of hotspot questions all change to the text of the corresponding column. If a column has no choices written (or their number does not match the Japanese), the question is still available in that language; only the choice buttons show the Japanese choices.

The explanation and the memory aid (column D) exist once and are shown whatever language is chosen. Workbooks in the 3-column layout can still be uploaded; they just have no English.

### 6-column layout (three languages)

Questions **and** answers in three languages: each language has its own question column and its own answer column, so the explanation and the memory aid can be read in English, Chinese or Japanese too.

| Column | Contents |
|---|---|
| **A** | The **English** question. The question type, the options and the hotspot items are taken from this column, and the question is recognised by it |
| **B** | The **Chinese** question (optional), with the same options or items as column A |
| **C** | The **Japanese** question (optional), with the same options or items as column A |
| **D** | The **English** answer and explanation: `【Answer】B`, then `【Explanation】` and `【Memory aid】`. The answer key is taken from this column |
| **E** | The **Chinese** answer and explanation (optional): `【答案】B`, `【解析·怎么选】`, `【记忆口诀】` |
| **F** | The **Japanese** answer and explanation (optional): `【正解】B`, `【解説】`, `【覚え方】` |

Here is one complete single-choice question in this layout (made up, like every example here; it is the same as in the guide on the **Question banks** page):

| Column | Cell |
|---|---|
| A | Which planet is known as the Red Planet?<br>A. Venus<br>B. Mars<br>C. Jupiter<br>D. Saturn |
| B | 哪颗行星被称为“红色星球”？<br>A. 金星<br>B. 火星<br>C. 木星<br>D. 土星 |
| C | 「赤い惑星」と呼ばれる惑星はどれですか。<br>A. 金星<br>B. 火星<br>C. 木星<br>D. 土星 |
| D | 【Answer】B<br>【Explanation】Iron oxide (rust) in the dust on its surface makes Mars look red.<br>【Memory aid】Mars rusts red. |
| E | 【答案】B<br>【解析·怎么选】火星表面的尘土含有氧化铁（铁锈），所以看起来是红色的。<br>【记忆口诀】火星生锈红彤彤。 |
| F | 【正解】B<br>【解説】火星の表面の砂には酸化鉄（さび）が含まれているため、赤く見えます。<br>【覚え方】火星はさびて赤い。 |

**English is authoritative.**

- The question type, the options, the hotspot items and the question's identity come from column A, the answer key from column D. A row without an English question in A or an English answer in D cannot be read; the preview lists it with the reason.
- Columns B and C must line up with column A (the same option letters, the same number of items). A language that does not line up is greyed out for that question on the exam page.
- When the answer in column E or F means something else than column D, the row is still imported with the English answer, and the preview warns, for example **1 answer differs from the English answer**, with the rows under **Show the rows**. Answers are compared by meaning: the same letters, or the same choice for each hotspot item (`Yes`, `是` and `はい` count as the same).

**Headings and answers**

- Each answer column may use the headings of any of the three languages: `【Answer】` `【Explanation】` `【Memory aid】` (or `【Mnemonic】`); `【答案】` `【解析·怎么选】` `【记忆口诀】`; `【正解】` `【解説】` `【覚え方】`.
- English answers are separated by `, ` and `; `: `【Answer】B, C`, or for a hotspot question `【Answer】1 Yes; 2 No`.
- A note in parentheses after the answer, in half-width or full-width parentheses, is not part of the answer; it is shown as **Note** on the answer cards, for example `【Answer】C, D (fairly sure)`.

**Hotspot questions**

- The first line of column A is `HOTSPOT`. A first line that starts with `HOTSPOT:` works too: the marker is removed and the rest of the line stays part of the question.
- Each item ends with its choices, for example `1. The sun rises in the east.　Yes / No`. Put a tab or a full-width space (`　`) before the choices. Single-word choices may also follow a plain space (`1. Water is wet. Yes / No`), but choices of several words (category names such as `Main course / Side dish`) need the tab or the full-width space.
- Columns B and C write the same items with their own choices, as in the 4-column layout; yes / no words of any language work in every answer column.

**What else to know**

- Xexam recognises this layout by the answer headings in column E or F and the Japanese question in column C (at least one row of a sheet needs one).
- Every question shows its explanation in the language chosen on its card or on the practice page. When a language has none, another language's text is shown.
- **Import a six-column file as a New bank.** The 3- and 4-column layouts recognise a question by its Japanese text, the 6-column layout by its English text. Updating a bank that came from a 3- or 4-column file with a 6-column file would therefore treat every question as new: all old questions would be retired, and the new ones would start without answer history.

### Questions that cannot be read

Questions that cannot be read are **never guessed**: they are listed in the preview with the reason (usually the answer column does not contain `【答案】…` or `【Answer】…`, or the answer letter is not among the options), and they are never silently marked wrong.

---

## Importing question banks

### Who may import

- **Administrators** import new banks and update or delete every bank.
- **Bank managers** (users with the **Bank manager** permission, set in [User management](#users-and-sign-in)) import new banks, which then belong to them, and update or delete only the banks they imported. Banks of administrators and of other users stay out of their reach.
- Other users do not see the **Question banks** page.

### New bank or update

A question bank is one imported Excel workbook. On **Question banks** → **Import Excel** you choose where the file goes with two small buttons next to the title (their tooltips explain them):

| Choice | What happens |
|---|---|
| **New bank** | A new bank is created, named after the file (without `.xlsx`; ` (2)`, ` (3)`, … is added if the name is taken). Existing banks are not changed. The same workbook can be imported twice as two separate banks; their study records do not mix |
| **Update a bank** | The file replaces the questions of the bank you pick under **Bank to update** (a bank manager can pick only their own banks). Questions missing from the file are retired **in that bank only**. The bank keeps its name and its owner |

Use **New bank** for a different workbook, and **Update a bank** for a newer version of a workbook you have already imported. Always upload the **whole** workbook, not only the new questions. A six-column workbook replacing a 3- or 4-column bank is a different workbook: import it as a **New bank** (see [6-column layout](#6-column-layout-three-languages)).

A new bank is visible to the person who imported it, to the administrators, and to users with **All question banks, including ones imported later**. An administrator gives it to other users in **User management**.

### Preview, then confirm

After the upload Xexam reads and checks the file and shows a preview: small chips with how many questions will be **Added**, **Updated**, **Restored** or **Retired** and how many are **Unchanged** and **Unreadable** (counts of zero are hidden), one line per sheet with its numbers, and for every row that could not be read, the row number and the reason, folded under the line of its sheet (for example **2 unreadable**). **Nothing is written until you confirm** (**Create the question bank** or **Confirm import**; **Cancel, do not import** drops it). The preview also warns you when:

- you create a new bank from a file with the same name as the file an existing bank was imported from (it offers to update that bank instead);
- you update a bank with a file whose name differs from the last import;
- questions would be retired. It lists some of them, and asks once more before the import;
- in a six-column file, the Chinese or Japanese answer of a row differs from the English one (the row is imported with the English answer).

Every person who imports has their own preview, kept in memory for 30 minutes. All open previews share a memory budget: when several large files are being previewed at the same time, the oldest preview of another person can be dropped. Confirming a preview that was dropped or is older than 30 minutes says that it has expired: upload the file again.

### Import rules

The file is treated as the **complete content of the bank** it goes to. Other banks are never touched.

| Case | Result |
|---|---|
| A question is in the file but not in the bank | **Added** |
| Same question, but other content (other options, answer, explanation, translation) changed | **Updated** (answer history is not affected) |
| A question is in the bank but cannot be found in the file | **Retired**: it stays saved with its answer history, but no longer appears in exams or practice |
| A question that was retired earlier appears again | **Restored**, and its wrong-answer notebook entries come back with it |
| The sheet name, the question text or the first option changed | Counts as **a new question**: the old one is retired, and the new one starts without answer history |
| The file contains not a single usable question | Import rejected (this prevents wiping the bank) |

A question is recognised by its worksheet name, its question text and its first option (spaces do not matter). The question text is the Japanese one in the 3- and 4-column layouts (column A in the 3-column layout) and the English one in the 6-column layout.

### Deleting a bank

**Delete** on a bank’s row asks for confirmation (for example **Delete “Kitchen basics”?**). The bank's questions (active and retired) and its import history are deleted, and it disappears from every user's question banks. The checkbox below is off by default:

- **Unchecked**: the answer history and the exam records stay. In those records, the bank's questions show as deleted.
- **Checked**, as an administrator (**Also delete this bank’s study records**): the answer history of the bank's questions and the records that no other remaining bank is part of are deleted too.
- **Checked**, as a bank manager (**Also delete your study records of this bank**): only your own answer history and records are deleted; other users' study records always stay, with the questions shown as deleted.

A delete cannot be undone; your own backup is the only way back (see [Data, backup and restore](#data-backup-and-restore)).

---

## Upgrading

Every upgrade keeps the same two images, the same port and the same volume `xexam_data`, so your data stays where it is: `docker compose pull`, then `docker compose up -d`. When the database schema changes, the first start of the new version writes a backup copy first and then migrates the database in one transaction. If the copy cannot be written (disk full, no permission), the backend refuses to start and leaves the database unchanged; `docker compose logs backend` says why.

### From v0.6 to v0.7

v0.7 moves the database from schema v3 to **schema v4** and switches it to WAL mode. It adds one optional setting, `XEXAM_SQLITE_JOURNAL` (see [Configuration](#configuration)); your `.env` can stay as it is.

Replace your `docker-compose.yml` with the new one (Option A, step 1): it lets the timestamps follow the host's time zone (a `TZ` in `.env` still overrides it) and limits the size of the container logs. The service names, the port and the volume are unchanged.

```bash
# 1. Back up the volume (stop first; the backup file holds your whole database, do not commit it)
docker compose stop
docker run --rm -v xexam_data:/data -v "${PWD}:/backup" alpine tar czf /backup/xexam-data-v06.tgz -C /data .

# 2. Pull v0.7 and start (do not use down -v)
docker compose pull
docker compose up -d

# 3. Look for the upgrade message and the name of the copy
docker compose logs backend
```

On its first start, before it answers any request, the backend:

1. **Writes a copy** of the database to `/data/backups/xexam-schema3-<YYYYmmdd-HHMMSS>.db` in the volume.
2. **Migrates** it to schema v4:
   - every existing user keeps seeing every question bank (**All question banks, including ones imported later**);
   - every existing bank belongs to the first administrator account (normally `admin`);
   - every existing study record becomes an **Exam** record;
   - nobody is a bank manager yet.
3. **Switches to WAL mode.** While the backend runs, `xexam.db-wal` and `xexam.db-shm` appear next to `xexam.db`; after a normal stop everything is in `xexam.db` and `xexam.db-wal` is empty (the two files may stay, harmless). Read [Backup and restore](#backup-and-restore) before your next backup.

The log contains a line like `Upgraded the database to schema 4. A copy of the previous version was saved first: /data/backups/xexam-schema3-20261111-090000.db (…)`.

After the upgrade:

- Decide who should be a **Bank manager**, and which question banks new users should see: a user you add from now on sees no question bank until you choose some.
- Check that <http://localhost:8765/api/info> shows `"version": "0.7"`, that every user still sees their question banks, and that **Study records** lists the earlier exams with the **Exam** tag.

**Rolling back to v0.6** means restoring that copy:

```bash
# 1. Stop v0.7 (do not use down -v)
docker compose stop

# 2. Find the copy
docker run --rm -v xexam_data:/data alpine ls -l /data /data/backups

# 3. Put the copy back as xexam.db (use the file name from step 2)
docker run --rm -v xexam_data:/data alpine cp /data/backups/xexam-schema3-20261111-090000.db /data/xexam.db

# 4. Set both images in docker-compose.yml to :v0.6, then start
docker compose up -d
```

Everything done in v0.7 after the migration is lost (new banks, imports, records, users, settings). v0.6 opens a WAL database without trouble (it switches it back to the older mode itself). If an older version did run on the migrated database and created users there, those users get **All question banks** the next time v0.7 starts. Details: [docs/en/migration.md](docs/en/migration.md) and [docs/en/database.md](docs/en/database.md).

### Upgrading from v0.4

v0.5 changed the interface, the **database schema** (v1 → v2) and, since v0.5.1, where the images come from: the new `docker-compose.yml` pulls them from Docker Hub (`docker.io/yuminspace/…`, public, no login) instead of `ghcr.io`. Replace your `docker-compose.yml` with the new one, or change its two `image:` lines to `docker.io/yuminspace/xexam-backend:latest` and `docker.io/yuminspace/xexam-frontend:latest`. The project name and the data volume `xexam_data` stay the same, so your data stays where it is.

#### Upgrade

These commands work in a Linux / macOS terminal and in Windows PowerShell.

```bash
# 1. Back up the volume (stop first; the backup file holds your whole database, do not commit it)
docker compose stop
docker run --rm -v xexam_data:/data -v "${PWD}:/backup" alpine tar czf /backup/xexam-data-v04.tgz -C /data .

# 2. With the new docker-compose.yml in place (Docker Hub images), pull the new version and start (do not use down -v)
docker compose pull
docker compose up -d

# 3. Look for the upgrade message and the name of the copy
docker compose logs backend
```

On its **first start**, before it answers any request, the backend:

1. **Writes a copy** of the current database to `/data/backups/xexam-schema1-<YYYYmmdd-HHMMSS>.db` in the volume (outside Docker: `backups/` in the data directory). An existing file is never overwritten.
2. **Migrates** the database in one transaction, through every schema step up to the current one. Everything you have becomes **one question bank**, named after the file of your latest import (without `.xlsx`; `Question bank` if there was never an import). **Question ids stay the same**, so the wrong-answer notebook, the per-question statistics and the score history stay linked to their questions. Earlier exam records are kept and belong to that bank; they have no per-question detail, because v0.4 did not store the answers. Since v0.6 all of this belongs to the administrator account `admin` (password `admin` until you change it at the first sign-in).

The log then contains a line like `Upgraded the database to schema 4. A copy of the previous version was saved first: /data/backups/xexam-schema1-20261110-140501.db (…)`. Later starts do not migrate again.

What else you will notice:

- **The interface starts in English** (v0.4 was in Chinese). Change it in the menu under **Settings** → **Language**; you set it once for every browser and device.
- The theme follows your device (light or dark) until you choose one in **Settings**.
- The question language used to be remembered by each browser. The first time a browser opens the new version, its choice moves to the server (if the server still has the default, Japanese).
- An unfinished exam saved in the browser is kept and can be continued.

Checks after the upgrade: <http://localhost:8765/api/info> shows the version you installed and the same `bankCount` as before (despite its name, this field counts the active questions, not the banks); **Question banks** shows one bank with all your sheets; **Study records** lists your earlier exams; **Study records** → **Review wrong answers** has as many questions as your wrong-answer notebook had.

#### Rolling back to v0.4

Rolling back means **restoring the copy written before the migration** and running the v0.4 images. Everything done in the new version after the migration is lost (new banks, imports, exam records, users, settings).

```bash
# 1. Stop the new version (do not use down -v)
docker compose stop

# 2. Find the copy
docker run --rm -v xexam_data:/data alpine ls -l /data /data/backups

# 3. Put the copy back as xexam.db (use the file name from step 2)
docker run --rm -v xexam_data:/data alpine cp /data/backups/xexam-schema1-20261110-140501.db /data/xexam.db

# 4. v0.4 is only on the private GHCR mirror (Docker Hub starts with v0.5.1): run docker login ghcr.io, set the two
#    images in docker-compose.yml to ghcr.io/yumin-space/xexam-backend:v0.4 and ghcr.io/yumin-space/xexam-frontend:v0.4, then start
docker compose up -d
```

Instead of step 3 you can restore your own `.tgz` from the upgrade (see [Backup and restore](#backup-and-restore)). ⚠️ **Never run v0.4 on a database that has several banks** instead of restoring the copy: v0.4 sees all banks as one, so its next import would retire the questions of every other bank, and it cuts the exam records back to 60. Details: [docs/en/migration.md](docs/en/migration.md) and section 6 of [docs/en/database.md](docs/en/database.md).

### Upgrading from older versions (v0.2.x, single Python image)

The data volume `xexam_data` is the same one. However, the old version was one container and the new version is two, and the container name / port are the same, so:

1. Run `docker compose stop` (for the old version).
2. Back up the data volume (see [Backup and restore](#backup-and-restore)).
3. Run `docker rm xexam` to remove the old version's container (**only the container, not the data volume**). The old container is called `xexam`; the new ones are `xexam-frontend` and `xexam-backend`. If you do not remove it, the old container still occupies port 8765 and the new ones will fail to start.
4. Swap in the new `docker-compose.yml`, then run `docker compose pull` and `docker compose up -d`. The first start migrates the database to the current schema, exactly as described above (with a backup copy in `backups/`).
5. Open <http://localhost:8765> and check that the number of questions, the number of wrong questions and the score history are the same as before the upgrade.

To go back to the old version: the old code is at the Git tag `python-v0.2`, and the old image `ghcr.io/yumin-space/xexam:v0.2.2` can still be pulled from GHCR (private: `docker login ghcr.io` first; it is not on Docker Hub). Restore the schema-1 copy from `backups/` (or your own backup) before you start it. See [docs/en/migration.md](docs/en/migration.md).

---

## Data, backup and restore

Data is stored in a **Docker named volume**, not in a folder you can see on your computer.

- The volume is named `xexam_data` (project name `xexam` + volume name `data`). It holds:
  - `xexam.db`, the database;
  - while the backend runs, also `xexam.db-wal` and `xexam.db-shm`: since v0.7 the database runs in WAL mode, and the latest changes may still be in `xexam.db-wal`. A normal stop (`docker compose stop`) writes them into `xexam.db` and empties `xexam.db-wal`; the two files may stay next to it, empty and harmless;
  - after an upgrade, the folder `backups/` with the copies written before each schema migration. The program never deletes those copies; remove them yourself once you no longer need them.
- Updating the image, running `docker compose down`, or deleting and recreating the container leaves **the data intact**.
- ⚠️ **Do not run `docker compose down -v`**: `-v` also deletes the volume, and the data is gone.
- On unusual storage (a network filesystem, or a Windows folder bind-mounted instead of the named volume), set `XEXAM_SQLITE_JOURNAL=delete` in `.env` to keep the older DELETE journal mode without the `-wal` / `-shm` files.

What is kept where:

| Where | What |
|---|---|
| `xexam.db` in the volume (with `-wal` / `-shm` while running) | The question banks with their owners and questions (retired ones included), the import history, the users (password hashes only), their sessions and which banks each user sees, each user's per-question statistics (wrong-answer notebook), study records (exams and practice) with their per-question detail, and settings (theme, interface language, auto-advance, question language) |
| `backups/` in the volume | The copies of the database written before each schema migration (`xexam-schema1-…`, `xexam-schema2-…`, `xexam-schema3-…`) |
| Memory of the backend only | The System log, the sign-in locks and the import previews; all of them start empty after a restart |
| Your browser (`localStorage`) | Per user: the unfinished exam or practice (question ids, answers, marks, checked questions, time used; no question text) and a copy of the settings to paint the first screen |

### Backup and restore

**Stop the app first**: after `docker compose stop` everything is in `xexam.db` (any `xexam.db-wal` left next to it is empty), so a copy is complete and consistent. A volume backup includes `backups/`. These commands work in a Linux / macOS terminal and in Windows PowerShell.

```bash
docker compose stop

# Backup: pack the volume into xexam-data.tgz in the current folder
docker run --rm -v xexam_data:/data -v "${PWD}:/backup" alpine tar czf /backup/xexam-data.tgz -C /data .

docker compose start
```

```bash
# Restore (overwrites files with the same name in the volume; removes leftover -wal / -shm files first,
# so they are not applied to the restored database)
docker compose stop
docker run --rm -v xexam_data:/data -v "${PWD}:/backup" alpine sh -c "rm -f /data/xexam.db-wal /data/xexam.db-shm && tar xzf /backup/xexam-data.tgz -C /data"
docker compose start
```

**Copying while the backend runs** is possible but needs care: copy `xexam.db` **together with** `xexam.db-wal` and `xexam.db-shm`, or make a consistent copy with SQLite itself (`VACUUM INTO`, or `.backup` in the `sqlite3` command-line tool). A copy of `xexam.db` alone can miss the latest changes.

Moving to a new computer: put the new `docker-compose.yml` in place on the new machine (no login needed, the images come from Docker Hub), run `docker compose up -d` first to create the volume, then restore the backup as described above. The backup file contains your entire database, so **do not commit it to Git** (`.gitignore` already ignores `*.tgz`). For more details (connection settings, verifying a backup) see [docs/en/database.md](docs/en/database.md).

### Migrating progress records from earlier versions (`progress.json`)

If you have a `progress.json` left over from an earlier version, use a temporary container to put it into the volume, then restart (the commands work in bash and in PowerShell):

```bash
docker compose stop
docker run --rm -v xexam_data:/data -v "${PWD}:/src:ro" alpine sh -c "cp /src/progress.json /data/progress.json && chown 10001:10001 /data/progress.json"
docker compose start
```

On restart, the scores and wrong-answer records in it are migrated into the database automatically, **only once**, and the log says `Migrated the old study records into the database` (visible with `docker compose logs`). This happens only while the database has no study records yet; existing records are never overwritten. If the file format is wrong, the log states the reason, and you can fix the file and restart to try again.

> `docker cp` is not used here: by default it writes as root, so the file would belong to root, while the program in the container runs as a non-root user (uid 10001) and may not be able to read it.

---

## Access from the internet via Cloudflare Tunnel

Use case: Xexam runs on your home LAN, `cloudflared` (Cloudflare's tunnel client) runs on a machine on the same LAN (not necessarily the same one), and you want to reach it from outside with your own domain, protected by authentication in **Cloudflare Access**.

```mermaid
sequenceDiagram
    autonumber
    actor U as You (outside)
    participant CF as Cloudflare<br/>(domain + Access)
    participant T as cloudflared<br/>(LAN)
    participant X as Xexam container<br/>(LAN, host IP:8765)

    U->>CF: Open https://xexam.your-domain.com
    CF-->>U: Require login (Cloudflare Access)
    U->>CF: Pass authentication
    CF->>T: Forward the request through the tunnel
    T->>X: HTTP request (Host is your domain, or rewritten to host IP:8765)
    X->>X: Check that Host is in the allow-list, and check Origin as well for write requests
    X-->>U: Page / data (returned along the same path)
    Note over U,X: POST requests such as uploading Excel or submitting an exam carry Origin https://xexam.your-domain.com from the browser<br/>The app compares hostnames only, so they are not blocked
```

### The three things to do

**1. Make sure cloudflared on the LAN can reach the container.** By default the container binds only to the local machine (`127.0.0.1`). Create a `.env` file next to `docker-compose.yml`:

```ini
# Let other machines on the LAN (cloudflared) reach port 8765 on this machine
XEXAM_BIND=0.0.0.0:8765
# Allow access through your domain (you can list several, comma-separated)
XEXAM_ALLOWED_HOSTS=xexam.your-domain.com
```

Then run `docker compose up -d`.

> If cloudflared and Xexam are on the **same machine**, you can leave `XEXAM_BIND` alone (keep the default local-only binding) and simply set the tunnel's target to `http://localhost:8765`.

**2. In Cloudflare, point the domain at Xexam.** Zero Trust dashboard → Networks → Tunnels → your tunnel → Public Hostname:

| Field | What to enter |
|---|---|
| Subdomain / Domain | `xexam` / `your-domain.com` |
| Service Type | `HTTP` (not HTTPS) |
| URL | `LAN-IP-of-the-Xexam-host:8765`, for example `192.168.1.20:8765` |

**3. Add an application policy in Cloudflare Access** that allows only your own account (email) to reach this domain.

### Common situations

| Your situation | How to configure it |
|---|---|
| The tunnel **passes Host through as your domain** | Just set `XEXAM_ALLOWED_HOSTS=xexam.your-domain.com`; step 1 above is enough |
| You set an **HTTP Host Header** (`httpHostHeader`) in Cloudflare that rewrites Host to `192.168.1.20:8765` | `XEXAM_ALLOWED_HOSTS=192.168.1.20` plus `XEXAM_ALLOWED_ORIGINS=https://xexam.your-domain.com`. The `Origin` the browser sends is still the public domain, so it must be allowed separately |
| You are not sure what Host actually is | Configure step 1 above first, then open the page. If it is rejected (403), the message on the page states the `Host` (or `Origin`) it saw: add that value to `XEXAM_ALLOWED_HOSTS` (or `XEXAM_ALLOWED_ORIGINS`) and recreate the container with `docker compose up -d` |
| You do not want to list domains, and Access authentication is already in front | `XEXAM_ALLOW_ANY_HOST=1` (`Host` is no longer checked; the `Origin` check on write requests still applies) |

How far this has been tested: the complete browser flow for both Host situations (a real Chrome and an HTTPS reverse proxy) was run against the old Python version. For the current stack, `tests/integration/stack_check.py` and `nginx_check.py` check at the header level that Nginx passes Host through unchanged and that the Host / Origin guard still works behind it. It has **not** been tested with real Cloudflare; for the first go-live, follow the "not sure" row above and read the 403 message.

### Things not to forget

- ⚠️ **Xexam's own sign-in is the only lock on the door.** Once you set `XEXAM_BIND` to `0.0.0.0`, or set `XEXAM_ALLOW_ANY_HOST=1`, **anyone who can reach this port reaches the sign-in page** and can try passwords (the escalating locks slow that down, but a weak password stays weak). So:
  - Let only cloudflared reach this port: **do not** set up port forwarding on the router. On Linux, the host firewall (ufw, firewalld) **cannot stop** ports published by Docker; to restrict sources, add rules in the `DOCKER-USER` chain, or set `XEXAM_BIND` to a specific LAN IP (such as `192.168.1.20:8765`). The safest setup is to run cloudflared and Xexam on the same machine and keep the default `127.0.0.1`.
  - Make sure the Cloudflare Access policy is in place and allows **only the people who should use Xexam**. A domain without a policy is open to the whole world.
  - Change the default `admin` password at the first sign-in, and give every user a password of their own.
- Your Cloudflare Access login expires after a while. After it expires, an upload or a submit fails with a message saying that your sign-in may have expired: just reload the page and sign in again. **An exam or practice in progress is stored in the browser and is not lost**.
- When a request is rejected (403), the message on the page states what `Host` / `Origin` it saw and which value should be added to which option. The same reason appears in `docker compose logs` and in the **System log** (bursts of rejections are summed up once a minute).

---

## Architecture

All the diagrams below are written in [Mermaid](https://mermaid.js.org/), which GitHub renders directly as images. See [docs/en/architecture.md](docs/en/architecture.md) for the full description.

### What connects to what

```mermaid
flowchart TB
    U["Browser<br/>(desktop / iPhone)"] -->|"HTTP / HTTPS"| N
    subgraph pub["Exposed to the outside: only xexam-frontend"]
        N["xexam-frontend (Nginx)<br/>Static frontend files · /api/ reverse proxy<br/>20 MB limit · security headers · rate limiting"]
    end
    subgraph priv["Docker private network (no published ports, no outbound internet access)"]
        B["xexam-backend (Rust / Axum)<br/>REST API · sign-in · bank permissions · XLSX parsing<br/>import flow · Host/Origin guard · System log (in memory)"]
    end
    N -->|"/api/*  Host passed through unchanged"| B
    B --> D[("Named volume xexam_data<br/>/data/xexam.db (+ -wal / -shm while running):<br/>banks, questions, users, records, settings<br/>/data/backups/: copies from before schema migrations")]
    CF["Cloudflare Tunnel + Access<br/>(set up by you, for access from outside)"] -.-> N
    X["Your Excel workbooks<br/>(one question bank each)"] -.->|"upload"| U
```

| Container / image | What is inside | Exposed? |
|---|---|---|
| `xexam-frontend` | Nginx + static frontend files (HTML / JS / CSS). Forwards `/api/` to the backend; 20 MB limit, security headers, rate limiting, timeouts | Yes: host `XEXAM_BIND` (default `127.0.0.1:8765`) → container port 8080 |
| `xexam-backend` | A single statically linked Rust executable: REST API, sign-in, Excel parsing, SQLite reads and writes, the System log | **No**: no `ports`, and the network is `internal: true` |

The browser talks to the backend only through the [REST API](docs/en/api.md); the frontend has no database access. **Grading happens in the browser**, in exams and in practice: for each question the server receives the question id, whether it was correct and (since v0.5) the answer given, which it only stores for the review. The server decides which questions count: only questions of banks the user may see.

### Data flow A: importing into a question bank (upload → preview → confirm)

```mermaid
sequenceDiagram
    autonumber
    actor You
    participant F as Frontend
    participant N as Nginx
    participant B as Backend
    participant D as SQLite
    You->>F: Choose the target (new bank or an existing bank) and an .xlsx file
    F->>N: POST /api/upload (raw bytes, header X-Bank-Target: new or a bank id)
    N->>B: Forward (Host unchanged, files over 20 MB get a 413 here)
    B->>B: Guards · may this user import into the target? · content checks · parse · compare with the target bank
    B-->>F: Preview + one-time confirmation token (nothing written to the database)
    F-->>You: Added / updated / restored / retired / unchanged / unreadable, warnings
    You->>F: Confirm (asked once more if questions would be retired)
    F->>N: POST /api/import/confirm {token}
    N->>B: Forward
    B->>D: BEGIN IMMEDIATE … owner check … COMMIT (one transaction, target bank only)
    B-->>F: Result + latest questions + bankId
```

The preview is held only in memory, one per user, for 30 minutes, and the confirmation token works once; all previews together stay within a memory budget. Leaving the Question banks page with a preview open cancels it (`POST /api/import/cancel`). Only administrators and bank managers may upload; a bank manager may update only the banks they own, which is checked again inside the write transaction. An upload without `X-Bank-Target` (older clients, the command-line `import`) goes to the bank that holds the data from before v0.5, exactly as in v0.4.

### Data flow B: taking an exam or a practice

```mermaid
sequenceDiagram
    autonumber
    actor You
    participant F as Frontend (browser)
    participant B as Backend (through Nginx)
    participant D as SQLite
    F->>B: GET /api/bank, /api/banks, /api/progress, /api/records, /api/settings
    B-->>F: The questions of the banks you may see, your study progress, records, settings
    You->>F: Pick banks and sheets, start an exam or a practice, answer
    F->>F: Save the unfinished exam or practice in localStorage after every change
    You->>F: Submit / Finish practice (a practice also checks each answer at once in the browser)
    F->>F: Grade in the browser
    F->>B: POST /api/attempt with id, correct and answer for each question, plus kind (exam or practice), sheets, mode and time
    B->>D: One transaction: update your statistics, save the record with its bank snapshot and one detail row per question, keep your newest 500
    B-->>F: Updated study progress
```

### Database: 13 tables (schema v4)

The diagram shows the tables in use. Schema v1 created `questions`, `stats`, `attempts`, `imports` and `meta`; v2 (v0.5) added `banks`, `attempt_items` and `settings`; v3 (v0.6) added `users`, `sessions`, `user_stats` and `user_settings`, after which the older `stats` and `settings` are kept but no longer used; v4 (v0.7) adds `user_banks` and the columns `users.all_banks`, `users.can_manage_banks`, `banks.owner_id` and `attempts.kind`. See [docs/en/database.md](docs/en/database.md) for every field.

```mermaid
erDiagram
    users {
        INTEGER id PK
        TEXT username "Unique, case-insensitive"
        TEXT role "admin · user"
        INTEGER all_banks "1 = sees every bank, also later ones (v4)"
        INTEGER can_manage_banks "1 = bank manager (v4)"
    }
    sessions {
        TEXT token_hash PK "SHA-256 of the session cookie"
        INTEGER user_id "User"
        INTEGER expires_at "Unix seconds"
    }
    user_banks {
        INTEGER user_id PK "User (v4)"
        INTEGER bank_id PK "A bank this user sees (v4)"
    }
    banks {
        INTEGER id PK
        TEXT name "Shown name (file name without .xlsx)"
        TEXT filename "File of the latest import"
        INTEGER owner_id "User who imported it (v4)"
    }
    questions {
        TEXT id PK "Content fingerprint of sheet + question text + first option"
        INTEGER bank_id "Bank"
        TEXT sheet "Sheet name"
        INTEGER active "1 active · 0 retired"
        TEXT data "JSON of the whole question"
    }
    user_stats {
        INTEGER user_id PK "User"
        TEXT question_id PK "Matched to questions by id"
        INTEGER wrong "Times answered wrong"
        INTEGER streak "Correct answers in a row"
    }
    attempts {
        INTEGER id PK
        INTEGER user_id "User"
        TEXT kind "exam · practice (v4)"
        INTEGER percent "Percentage (round half to even)"
        TEXT banks "Snapshot of the banks used, JSON"
    }
    attempt_items {
        INTEGER attempt_id PK "attempts.id"
        INTEGER seq PK "Position in the exam"
        TEXT question_id "Question"
        TEXT answer "Answer given, JSON (NULL = unanswered)"
        INTEGER correct "1 · 0"
    }
    imports {
        INTEGER id PK
        INTEGER bank_id "Bank"
        TEXT filename "Uploaded file name"
        INTEGER added "Added"
    }
    user_settings {
        INTEGER user_id PK "User"
        TEXT key PK "theme · uiLang · autoAdvance · questionLang"
        TEXT value "JSON text of the value"
    }
    meta {
        TEXT key PK "schema · users_known · …"
        TEXT value
    }
    users ||--o{ sessions : "user_id"
    users ||--o{ user_banks : "user_id"
    banks ||--o{ user_banks : "bank_id"
    users |o--o{ banks : "owner_id"
    banks ||--o{ questions : "bank_id"
    banks ||--o{ imports : "bank_id"
    users ||--o{ attempts : "user_id"
    attempts ||--o{ attempt_items : "attempt_id"
    users ||--o{ user_stats : "user_id"
    questions |o--o{ user_stats : "matched by id, no foreign key"
    users ||--o{ user_settings : "user_id"
```

**Question ids.** The questions of the bank that holds the data from before v0.5 keep their old ids. Every other bank stores its questions under ids scoped to the bank, so the same workbook can be imported as two banks without their records mixing.

**Why the statistics are not deleted along with the question:** retiring a question only sets `active` to 0, and the answer history is kept as it is; if the question appears in the Excel file again later, it is restored automatically. Only deleting a bank removes questions.

**How the wrong-answer notebook works:** for each user, a question is in the notebook as long as `wrong > 0` and `streak < 2` (fewer than 2 consecutive correct answers); after 2 consecutive correct answers it is moved out automatically. Exams and practice both count.

### How the pages connect

```mermaid
flowchart LR
    subgraph shell["Pages with the header and the menu"]
        H["Home"]
        BK["Question banks<br/>(administrators, bank managers)"]
        R["Study records"]
        S["Settings"]
        U["User management<br/>(administrators)"]
        L["System log<br/>(administrators)"]
        AB["About Xexam"]
        D["Exam / practice record"]
        W["Wrong-answer review"]
        RS["Result"]
    end
    Q["Exam / practice page<br/>(full screen, no header)"]
    H -->|"Start exam / Start practice / Continue"| Q
    Q -->|"Save and exit"| H
    Q -->|"Submit / Finish practice"| RS
    RS -->|"Retry wrong answers only / Take it again"| Q
    RS -->|"View in study records"| D
    R --> D
    R -->|"Review wrong answers"| W
    D -->|"Retry wrong answers / Retake"| Q
    W -->|"Practise these"| Q
```

The menu reaches **Home**, **Study records**, **Settings** and **About Xexam** from every page with a header, plus **Question banks**, **User management** and **System log** for those allowed to use them; opening one of these without permission shows **Not allowed** (User management sends you to Home). Single-choice, multiple-choice and hotspot questions look slightly different, but exams and practice all use the same page.

---

## Security notes

For the detailed threat model and every line of defense, see [docs/en/security.md](docs/en/security.md). Key points:

- **Only Nginx is exposed.** The backend publishes no port, and its network is `internal: true` (so it cannot reach the internet either).
- By default only requests addressed to the local machine are accepted (the `Host` is checked); when running behind an authenticating reverse proxy such as Cloudflare, put your domain in `XEXAM_ALLOWED_HOSTS`. Write requests also have their `Origin` checked: it must be the same host as `Host` (or be listed in `XEXAM_ALLOWED_ORIGINS`), which prevents a malicious web page from operating the app through your browser. The backend reads only the real `Host` header, never `X-Forwarded-*`.
- **Sign-in since v0.6.** Every page and API needs a signed-in account, except `/api/info` and signing in itself. Passwords are stored as Argon2id hashes; the session cookie is `HttpOnly` and `SameSite=Strict` (`Secure` only over HTTPS or with `XEXAM_COOKIE_SECURE=1`, so plain HTTP on your own network works). Wrong passwords lock signing in to that account, longer each time; the lock belongs to the account and a flood of made-up usernames cannot reset it (see [Users and sign-in](#users-and-sign-in)). Change the default `admin` password at the first sign-in. For access from the internet, keep authentication in front of it (Cloudflare Access) and never expose the port directly.
- **Question bank permissions (since v0.7)**, enforced by the backend on every route:
  - a user reads only the questions, banks, statistics and record details of the banks they were given;
  - only administrators and bank managers may import; a bank manager may update or delete only the banks they own, checked again inside the write transaction, and gets 403 for any other bank;
  - only administrators see who owns a bank.
- **System log**: kept in memory only (the last 2000 lines), readable by administrators only. Log lines name users by id and never contain passwords, session tokens, cookie values or question text. Rejected requests and failed sign-ins are summed up at most once a minute, so nobody can push the other lines out without signing in.
- Deleting a bank, a user or records is always a POST with a JSON body, runs in one transaction, and in the page always asks for confirmation; deleting study data along with a bank, or the wrong-answer notebook along with all records, is a separate checkbox that is off by default. Settings accept only known values.
- Uploaded files are checked by **content**: empty files, non-`.xlsx` files, corrupt archives, duplicate archive entries, and files containing an XML entity declaration (DOCTYPE) are rejected, with a maximum of 20 MB; parsing has an overall time limit and a memory cap.
- After an upload, the preview is kept in memory for 30 minutes (one per user, all within a memory budget), and confirming requires a one-time verification code (128-bit random) that is invalidated once used.
- Database writes happen in a single transaction: if anything fails midway, the whole thing is rolled back and no half-written data is left. Before every schema migration the backend writes a backup copy, and it does not start if it cannot. In WAL mode a commit still waits for the disk (`synchronous=FULL`), so nothing that was saved is lost on a power failure.
- Both containers run as non-root, with `cap_drop: ALL`, `no-new-privileges`, and a read-only root filesystem.
- The images contain no secrets, no database, and no question bank.

---

## Automated build and release

[`.github/workflows/ci.yml`](.github/workflows/ci.yml) is the **only** pipeline. It runs on every push to `main` or `dev`, on every pull request into `main`, and when started by hand from the Actions page:

```mermaid
flowchart LR
    C["① check<br/>VERSION · changelogs · badge · docs layout · links"]
    subgraph T["② Tests"]
        F["frontend<br/>type check · unit tests · build"]
        B["backend<br/>fmt · clippy · tests · migration check"]
        S["stack<br/>Nginx checks · full-stack end-to-end"]
        A["arm64<br/>build and start under QEMU"]
        F --> S
        B --> S
        F --> A
        B --> A
    end
    C --> F
    C --> B
    S --> P["③ Publish two images<br/>amd64 + arm64 (main only)"]
    A --> P
    P --> R["④ Create the GitHub Release,<br/>then record RELEASE_VERSION<br/>(main only)"]
```

- **Trigger:** a push to `main` (publishes), a push to `dev`, a pull request into `main`, or a manual run from the Actions page. Only `main` publishes; pushes to `dev`, pull requests and manual runs on other branches are only checked and tested, and **never publish**. A push to any other branch does not start the pipeline.
- **Two versions in `VERSION`:**
  - `CODE_VERSION` is the version of the code. Every update must raise it and add an entry for the same version at the very top of the changelog of **all three READMEs** (`README.md`, `README/README.ja.md`, `README/README.zh-CN.md`); `backend/Cargo.toml` and `frontend/package.json` must carry the same version. The images are built and tagged with it, and `/api/info` reports it.
  - `RELEASE_VERSION` is the last version that was actually published. It is **never edited by hand**: the release job writes it, together with the release badge at the top of the READMEs.
  - Step ① fails when `VERSION` does not have exactly one line of each, when the changelogs or the manifests do not match `CODE_VERSION`, or when a badge does not show `RELEASE_VERSION`.
- **Docs check (also step ①):** the seven documents exist in `docs/en`, `docs/ja` and `docs/zh-CN`, `docs/` contains nothing but these three folders, only `README.md` stays at the root (the other two READMEs are in `README/`), the compose files and `.env.example` are in `docker/`, and every relative link and image path in the READMEs and the docs points at a file that exists (links to GitHub pages such as Releases must climb exactly to the repository page).
- **Publish only if everything passes:** the two images (`xexam-frontend`, `xexam-backend`) are each published as a multi-architecture image, and the pipeline then checks that the image manifest really contains `amd64` and `arm64` and that the tag points to the digest that was just pushed. If a test fails, `latest` stays on the previous working version.
- **Published tags:** `latest`, `v<CODE_VERSION>`, `sha-<commit>`, the same on Docker Hub and on ghcr.io.
- **How the backend image gets arm64:** it is cross-compiled with zig on the build machine instead of compiling Rust under QEMU (which is an order of magnitude slower); only the final, very small alpine stage uses the target architecture.
- **Releases page:** once both images are published and verified, the pipeline automatically creates (or updates) a Release named `v<CODE_VERSION>`. Its description is taken from this version's section of the Changelog in this file ([`scripts/release-notes.py`](scripts/release-notes.py)), and a git tag with the same name is created automatically. If the images were not published successfully, there is no Release. A re-run of an older run does not mark its Release as the latest.
- **Recording the release:** the last step of the release job runs [`scripts/set-release-version.py`](scripts/set-release-version.py), which sets `RELEASE_VERSION` and the release badges, commits that to `main` as `Record release v… [skip ci]` and, when `dev` has no commits of its own, fast-forwards `dev` to the same commit. It never moves `RELEASE_VERSION` back. **Afterwards, pull both `main` and `dev`** before your next push; if the job reports that `dev` had moved on, merge `main` into `dev` (`git pull origin main` on `dev`) before the next merge to `main`.
- **Releases and images live in different places:** the images are on **Docker Hub** (`docker.io/yuminspace/xexam-frontend` and `docker.io/yuminspace/xexam-backend`, public; this is what `docker-compose.yml` pulls) and, as a private mirror, under the repository's **Packages** (ghcr.io); the Release is on the **Releases** page and lists the Docker Hub pull commands first. All of them are produced by the same build.

Publishing to Docker Hub needs two repository secrets, `DOCKERHUB_USERNAME` and `DOCKERHUB_TOKEN` (a Docker Hub access token with **Read & Write**), and optionally the repository variable `DOCKERHUB_NAMESPACE`. Without the secrets, or if `docker/docker-compose.yml` does not name the Docker Hub images of that namespace, the publish job stops before it pushes anything. On ghcr.io the images are private (the repository is private): the first time a new image name is published there, add this repository under **Manage Actions access** in the GitHub package settings with the role **Admin**, otherwise the push is rejected (`permission_denied: write_package`).

---

## Development

For the everyday commands for changing code and running tests, see [docs/en/development.md](docs/en/development.md). The most common ones:

```bash
# Backend (Rust 1.99.1; rustup installs it automatically from backend/rust-toolchain.toml)
cd backend && cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test --all-targets

# Frontend (Node 24)
cd frontend && npm ci && npm run typecheck && npm test && npm run build

# During development: backend at 127.0.0.1:8765, frontend at http://127.0.0.1:5173 (/api is proxied to the backend)
cd backend && cargo run -- serve --data ../data --no-browser
cd frontend && npm run dev

# Full-stack check (uses Docker, a temporary project name and a temporary volume, does not touch xexam_data)
python -I tests/integration/stack_check.py
```

- The backend has 302 tests and the frontend 119 unit tests. The backend tests run against the committed **legacy snapshot** in `tests/baseline/`: 172 golden responses, 49 parser fixtures and their outputs, and 85 question-id vectors. All of these fixtures are made up; a real question bank is never committed.
- Each version's behaviour has its own test files in `backend/tests/`: `v05_banks_records_settings.rs` (banks, records, settings, the schema-v2 migration), `v06_users_and_sign_in.rs` (users, sign-in, locks), `v065_bank_access_and_practice.rs` (bank access, bank managers, practice, the schema-v4 migration), `parser_six_columns.rs` (the six-column layout), `v07_wal_and_logs.rs` and `v07_log_floods.rs` (WAL mode, the System log and its burst summaries).
- Every text in the interface comes from `frontend/src/i18n/catalog/*.ts`, in English, Chinese and Japanese; a test fails if a language misses a key, and another one if a backend error message has no translation.
- The **About Xexam** page shows the changelogs of the three READMEs. They are read when the frontend is built (the `xexam-changelog` plugin in `frontend/vite.config.ts`), so a frontend build needs `README.md`, `README/README.ja.md` and `README/README.zh-CN.md` in the folder above `frontend/` (the repository root).
- Opening an older database with the current backend migrates it to schema v4, writes `backups/` next to it and switches it to WAL mode. Experiment on a copy.

---

## Directory structure

```
README.md                  This file (English); the Japanese and Chinese versions are in README/
README/                    README.ja.md (Japanese) and README.zh-CN.md (Chinese)
LICENSE                    MIT license
VERSION                    CODE_VERSION (the code, equal to the top changelog entry) and RELEASE_VERSION (the last release)
docker/                    Docker Compose files (the Dockerfiles stay in backend/ and nginx/)
  docker-compose.yml         Pulls and runs the two images (this is the only file you need)
  docker-compose.build.yml   Builds the two images from source and runs them (build context: the repository root)
  .env.example               Local configuration template (copy to docker/.env; .env is not committed)
backend/                   Rust backend (Axum + rusqlite)
  src/main.rs src/lib.rs     Command line (serve, import, reset-password, …), startup, graceful shutdown
  src/api/                   Routes, guards (Host / Origin / session / Content-Type / size), sign-in and users, JSON format
  src/auth/                  Sessions, password hashing, who may do what, the escalating sign-in throttle
  src/services/              Pending imports (one per user, shared memory budget), parser engine interface, import flow
  src/parser/                .xlsx parsing: ZIP protection, XML, the 3 / 4 / 6-column layouts, question id
  src/database/              SQLite storage, schema (v1 verbatim + v2 / v3 / v4 additions), journal.rs (WAL mode),
                             legacy progress.json migration, timestamps
    store/                     Banks, records, settings, users, schema migrations (with the backup copies)
  src/logs.rs                The System log: an in-memory buffer of the last 2000 lines, burst summaries
  src/security/ src/config/  Host / Origin checks, size-limited request body reading; environment variables
  tests/                     Unit, golden-response, parser comparison, id-vector, concurrency, and one file per version
  Dockerfile                 Cross-compiles on the build machine (zig); the runtime image is a single statically linked executable
frontend/                  TypeScript + Vite frontend (plain DOM)
  src/main.ts src/router.ts  Entry point; hash routes (#/, #/banks, #/records, #/users, #/logs, #/settings, #/about, #/quiz, #/result, …)
  src/pages/                 home, banks, records, settings, users, logs, about, auth (sign-in), forbidden (Not allowed),
                             quiz (exams and practice), result
  src/components/            Shell (header, menu), review (answer cards), lang-switch (English / 中文 / 日本語), common parts
  src/domain/                Pure rules with their tests: grading, practice, languages, records filter, import warnings, log view, …
  src/i18n/                  Interface texts in English, Chinese and Japanese (catalog/*.ts), translation of server messages
  src/api/ state/ types/ utils/ styles/
  vite.config.ts             Build settings; reads the changelogs of the three READMEs for the About page
nginx/                     nginx.conf + Dockerfile (builds the frontend, with the three READMEs -> ships only the static output)
tests/baseline/            Behavior snapshot of the legacy Python version (golden responses, parser fixtures, id vectors, sample database)
tests/integration/         Nginx behavior checks, full-stack end-to-end checks, migration and progress.json checks
docs/                      en/ (English), ja/ (Japanese), zh-CN/ (Chinese): architecture, API, database, security, deployment, migration, development,
                           each folder with its own screenshots/; en/openapi.yaml; zh-CN/history/ holds older documents (Chinese only)
scripts/release-notes.py   Generates the GitHub Release notes from the changelog (used by CI, can also be run locally)
scripts/set-release-version.py  Records a published release: RELEASE_VERSION and the release badges (run by the release job)
.github/workflows/ci.yml   The only pipeline: check -> test -> publish -> Release
```

---

## License

Xexam is released under the [MIT License](LICENSE). Copyright (c) 2026 Yumin-Space.

The license covers the software in this repository. The repository contains no exam questions: the question banks you import are your own files, and you are responsible for making sure that you may use them.

---

## Changelog

Every update must add an entry here, and the **version number must match `CODE_VERSION` in the `VERSION` file in the repository root** (the automated build checks this and refuses to build on a mismatch). The latest image carries both the `latest` tag and the matching `vX.Y` tag.

The **About Xexam** page of the app shows these entries (they are built into the frontend), so keep them in simple Markdown: paragraphs, `- ` bullets, nested `  - ` bullets, `**bold**` and `` `code` ``. Links are shown as plain text there.

### v0.7 — 2026-11-11

**Question banks per user, bank managers, a practice mode, a six-column Excel layout with answers in three languages, and a System log for administrators. The database moves to schema v4 automatically, after a backup copy is written, and now runs in WAL mode.**

- **Question banks per user**: in **User management** an administrator chooses which question banks each user sees: **All question banks, including ones imported later**, or only the banks ticked. A new user sees no bank until one is chosen; users from before v0.7 keep seeing every bank, and administrators always see every bank. Taking a bank away keeps the user’s study records as history.
- **Bank managers**: a user with the new **Bank manager** permission (set in **User management**) can import new question banks and update or delete the banks they imported, but not the banks of administrators or other users. Users without it do not see **Question banks** in the menu, and opening the page directly shows **Not allowed**. Administrators see who uploaded each bank; when a user who uploaded banks is deleted, the banks move to the administrator who deletes them.
- **Practice mode**: **Start practice** next to **Start exam** on Home, with the same banks, sheets, count and order. Each question is checked at once (a single-choice question when you choose an option, the others with **Check answer**) and shows right or wrong, the correct answer, the explanation and the memory aid. A practice is saved as a study record of its own kind and counts in the wrong-answer notebook like an exam. **Study records** tags every record as **Exam** or **Practice** and can show only one kind, Home’s **Latest result** shows the latest record of either kind, and **Practise these** in the wrong-answer review now starts a practice.
- **Six-column Excel layout**: A English, B Chinese and C Japanese question; D English, E Chinese and F Japanese answer with explanation and memory aid. English is authoritative: the question type, the options, the hotspot items, the question identity and the answer key come from columns A and D. When the Chinese or Japanese answer differs from the English one, the row is imported with the English answer and the preview lists a warning. The 3- and 4-column layouts work as before. A six-column file gives its questions new identities, so import it as a **New bank** rather than as an update of a 3- or 4-column bank.
- **Answers and explanations in three languages**: every question card of the result page, an exam record and the wrong-answer review has its own **English** / **中文** / **日本語** switch for the question, the options, the explanation and the memory aid; the separate Chinese translation toggle is gone. Every question-language switch now says **English** / **中文** / **日本語** in every interface language.
- **Solid green and red answers**: in the answers and explanations and in practice, the correct options are solid green and a wrongly chosen option is solid red, with white text, in the light and the dark theme alike. Graded hotspot statements sit on a neutral background.
- **System log** (menu, administrators only): the last 2000 lines the backend logged since it started, updated live, with **Pause**, **Jump to latest**, a level filter and a search. Rejected requests and failed sign-ins that come in bursts are summed up once a minute, so they cannot push the other lines out.
- **SQLite WAL mode**: the database now runs in WAL mode (still with `synchronous=FULL`, so a committed change is not lost) and is checkpointed when the backend stops normally. A backup copied while the backend runs must include `xexam.db-wal` and `xexam.db-shm`, or stop the backend first. `XEXAM_SQLITE_JOURNAL=delete` in `docker/.env` keeps the old mode for unusual storage such as a network filesystem.
- **Time zone**: timestamps follow the host’s time zone by default (the compose file mounts the host’s `/etc/localtime`); a `TZ` set in `docker/.env` overrides it.
- **Sign-in and users**: the lock after wrong passwords is kept per account and cannot be worn away by guessing many other usernames; changing a password signs out your other sessions; **Unlock** is also offered while a user still has tries left, and the lock note is shown under the user’s row; User management counts each user’s study records (exams and practice).
- **Version file**: `VERSION` now holds `CODE_VERSION` (the version of the code, bumped together with this changelog) and `RELEASE_VERSION` (the last published release). The release job writes `RELEASE_VERSION` and the release badge at the top of the READMEs only after the images and the GitHub Release are published.
- **Database schema v4**: new columns `users.all_banks`, `users.can_manage_banks`, `banks.owner_id` and `attempts.kind` and the table `user_banks`; before the migration a copy is written to `<data dir>/backups/xexam-schema3-<date-time>.db`.
- **New API**: `GET /api/logs` (administrators only); `banks` and `canManageBanks` in `/api/users`, `/api/users/create` and `/api/users/update`; `kind` (`exam` or `practice`) in `/api/attempt`, `/api/records` and `/api/record`. Only administrators see who owns a bank.
- Tests: backend 232 → 302, frontend 59 → 119.

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
