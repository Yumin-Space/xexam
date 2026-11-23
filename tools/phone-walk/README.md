# Phone walk

Drives the real Xexam UI on a phone-like Chrome (touch taps, rotation, safe-area insets, a simulated keyboard) against a
throw-away stack that holds only made-up data and a made-up AI provider. It is a release gate run by the coordinator and by
the UX reviewers, not a CI job. Run it from the repository root:

```sh
python tools/phone-walk/walk.py                      # default run: report + screenshots into a new temporary folder
python tools/phone-walk/walk.py --full               # every check on the whole matrix (final gate)
python tools/phone-walk/walk.py --pages home,quiz,result,ai,menu
python tools/phone-walk/walk.py --only W-QZ-*,W-CH-1
python tools/phone-walk/walk.py --langs zh --widths 393x852,360x740 --themes light
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
| Practice / exam | W-QZ-1 first stem line, W-QZ-2 one-row bar, one-line button labels and fixed slots (Check answer and Next too), W-QZ-3 Next not covered, W-QZ-4 panel head position, W-QZ-5 question changes keep the panel and send no ask, W-QZ-6 greyed bar, W-QZ-7 landscape space, W-QZ-8 answer sheet (phones: a left drawer opened by the "1 / n" button), W-QZ-9 Check answer and Next are two buttons on every practice question (no Check answer in an exam) |
| Other pages | W-RS-1 / 2 result, W-RC-1 records, W-HM-1 Home, W-CH-1 / 2 / 3 AI Chat (W-CH-3 = the keyboard, CH-13), W-US-1 users, W-LG-1 log swipe, W-ST-1 Settings, W-AS-1 AI settings, W-AS-2 answer and memory length in characters, W-AS-3 the four category tabs, W-CH-4 memory length (n / 200), W-CH-5 touch swipes on the history list / conversation head move no frame (A-OWNER-76), W-US-2 #/users without permission (P2) |
| Caps | W-CAP-1 second ask (with `--slow`: the callout goes away after `retryAt`), W-CAP-2 AI Chat capped, W-CAP-3 stale block |

Run dimensions (spec 5.4): the default run is zh / light at the sizes a check names (default 393x852, 440x956, 852x393); the
text-fit checks (W-QZ-2, W-HM-1, W-CH-2, W-US-1, W-AS-1) run zh, en and ja; contrast runs light and dark; `--full` runs every
check on the whole matrix (360x740, 390x844, 412x915, 740x360, 844x390, 768x1024, 1280x800) x 3 languages x 2 themes;
`--widths` takes WxH or the names iphone16 (393x852), iphone16pro (402x874), iphone16plus (430x932), iphone16promax (440x956), each with an optional `-land` suffix that swaps width and height; `--langs`, `--widths`, `--themes`, `--pages` and `--only` narrow or override. The walk raises the administrator's AI request
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
- Since v2.7 the phone answer sheet is a left drawer (`#qz-rail`, opened by `#qz-pos`, closed by `.rail-close`, the backdrop, Escape or a tile tap); W-QZ-5, W-QZ-8 and W-AS-1v open and close it that way.

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
| W-BT-1 | every AI+ badge (header, menu, Profile, User management, AI settings rows, Home) has a non-empty tip; the account button has no native `title` |
| W-FP-1 | AI settings > AI+ users: five checkboxes in two groups per row (AI hint and AI analysis are one box), each group on one row in zh / ja; administrators' boxes disabled |
| W-UC-1 | AI settings > AI usage at 360 / 390 / 412: three charts with an `aria-label`, every bar labelled, the calls / tokens switch on both bar charts (the title follows, tokens are stacked), a tip on a tapped bar and 10 px beside the line (none 90 px away), the line 3 px, labels never overlap, they fit the card |
| W-AB-1 | one AI bank is generated (admin), practised and deleted again: the AI+题集 badge with its tip and caution line is on the question card and on the result (skipped when no AI bank can be made on the stack) |
| W-AS-1v | practice and exam at 360 / 390 / 768: no finish button in the answer sheet (closed or open drawer), the top row has "已答 x/n" and a 完成考试 (green) / 完成练习 (orange) button, white text, left of the clock (44 px on touch, no overlap, inside the screen); at 1280 the rail keeps its button and the top row has neither |
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

## v1.3 checks

`checks_v13.py` (group `g_v13`, registered at the end of `checks.py`; run alone with `--only 'W-V13-*'`), zh / en / ja at
393x852, 360x740, 320x640 and 1280x800. Every case runs the shared measures (no sideways overflow, touch hit areas, nothing
covers a control) next to what the screen must show:

