# Screenshot tool

Retakes every screenshot of the READMEs and docs (`docs/<lang>/screenshots/`, 13 desktop and 4 iPhone images per
language) from a throw-away Xexam instance that holds only made-up demo data. Run it from the repository root:

```sh
python tools/screenshots/retake.py                      # all three languages, into docs/<lang>/screenshots/
python tools/screenshots/retake.py --lang ja            # one language: en, ja or zh-CN
python tools/screenshots/retake.py --out /tmp/shots     # write to /tmp/shots/<lang>/ instead (to compare first)
python tools/screenshots/retake.py --only desktop-13-logs.png,iphone-3-menu.png
python tools/screenshots/retake.py --chrome /usr/bin/chromium
```

## What it needs

- Python 3.8 or newer with `openpyxl` (`pip install openpyxl`); nothing else outside the standard library.
- Docker (the backend is built from `backend/Dockerfile`). Without Docker, `--backend cargo` builds `backend/` with
  cargo instead; that run skips `desktop-13-logs.png`, because the System log would show the host path of the data
  folder (the container shows `/data`).
- Node.js; `frontend/node_modules` is installed with `npm ci` when it is missing.
- Chrome or Chromium (Edge works too): `--chrome PATH`, or the `XEXAM_CHROME` or `CHROME_PATH` environment variable,
  otherwise the usual install locations of each operating system.

A full run of the three languages takes a few minutes, plus the image build when Docker has nothing cached.

## What it does

1. `demo_banks.py` writes the demo banks (General knowledge, Kitchen basics, Kitchen basics (v2)) to a temporary folder.
   They are six-column workbooks with invented questions; no `.xlsx` file is ever written into the repository.
2. The backend image is built under a unique tag and started with a new volume on `/data`, `TZ=UTC` and `XEXAM_VERSION`
   set to `CODE_VERSION` from `VERSION`, published on a free port of 127.0.0.1.
3. The frontend is built with Vite into the temporary folder and served by `vite preview` on a free port, with `/api`
   proxied to that backend.
4. `seed.py` signs in as the default administrator (who chooses a demo password), imports the banks, adds the users
   alex, sam, kim and lee and some study records. The backend is then restarted, so the System log shows only a short
   stretch of everyday activity: sign-ins, a user created and a user updated by the administrator, and a made-up
   username that is guessed until it locks.
5. `scenes.py` opens one headless Chrome per language (temporary profile, UTC clock, light theme unless the scene is the
   dark one) and drives the real UI through DOM clicks. Tall pages are cut where no text line, control or card is
   split; the System log image is sized so its whole card (lead, filters, lines, note) is in view. Two pages are
   shortened in the page itself so their card ends inside the image: Question banks shows the guide unfolded with
   only its first two sections (basics and the six columns), and About shows only the first bullets of the newest
   version. PNG text, time and EXIF chunks are removed (`browser.strip_png`).
6. The container, the volume, the image tag, the temporary folders and every process the tool started are removed at
   the end, also after an error or Ctrl+C. Nothing is pushed anywhere.

## Files

| File | Purpose |
|---|---|
| `retake.py` | Entry point: options, backend, frontend, seeding, the languages, clean-up |
| `demo_banks.py` | The made-up demo question banks |
| `seed.py` | Users, banks, records and the System log activity, through the API |
| `scenes.py` | The 17 scenes and how each image is sized |
| `browser.py` | Finding and starting Chrome, a small DevTools (CDP) client, PNG metadata removal |

When the UI changes, adjust the scene in `scenes.py` (the selectors are those of `frontend/src/pages/`), run the
tool with `--out` and look at the images before writing them into `docs/`.
