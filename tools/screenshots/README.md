# Screenshot tool

Retakes every screenshot of the READMEs and docs (`docs/<lang>/screenshots/`, 25 desktop and 8 iPhone images per
language, 33 in all) from a throw-away Xexam instance that holds only made-up demo data. Every value the images show is
made up, also on the AI pages: the stub's address `http://ai-stub:9000/v1`, the fake key (shown as `…9c21`), the
`demo-*` models, the demo users and banks. The tool never connects to any other instance, so real addresses, keys or
model choices cannot appear. Run it from the repository root:

```sh
python tools/screenshots/retake.py                      # all three languages, into docs/<lang>/screenshots/
python tools/screenshots/retake.py --lang ja            # one language: en, ja or zh-CN
python tools/screenshots/retake.py --out /tmp/shots     # write to /tmp/shots/<lang>/ instead (to compare first)
python tools/screenshots/retake.py --only desktop-13-logs.png,iphone-3-menu.png
python tools/screenshots/retake.py --chrome /usr/bin/chromium
```

## What it needs

- Python 3.8 or newer with `openpyxl` (`pip install openpyxl`); nothing else outside the standard library.
- Docker (the backend is built from `backend/Dockerfile`, ai-service from `ai-service/Dockerfile`; the made-up AI
  provider runs in the `python:3.12-slim` image). Without Docker, `--backend cargo` builds `backend/` and `ai-service/`
  with cargo instead and runs the made-up provider as a local Python process; that run skips `desktop-13-logs.png`,
  because the System log would show the host path of the data folder (the container shows `/data`).
- Node.js; `frontend/node_modules` is installed with `npm ci` when it is missing.
- Chrome or Chromium (Edge works too): `--chrome PATH`, or the `XEXAM_CHROME` or `CHROME_PATH` environment variable,
  otherwise the usual install locations of each operating system.

A full run of the three languages takes a few minutes, plus the image builds when Docker has nothing cached.

## What it does

1. `demo_banks.py` writes the demo banks (General knowledge, Kitchen basics, Kitchen basics (v2)) to a temporary folder.
   They are six-column workbooks with invented questions; no `.xlsx` file is ever written into the repository.
2. The backend and ai-service images are built under unique tags (`xexam-shots-backend:<hex>`,
   `xexam-shots-ai-service:<hex>`) and started on a new Docker network: the backend with a new volume on `/data` and a
   new link volume on `/link` (network alias `backend`), ai-service with its own new volume on `/data` and the link
   volume read-only. Both get `TZ=UTC` and `XEXAM_VERSION` set to `CODE_VERSION` from `VERSION` and are published on
   free ports of 127.0.0.1. `tests/integration/ai_stub.py`, a made-up AI provider that answers with self-made text in
   the reply language the page sends (the Settings language), runs on the same network under the single-label name
   `ai-stub` (so AI settings shows the address `http://ai-stub:9000/v1` without an http:// warning and without a real
   domain).
3. The frontend is built with Vite into the temporary folder and served by `vite preview` on a free port, with `/api`
   proxied to the backend and `/api/ai` to the AI service; the auth step of `vite.config.ts` checks every AI request with
   the backend first, as Nginx does. The tool waits until ai-service's `/healthz` answers and `/api/ai/status`
   answers 401 without a session (the whole chain works).
4. `seed.py` signs in as the default administrator (who chooses a demo password), imports the banks, adds the users
   alex, sam, kim and lee and some study records, sets up AI with the made-up provider and a made-up key (provider,
   key, model, test, the AI+ feature on) and saves two memories of the administrator. Because a user's AI+ feature can
   be switched only while AI+ is available, sam and alex get it only after that, and alex's AI+ feature is then
   paused: Users shows a bright (sam) and a dark (alex) badge, AI settings an Active and a Paused row. No AI chat
   is seeded. The backend is then restarted, so the System log shows only a short stretch of everyday activity:
   sign-ins, a user created and a user updated by the administrator, and a made-up username that is guessed until it
   locks. Lines of ai-service (with the prefix `AI: `) may appear there too; they are real lines. After a restart the
   backend knows nothing about ai-service until its next report (every 25 s), so the tool then waits until
   `GET /api/users` says `aiAvailable: true` (at most 40 s) before the scenes.