| Check | What |
|---|---|
| W-V13-CV | AI Chat: a message with 2 versions shows `‹ 2/2 ›`, the left arrow goes to 1/2 and back, a regenerate button is there |
| W-V13-ED | AI Chat: Edit on the last question opens the edit banner inside the screen, Cancel closes it |
| W-V13-MM | AI Chat, administrator in free mode: the model menu is one select inside the screen with groups, laid over the round brain button; the composer is one box (input above one bar: brain button left, Send right; empty box at most 120 px high at 393 px wide) |
| W-V13-AC | AI settings, Connections and models: status cards, connection cards, slot rows, switches card, the Add form |
| W-V13-AT | AI settings, the other four tabs open and nothing sticks out (the sort buttons of the long tables are not judged) |
| W-V13-US | User management: the sign-in info eye closed and open (the server value is shown), the last-active line on every row |
| W-V13-LG | Logs: the Activity tab (rows, an open row, the settings popover) and the Service tab |
| W-V13-SI | Sign-in page: "keep me signed in" checkbox and its hint under it |
| W-V13-RS | Result page: when saving the attempt fails, the retry-save banner and its button |

The stack's AI is set up the v1.3 way by `tools/screenshots/seed.py`: a connection with the made-up provider and key, a second
connection that reuses its key, the default model picked and AI+ switched on.

## v2.2 checks

`checks_v22.py` (group `g_v22`, registered at the end of `checks.py`; run alone with `--only 'W-V22-*'`), zh / en / ja at
393x852, 360x740, 320x640 and 1280x800. Every case saves screenshots (`<id>-<page>-<user>-<lang>-<theme>-<w>x<h>-<state>.png`)
into the output folder, passed or not; a check that cannot run (no AI job on the stack yet) is reported as `skipped`.

| Check | What |
|---|---|
| W-V22-AB | Phone top bar (admin, sam, alex, riley): the account button shows the person icon, the compact AI+ badge and the short admin tag with no overlap (inside the button, clear of the menu / theme buttons), the user name hidden, 44 px high; a tap shows the name tip (inside the screen, names the user, opens no menu), a tap elsewhere and Esc close it, the menu button opens the menu; at 1280 the name is shown and the button opens the menu |
| W-V22-QT | Quiz top row, practice and exam: the theme toggle is in the right group, 44 px on phones, nothing in the row overlaps or leaves the screen (position, count, finish, timer, toggle, exit), a tap switches dark / light (page colour and label change), a second tap restores it |
| W-V22-CS | AI Chat (administrator, free mode): the strength button is on the same row right of the brain button, no overlap with the brain button or Send, nothing overflows the composer bar / box; its menu opens with Default first, one item checked, items inside the menu and the screen, Esc closes it |
| W-V22-MS | AI settings, model rows: every strength dropdown has Default (empty value) first, its text says in brackets that no parameter is sent, fits the row and the screen |
| W-V22-GJ | AI settings > AI bank tab: the jobs list is a paged table (search, sort, pager when more than 10 jobs), cards on phones, no sideways overflow; the details toggle opens the details (several fields, inside the screen) and closes them again |
| W-V22-FT | Footer disclaimer on Home and Settings: centered (equal gaps) when it fits on one line; when it wraps it is not wider than the page and reads left to right; the shutdown button, when shown, is centered |

## v2.3 checks

`checks_v23.py` (group `g_v23`, registered at the end of `checks.py`; run alone with `--only 'W-V23-*'`), zh / en / ja. AI Chat is
laid out like a chat app: `#/ai` always opens a new chat, the history is a drawer behind the menu button (`#ai-drawer-btn`) on
phones (< 720 px) and an always-visible sidebar (`aside.ai-list`) on tablets / desktops. Screenshots are saved as in v2.2.

