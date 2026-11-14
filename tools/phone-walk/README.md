# Phone walk

Drives the real Xexam UI on a phone-like Chrome (touch taps, rotation, safe-area insets, a simulated keyboard) against a
throw-away stack that holds only made-up data and a made-up AI provider. It is a release gate run by the coordinator and by
the UX reviewers, not a CI job. Run it from the repository root:

```sh
python tools/phone-walk/walk.py                      # default run: report + screenshots into a new temporary folder
python tools/phone-walk/walk.py --full               # every check on the whole matrix (final gate)
python tools/phone-walk/walk.py --pages home,quiz,result,ai,menu
python tools/phone-walk/walk.py --only W-QZ-*,W-CH-1
python tools/phone-walk/walk.py --langs zh --widths 390x844,360x740 --themes light
python tools/phone-walk/walk.py --keep               # leave the stack running: prints the URL and the made-up accounts
python tools/phone-walk/walk.py --keep --lan         # the same, reachable from a phone on the home network
python tools/phone-walk/walk.py --out DIR --chrome PATH
```

## What it needs

The same as `tools/screenshots` (see its README): Python 3.8+ with `openpyxl`, Node.js with `frontend/node_modules`,
Docker (or `--backend cargo`), Chrome / Chromium / Edge (`--chrome PATH`, or `XEXAM_CHROME` / `CHROME_PATH`).

## The stack

`walk.py` reuses `tools/screenshots`: `retake.DockerBackend` / `CargoBackend`, `seed.prepare`, `demo_banks`,
`browser.Chrome` / `CDP`. It builds the backend and ai-service images under unique tags (`xexam-walk-backend:<hex>`,
`xexam-walk-ai-service:<hex>`), starts them on a new network with new volumes, with `TZ=UTC`, ports published on
127.0.0.1, next to `tests/integration/ai_stub.py` (the made-up AI provider). The frontend is built with Vite and served by
`vite preview`, which does what Nginx's `auth_request` does for `/api/ai/`. The ports are free ports that are never one of
the owner's preview ports (5285, 5295, 8885, 8886, 8895, 8896, 9085, 8899). It removes only what it created (by name, no
prune), also on Ctrl+C; `stack.json` in the output folder lists every Docker name it created. It never connects to any other
instance.

Made-up users (one password for all of them, printed by `--keep`; a random password per `--lan` run):

| User | What it is |
|---|---|
| admin | administrator, AI+ always |
| lee | administrator with 60 study records |
| sam | user, may manage banks, AI+ |
| riley | user, no AI+ |
| alex | user, AI+ paused (greyed AI controls) |
| kim | user, disabled |
| jordan | user, locked by failed sign-ins |

Banks: General knowledge, Kitchen basics (from `tools/screenshots`) and "Long sample bank" (`seed_extra.py`: 51 invented
questions, one very long with an unbroken `https://www.example.com/...` address, a 5-item hotspot, "choose 2" questions).
The stub answers with self-made text.

## `--keep` and `--lan`

`--keep` starts and seeds the stack, prints its URL and the made-up accounts, runs no checks and waits. Stop it with Ctrl+C
or by creating a file named `STOP` in the output folder; both remove everything the stack created.

`--lan` (only with `--keep`) is for trying a build on a real phone over the home network:

- it prints a warning first: the stack is then reachable from every device on this network; only made-up data and the AI
  stub are in it; stop with Ctrl+C;
- only the web port (Vite preview) is published, on this computer's home-network address (10.x, 172.16-31.x or 192.168.x)
  instead of 127.0.0.1; the backend and ai-service ports stay on 127.0.0.1;
- the backend's Host / Origin guards (`XEXAM_ALLOWED_HOSTS`, `XEXAM_ALLOWED_ORIGINS`) list that one address and port, never
  `XEXAM_ALLOW_ANY_HOST`; without `--lan` they keep the localhost-only default;
- it prints the phone URL `http://<address>:<port>/`; Windows may show a firewall prompt the first time (allow it for the
  private network only);
- nothing real is ever loaded into it.

## The checks (`checks.py`, `checks.js`)

`checks.py` runs the checks of the spec (section 5.4) and writes `report.json` and one screenshot per entry into the output
folder (`<id>-<page>-<user>-<lang>-<theme>-<w>x<h>[-<state>].png`, metadata stripped). The in-page measurement code is
`checks.js` (`window.__walk.*`, loaded after every page load). Checks find elements only through the test hooks of spec 2.12 f.
The exit code is 1 when a P1 check failed; P2 parts (the bank-access row of W-US-1, the usage cards of W-AS-1, W-US-2, the short-page entry of W-SH-10) are reported
but never fail the run; a check that cannot run (no safe-area emulation in this Chrome, a control that is not on the page)
is reported as `skipped` with the reason.