5. `scenes.py` opens one headless Chrome per language (temporary profile, UTC clock, light theme unless the scene is the
   dark one) and drives the real UI through DOM clicks. Tall pages are cut where no text line, control or card is
   split; the System log image is sized so its whole card (lead, filters, lines, note) is in view. Two pages are
   shortened in the page itself so their card ends inside the image: Question banks shows the guide unfolded with
   only its first two sections (basics and the six columns), and About shows only the first bullets of the newest
   version. The AI Chat scene (as the administrator) turns the administrator's free mode on through `POST /api/ai/free`, so the green
   "Free mode" tag shows right of the title (AI Chat has no mode switch; the choice is in Settings, in the
   administrator-only AI chat mode section that the Settings image shows, with Normal selected), starts a chat through the
   API in the language of the run, opens it, sends a second question through its composer and deletes the chat again
   and turns free mode off afterwards; its conversation box follows the window height, so the window is made just tall enough for the whole
   conversation and the image ends below it.
   The v1.0 UI is phone-first, and the scenes follow it:
   - **Practice and the AI+ panel** (`desktop-11-practice`, `desktop-17-ai-hint`, `iphone-5-ai-hint`): the bar shows both
     Check answer and Next as two separate buttons; the AI hint opens the collapsible AI+ panel under the card (expanded on
     the stub's hint) with the muted "Saved in AI Chat" text in its footer (a tip names the chat; there is no Open in AI
     Chat control and nothing in the panel navigates away). On the iPhone image the panel is scrolled in under the top row.
   - **Result and summary** (`desktop-18-ai-result`, `iphone-6-result`, `desktop-19-ai-summary`): card 1 of the exam result
     with its panel expanded on AI analysis and card 2's collapsed AI+ bar below it; the summary window of a study
     record with the same Saved text.
   - **AI Chat on a phone** (`iphone-7-ai-chat`): a short chat started through the API, opened full height.
   - **AI settings** is one page with category tabs under the status chips; the scene presses Refresh list and Test on
     the AI+ function tab (`#/ai-settings?tab=feature`), then shoots `?tab=users` (`desktop-15-ai-settings`), `?tab=usage`
     (`desktop-16-ai-usage`, with the Today column) and `?tab=safety` (`desktop-20-ai-safety`: chat retention and the
     limits: the per-user request caps, the "Answer and memory length" group, the server protection). Sam asks one
     made-up question while seeding, so the users list shows a calls line and the usage table has more than one user.
   - **The footer** (the disclaimer, then the centered "GitHub | Xexam <version>" line) is part of the whole-page Home
     image and is checked on the phone Home and on the AI Chat list pane.
   - **v1.1** (`desktop-22-timed-exam` … `desktop-25-ai-bank-home`, `desktop-21-ai-banks`, `iphone-8-banks`): an exam
     started with the exam time limit ticked on Home (45 minutes, the countdown in the top row), its submit confirm with
     questions not completed and its result filtered on them; then one AI generation from the checked sheets of General
     knowledge, made by the stub (the minimum source size is lowered to 5 only for the start, the demo bank being small),
     with a name per language: Home with the AI+ Set row and the done card, the AI banks tab of AI settings with the job,
     Bank management on a phone. The AI bank is deleted right after these images, so no other scene asks its made-up
     questions.
   Their asks are saved in AI Chat; the run deletes the administrator's AI chats before the AI Chat scenes and at the
   end. Each language adds a few calls to the AI usage numbers, so they differ slightly between the languages. PNG text,
   time and EXIF chunks are removed (`browser.strip_png`).
   The same pass checks the layout and the owner's feedback (a failed check stops the run with a message), in every
   language: at 360, 390 and 412 px the practice bar is one row with Check answer and Next as separate buttons in fixed
   places (nothing moves before / after the check or on the next question), 44 px touch targets and no sideways scroll;
   for alex (AI+ feature paused) at 390 and 360 px the same, the AI controls greyed and the greyed AI+ bar in view; the
   exam bar is one row with no Check answer and no AI buttons; the AI+ panel and the summary window show the Saved
   text, contain no link or Open in AI Chat control and draw no focus frame on a title focused by script; the
   footer line is centered with its two links (`https://github.com/Yumin-Space/xexam` and `.../releases`, new tab, `noopener
   noreferrer`) below the disclaimer, with no old "Xexam v..." line; AI settings has five category tabs with the right
   one selected and, in AI safety, the three limit groups; Study records cards are at most 140 px tall at 360 px; at
   800 px the Study records list (English and Japanese) is in the card layout with no horizontal scroll and Delete on
   one line.
6. The containers, the volumes, the network, the image tags, the temporary folders and every process the tool started
   are removed at the end, also after an error or Ctrl+C. Nothing is pushed anywhere.

Several runs can go at the same time, for example one per language (`--lang en`, `--lang ja` and `--lang zh-CN` in
three terminals): every image tag, container, volume and network name carries a random suffix of its run, each run has
its own temporary folder and Chrome profile, the ports are free ports of 127.0.0.1 (a port that a parallel run took
first is retried with another one), and `npm ci` (only when `frontend/node_modules` is missing) runs once at a time.

## Files

| File | Purpose |
|---|---|
| `retake.py` | Entry point: options, backend and ai-service, frontend, seeding, the languages, clean-up |
| `demo_banks.py` | The made-up demo question banks |
| `seed.py` | Users, banks, records, the AI setup (with the AI+ feature of sam and alex, alex paused) and the System log activity, through the API |
| `scenes.py` | The 33 scenes (the AI+ panel, result, summary, AI Chat, the AI settings tabs and safety limits included), the layout / feedback checks, and how each image is sized |
| `browser.py` | Finding and starting Chrome, a small DevTools (CDP) client, PNG metadata removal |
| `../../tests/integration/ai_stub.py` | The made-up AI provider (shared with the stack check of CI) |

When the UI changes, adjust the scene in `scenes.py` (the selectors are those of `frontend/src/pages/`), run the
tool with `--out` and look at the images before writing them into `docs/`. Check every AI image (and every image
with an AI+ badge) by eye before it is committed: only the made-up values above may appear.

The app icons in `frontend/public/` are made by a separate script, `tools/icons/make_icons.py`, which uses
`browser.py` from this folder.