| Check | What |
|---|---|
| W-V23-EN | `#/ai` opens a new chat at 393 / 360 / 320 px, 768 and 1280: address `#/ai/new`, greeting and composer on screen, no history list on a phone (menu button shown), the sidebar on a tablet / desktop (no menu button) |
| W-V23-DR | Phone drawer: the menu button opens it, width about min(320, 82 %), inside the screen, nearly full height, dialog semantics (`role`, `aria-modal`, label), focus on the drawer itself (not the search box), a backdrop and a free strip beside it; a backdrop tap, Esc and the back button each close it (back stays on AI Chat), the focus returns to the menu button |
| W-V23-SE | The search box (`#ai-search`) filters the rows live and case-insensitively, a made-up word leaves no row, clearing restores every row (393 and 1280) |
| W-V23-MEM | The memory row (`#ai-mem-open`) opens the memories view (the phone drawer closes), the back-to-chat control is 44 px high and inside the screen and returns to the conversation |
| W-V23-HD | Phone chat header row (new chat and an open chat; the administrator in free mode too): menu button, title, free-mode tag, new-chat and more buttons (`#ai-chat-more`, only for an existing chat) without overlap or overflow at 320 px, 44 px targets, row at most 44 px high |
| W-V23-SB | Tablet 768 / desktop 1280: the sidebar is visible and inside the screen (240 / 272 px), no menu button, search box, new chat button, memory row, rows with group headings and a row menu button (`#ai-cmenu-<id>`) each |
| W-V23-DL | Delete from the phone drawer: the confirm dialog keeps Tab and Esc (the drawer does not take them); deleting the open chat or all chats ends on a clean new chat whose address stays `#/ai/new` after the drawer closes and after a reload, never an error; the red items have no shadow |
| W-V23-GONE | A chat id that no longer exists opens a real new-chat draft (`#/ai/new`) without an error, also after opening and closing the drawer |
| W-V23-RO | A paused (read-only) user: the latest chat opens, no new-chat button in the header, the paused line is shown, no greeting |
| W-V23-FW | Closing the drawer leaves no dead forward entry (Forward twice, then Back, leaves AI Chat) |
| W-V23-KB | The drawer's bottom follows the keyboard height (`--kb`) |
| W-V23-LS | Phones turned sideways (568x320, 667x375): the whole drawer scrolls as one, Delete all and the GitHub line are reachable |
| W-V23-LT | "Jump to latest" scrolls to the end and the focus goes to the message list, never to the input |
| W-V23-CTR | An empty new chat: greeting, composer and reply-language hint are one group centred in the visible area under the chat head (composer centre in the middle third, greeting above, hint below, no line above the composer, same background as the conversation), above an emulated keyboard (`--vvh` / `--kb`); an open chat with messages has the composer at the bottom |
| W-V23-CMP | Composer growth: empty = one compact pill row (input and send on one line, fully rounded); a long line = expanded layout (input at the full width above the controls row, radius 16-32 px, focus kept); many lines grow it up to 8 lines / 40 % of the visible height, then it scrolls inside; clearing returns to the compact row (393, 320, 1280) |
| W-V23-NT | The administrator notice is under the composer on one line at 375 / 393 / 430 px, and not in the drawer |

W-V22-AB also checks the top bar's brand: the word Xexam without the logo icon on phones, with it on wider screens.

Older checks that assumed the list-first phone flow or the Chats / Memory tabs now open the drawer first (`open_sidebar`) and use
the memory row (`open_memories`), both in `checks.py`: W-CH-2 (phone header row instead of the title row), W-CH-4, W-CH-5,
W-CH-82 and W-CH-83.

## v2.6 checks (`checks_v26.py`)

`checks_v26.py` (group `g_v26`, registered at the end of `checks.py`; run alone with `--only 'W-V26-*'`), zh / en / ja, light and
dark, 320 / 375 / 430 px plus 1280. Since v2.6 every made-up user signs in with an e-mail (`seed.LOGINS`, e.g. `sam@example.com`)
and has a display name (`seed.NAMES`); the checks keep the short handles (`sam`, `lee`, ...) and `seed.Session.sign_in` passes
the first-sign-in gate (new password + name, or name only). Hand-written mode is on by default (`html[data-hand]`). The final Home
has this order: greeting, resume card, the Exam & practice hero (a paper plane), six tiles (3 per row), the leaderboard as a
hand-drawn line chart (`.dash-rank`), the most-missed parabola board (`.dash-wrong`) and the AI assessment card (`.dash-ai`),
folded on every visit and last. `lee` has wrong answers on at least five questions, so the most-missed board has five points; the
card check makes one assessment of `lee` through the made-up provider on first use.