| Group | Checks |
|---|---|
| Shared | W-SH-1 sideways scroll, W-SH-2 toast vs pinned bars and the screen, W-SH-3 hit areas, W-SH-4 insets, W-SH-5 nothing covers a control, W-SH-6 contrast (also the footer lines and the Saved text), W-SH-7 dialog action rows, W-SH-8 nothing in the AI+ panel or the AI summary window navigates (the Saved text and its tip), W-SH-9 no focus frame on titles focused by script, W-SH-10 the footer's GitHub line on every page (a short page's blank space under it is reported as P2), W-MN-1 menu |
| Practice / exam | W-QZ-1 first stem line, W-QZ-2 one-row bar, one-line button labels and fixed slots (Check answer and Next too), W-QZ-3 Next not covered, W-QZ-4 panel head position, W-QZ-5 question changes keep the panel and send no ask, W-QZ-6 greyed bar, W-QZ-7 landscape space, W-QZ-8 answer sheet, W-QZ-9 Check answer and Next are two buttons on every practice question (no Check answer in an exam) |
| Other pages | W-RS-1 / 2 result, W-RC-1 records, W-HM-1 Home, W-CH-1 / 2 / 3 AI Chat (W-CH-3 = the keyboard, CH-13), W-US-1 users, W-LG-1 log swipe, W-ST-1 Settings, W-AS-1 AI settings, W-AS-2 answer and memory length in characters, W-AS-3 the four category tabs, W-CH-4 memory length (n / 200), W-CH-5 touch swipes on the history list / conversation head move no frame (A-OWNER-76), W-US-2 #/users without permission (P2) |
| Caps | W-CAP-1 second ask (with `--slow`: the callout goes away after `retryAt`), W-CAP-2 AI Chat capped, W-CAP-3 stale block |

Run dimensions (spec 5.4): the default run is zh / light at the sizes a check names (default 360x740, 390x844, 844x390); the
text-fit checks (W-QZ-2, W-HM-1, W-CH-2, W-US-1, W-AS-1) run zh, en and ja; contrast runs light and dark; `--full` runs every
check on the whole matrix (360x740, 390x844, 412x915, 740x360, 844x390, 768x1024, 1280x800) x 3 languages x 2 themes;
`--langs`, `--widths`, `--themes`, `--pages` and `--only` narrow or override. The walk raises the administrator's AI request
caps while it runs (so the AI checks do not trip them by accident) and puts the defaults back at the end; W-CAP-* lower them
on purpose. W-CH-3 (CH-13) uses Chrome's emulation only (the simulated keyboard and the iOS layout maths); it never replaces
the check on a real iPhone.

How some checks measure (so a finding means what a thumb would meet):

- W-SH-1 counts a block as too wide only when one of its element boxes sticks out (the invisible `::after` tap margins of
  buttons at an edge do not count), and also flags a vertical scroller (a message list, a panel body, a menu) that can be
  scrolled sideways; tables inside a scroller scroll sideways on purpose.
- W-SH-3 / W-SH-5 first bring a control into the free area (below the top row, above the pinned bar or composer, and inside
  its own scroller such as the menu), as a user would by scrolling; `tap_sel` does the same before it taps.
- W-SH-2 shows the toast the way `toast()` does (the lift is measured again for its text) at the top, the middle and the
  end of the page (practice, Home for lee and alex, AI Chat): it must not touch the bar and must lie fully between the
  top row (`--top-h`) and the bottom safe area.
- W-QZ-1 measures from the top of the safe area (the emulated 47 px notch inset is screen the page cannot use).
- W-QZ-2 compares the slots' left edge, width and top inside the bar; the bar itself may sit higher up on a short page.
- W-QZ-4 also passes when the page cannot scroll the panel head that far up (a short page): the whole panel is then between
  the top row and the pinned bar.
- The checks that open the folded answer sheet (W-QZ-5, W-QZ-8) fold it again, because the app remembers it per browser.

## Contract with `checks.py` (T1b)

`walk.py` calls `checks.run(walk)` when `checks.py` is next to it; the return value is the exit code (1 when a P1 check
failed). `walk` has: `args` (the parsed options: `full`, `pages`, `only`, `langs`, `widths`, `themes`, `slow`; `None` means
"use the run dimensions of the spec"), `stack` (`base`, `cookie(user)`, `session(user)` for API calls with `seed.Session`),
`out` (the output folder for `report.json` and the screenshots), `chrome`, `started`, `device(user, lang, theme, width,
height)` (a `device.Device`), `wants_page(page)` and `wants_check(id)`. Checks find elements only through the test hooks of
spec 2.12 f.

`device.Device` (`device.py`): `set_size`, `set_theme`, `set_reduced_motion`, `keyboard(True / False)`, `nav`, `js`,
`js_call`, `rect`, `tap`, `tap_sel`, `swipe`, `type`, `key`, `scroll`, `shot`, `net_clear` / `net_requests`, `close`.
`safe_area_supported` is False when this Chrome has no `Emulation.setSafeAreaInsetsOverride`; the checks that need the
insets are then reported as skipped.

## v1.1 checks

`checks.py` group `g_v11` (run them alone with `--only W-IM-1,W-IM-2,W-GB-1,W-BT-1,W-FP-1,W-UC-1,W-AS-1v,W-FT-1,W-SI-1,W-AB-1`):

| Check | What |
|---|---|
| W-IM-1 | Bank management (sam) at 360 / 390 / 412 / 768 / 1280: the AI import is its own card (not inside the Excel card) with a heading, a note naming the size and question limit, a drop zone; its button is the Excel button's twin (same size and left edge), 44 px high on phones, no sideways scroll; a user without AI+ (riley) sees no card |
| W-IM-2 | Bank management (sam) at 360 / 390 / 412: a made-up file with 130 numbered lines fails locally (no AI call); the failed import card has a close button in its top-right corner, 44 px on a phone, and tapping it removes the card |
| W-GB-1 | Home (sam): the AI generation button sits in the start panel header inside the screen, no sideways scroll |
| W-BT-1 | every AI+ badge (header, menu, Settings, User management, AI settings rows, Home) has a non-empty tip; the account button has no native `title` |
| W-FP-1 | AI settings > AI+ users: six checkboxes in two groups per row, each group on one row in zh / ja; administrators' boxes disabled |
| W-UC-1 | AI settings > AI usage at 360 / 390 / 412: three charts with an `aria-label`, every bar labelled, the calls / tokens switch on both bar charts (the title follows, tokens are stacked), a tip on a tapped bar and 10 px beside the line (none 90 px away), the line 3 px, labels never overlap, they fit the card |
| W-AB-1 | one AI bank is generated (admin), practised and deleted again: the AI+题集 badge with its tip and caution line is on the question card and on the result (skipped when no AI bank can be made on the stack) |
| W-AS-1v | practice and exam at 360 / 390 / 768: no finish button in the answer sheet (folded or open), the top row has "已答 x/n" and a 完成考试 (green) / 完成练习 (orange) button, white text, left of the clock (44 px on touch, no overlap, inside the screen); at 1280 the rail keeps its button and the top row has neither |
| W-FT-1 | the footer has one blank text line between its divider and its text, and one between the text and the GitHub line |
| W-SI-1 | sign-in page: the language switch is about 16 px under the card; the GitHub line is at the screen bottom |

`panel_case` (W-QZ-3 / 4 / 5) now taps an option and then 核对答案 (v1.1: no question type is checked on pick). End-to-end AI
import / generation runs against the made-up provider in `tests/integration/stack_check.py` (section A6); the walk does not
upload files.

## v1.2 checks

Each group lives in its own file (`checks_v12_*.py`) and is registered at the end of `checks.py` (`GROUPS_V12_*`). The old
one-line token check W-TK-1 is retired (its group `g_tk` stays in `checks.py` but is no longer in `GROUPS`); W-TK-2 replaces it.

| Check | What |
|---|---|
| W-QZ-81 | `checks_v12_quiz.py`: the finish pill in the top row of an exam / practice always shows its text (zh 完成练习 / 完成考试; en and ja use a short label when the row is tight), one line, 44 px tap area, zh / en / ja at nine sizes |
| W-CH-82 | `checks_v12_ai.py`: AI Chat history and memory screens are fixed frames: every drag (up, down, sideways, diagonal) on the empty area under a short list, the rows, the GitHub line, the list head and the top bar is cancelled; a long list still scrolls |
| W-CH-83 | the AI Chat title row and the top bar never overlap or spill at text zoom 1 / 1.25 / 1.5 |
| W-CH-88 | with the keyboard open the messages do not move (open, typing, close; list at its end, middle and top), a send still ends at the bottom |
| W-TK-2 | `checks_v12_tokens.py`: the live token text of a running generation / import card is a grid of two aligned lines (calls + Input/output, Cache write/read), no wrap, no sideways scroll; the import card says Cancel import |
| W-UC-3 | `checks_v12_charts.py`: AI usage charts have no slanted label; By feature / By model are horizontal bars on a phone, By time has short horizontal dates, nothing outside the card |