| Check | What |
|---|---|
| W-V26-RANK | Leaderboard line chart (lee): every point (`.dash-rank .dash-pt`) a button >= 44 x 44 px, inside the width, no overlap, an aria-label each, place 1 highest, every SVG text inside the board; before a tap the hint shows, a tap shows the line (`.dash-rank-pick`, no `@`) and presses one point |
| W-V26-POD | Most-missed parabola board (lee): five points >= 44 x 44 px, no overlap, nothing cut or outside the width, SVG texts inside the board; a tap on place 1 opens the question and its correct answer under the board (`aria-live` polite), a second tap closes it; on a desktop Escape clears a pinned pick |
| W-V26-CAP | AI assessment card (lee): last block of the page, folded on arrival (`aria-expanded` false, body hidden), toggle >= 44 px high and inside the card, title and AI+ badge apart; opened by a tap: whole text, no model name, action button >= 44 px high and inside the card (disabled with a reason line when today's is used); folded again after leaving Home and coming back; a user without AI+ and without a saved text (riley) has no card |
| W-V26-TL | Tiles: 5 or 6 tiles are rows of three (a wide screen may also show one row), 4 tiles are 2 x 2 up to 480 px and one row wider; order records, wrong answers, AI Chat, banks, settings, profile; each >= 88 px high, `data-n` matches the count (lee, sam, riley); hand mode is on by default and the Aa switch is visible |
| W-V26-PRF | Profile page (sam): account / username / password sections, every control >= 44 px on a phone, nothing outside the width, the login line is an e-mail, the password row opens three fields in place; Settings has no account section; the menu user row is a >= 44 px link to `#/profile` and opens it |
| W-V26-CTR | 1280 / 1440 px: Settings, Profile, Users, AI settings and About are centred (left and right space equal within 2 px) |
| W-V26-LOG | System log (admin): a pager shows on one of the two tabs, inside the width, controls >= 44 px on a phone, no overlap, no sideways scroll |
| W-V26-NAME | First sign-in (page `signin`): a new user sees the new-password page with the username field above the password fields; a user who only lacks a username sees the name-only page (one field, Save, Sign out); targets >= 44 px on a phone, nothing sticks out, a 21-character name and a name with `@` show an inline error. The name-only part clears the user's name in `xexam.db`, so it runs with `--backend cargo` only and is skipped with Docker |

`W-V25-AI` (the amber AI button and its dialog) was retired in v2.6; the AI card is checked by `W-V26-CAP`. The page list of `walk.py`
(`--pages`) has a new entry `profile`.


## v2.7 checks (`checks_v27.py`)

`checks_v27.py` (group `g_v27`, registered at the end of `checks.py`; run alone with `--only 'W-V27-*'`), zh / en / ja, light and
dark. It uses the users the walk already seeds: `rc.paging_users()` (12 made-up AI+ users) gives User management a second page and
`lee` (60 records) gives Study records one. No extra seed data was needed (`seed_extra.py` is unchanged).

| Check | What |
|---|---|
| W-V27-QD | Phone answer sheet drawer (sam, practice, 51 questions) at 320 / 360 / 375 / 393 / 430 px: closed = `#qz-pos` button with `aria-expanded` false, drawer hidden and off screen, top row without overlap or sideways scroll; open = flush left and fully on screen, dialog role, backdrop over the whole page, page locked, close button >= 44 px, tiles <= 44 px, the tile list scrolls inside the drawer; closes by backdrop, close button and Escape; a tile tap closes it and goes to that question. At 1280 px the sheet stays a static side rail (no dialog role, no backdrop, no `#qz-pos`). On 320 / 393 px (light) the drawer's history entry is tested too: Back closes it and stays on the quiz, one more Back leaves the quiz in one step (also after a close by button, backdrop or tile), Forward over the entry a close left behind does not reopen it |
| W-V27-QT | Quiz top row, exam (riley) and practice (sam), 320 / 360 / 393 / 430 px, light and dark: a screenshot is kept for every case; at 360-430 px the answer-sheet button and the two-line count are both visible inside the width and sit at the same x (+-1 px) in the exam and the practice |
| W-V27-PG | Shared pager in User management (`#us-*`, admin) and Study records (`#rec-*`, lee) at 320 / 390 / 1280 px: drawn with "Page 1 / n", inside the width, no overlap, no sideways scroll, buttons >= 44 px on phones, page size 5 / 10 (users) and 10 / 20 (records), page 2 marks itself and changes the rows |
| W-V27-UR | The administrator's read-only view (admin, 375 and 1280 px): the book icon `#us-records-<id>` of lee (found on any list page) opens `#/users/<id>/records`: note + back link, no delete / delete-all / AI summary; a record opens read-only (no retry / retake / delete / AI buttons); Back returns to the same list page, and the list's back link returns to User management on the same page |

The walk itself was not run for these checks in the build step; the stack run belongs to the release gate.
