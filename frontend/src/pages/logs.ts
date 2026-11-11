// System log page (#/logs, v0.7, administrators only; main.ts shows the "Not allowed" page to everyone else). It shows the
// recent log lines the backend keeps in memory (GET /api/logs, at most 2000, gone after a restart) in a monospace list:
//   · it reads new lines every 0.5 seconds while it is open and the browser tab is visible, one request at a time (the poll
//     loop of domain/logs.ts: the next request only after the previous answer; longer waits after failed reads), and stops
//     when the page is left or the tab hidden; Pause / Resume stop and restart it;
//   · new lines are appended at the bottom; the list follows them while it is scrolled to the bottom, and stays where it is
//     when the administrator scrolled up (a "Jump to latest" button then brings it back);
//   · a level filter (All / Warnings / Errors) and a text search change only what is shown;
//   · a gap (lines dropped while the page was paused or hidden for long) is marked in the list.
// The page draws its list itself: a poll never redraws the whole page (the search field keeps its focus and text), and a
// redraw of the page (another state change) keeps the lines, the filters and the scroll position.
import '../styles/logs.css';
import { ApiError } from '../api/client.ts';
import { fetchLogs } from '../api/logs.ts';
import { pageHead } from '../components/common.ts';
import {
  EMPTY_LOG, LEVEL_FILTERS, LOG_CAP, atBottom, createPoller, levelOf, linePasses, mergeChunk, type LevelFilter, type LogLine, type LogState,
} from '../domain/logs.ts';
import { errorText } from '../i18n/errors.ts';
import { t, tn, type MsgKey } from '../i18n/index.ts';
import { onRouteChange, onSignedIn } from '../state/actions.ts';
import { S, isAdmin } from '../state/store.ts';
import { h } from '../utils/dom.ts';

const FILTER_LABEL: Record<LevelFilter, MsgKey> = { all: 'logs.filter.all', warn: 'logs.filter.warn', error: 'logs.filter.error' };

// ---------------------------------------------------------------- page-local state (kept while the app runs)

let log: LogState = EMPTY_LOG;
let level: LevelFilter = 'all';
let query = '';
/** The first answer has arrived */
let loaded = false;
/** Why the last read failed (cleared by the next good one) */
let readError: unknown = null;
/** The list follows new lines (it is scrolled to the bottom) */
let stick = true;
/** The scroll position while not at the bottom (restored after a redraw of the page) */
let savedTop = 0;

/** The elements of the drawn page that a poll updates */
interface View {
  box: HTMLElement;
  list: HTMLOListElement;
  empty: HTMLElement;
  count: HTMLElement;
  jump: HTMLButtonElement;
  status: HTMLElement;
  statusText: HTMLElement;
  pause: HTMLButtonElement;
  err: HTMLElement;
  /** A gap was seen after the last drawn line, but no line was shown after it yet */
  gapPending: boolean;
}
let view: View | null = null;

const onPage = (): boolean => S.route.name === 'logs' && !S.authView && S.phase === 'ready' && isAdmin();

const poller = createPoller({
  fetch: poll,
  visible: () => document.visibilityState !== 'hidden',
  setTimer: (fn, ms) => window.setTimeout(fn, ms),
  clearTimer: (id) => window.clearTimeout(id as number),
});

let installed = false;
function installOnce(): void {
  if (installed) return;
  installed = true;
  document.addEventListener('visibilitychange', () => poller.visibilityChanged());
}

onRouteChange((from, to) => {
  if (from.name === 'logs' && to.name !== 'logs') {
    poller.stop();
    view = null;
    // Next time the page opens at the newest lines
    stick = true;
    savedTop = 0;
  }
});
// A new sign-in (possibly another user): forget the lines and the error, read again
onSignedIn(() => {
  log = EMPTY_LOG;
  loaded = false;
  readError = null;
});

async function poll(): Promise<void> {
  if (!onPage()) {
    poller.stop();
    return;
  }
  try {
    const chunk = await fetchLogs(log.last, log.boot);
    if (!onPage()) return;
    const r = mergeChunk(log, chunk);
    log = r.state;
    const wasBroken = readError != null || !loaded;
    loaded = true;
    readError = null;
    if (r.reset || wasBroken) redrawList();
    else appendLines(r.added);
  } catch (e) {
    // Signed out (401) or no longer an administrator (403): the app shows the sign-in or the "Not allowed" page; stop here
    if (e instanceof ApiError && (e.status === 401 || e.status === 403)) poller.stop();
    readError = e;
    showError();
    // Tell the poll loop the read failed: it then waits longer before the next one (pollDelay in domain/logs.ts)
    throw e;
  }
}

// ---------------------------------------------------------------- drawing the list

function lineEl(l: LogLine): HTMLLIElement {
  const lv = levelOf(l.level);
  // "YYYY-MM-DD HH:MM:SS": phones show the time only (the date is in the tooltip)
  const m = /^(\d{4}-\d{2}-\d{2})[ T](.*)$/.exec(l.time);
  const time = m
    ? h('span', { class: 'lg-time', title: l.time }, h('span', { class: 'lg-date' }, `${m[1]} `), m[2] ?? '')
    : h('span', { class: 'lg-time' }, l.time);
  return h('li', { class: `lg-line lv-${lv}`, 'data-seq': String(l.seq) },
    time, ' ',
    h('span', { class: `lg-lv lv-${lv}` }, lv.toUpperCase()), ' ',
    l.target ? [h('span', { class: 'lg-target' }, l.target), ' '] : null,
    h('span', { class: 'lg-msg' }, l.message));
}

function gapEl(seq: number): HTMLLIElement {
  return h('li', { class: 'lg-gap', 'data-seq': String(seq) }, t('logs.gap', { n: LOG_CAP }));
}

/** The elements for `lines` under the current filters; `gap`: a gap is pending before the first shown line */
function build(lines: readonly LogLine[], gap: boolean): { els: HTMLLIElement[]; gap: boolean } {
  const els: HTMLLIElement[] = [];
  let pending = gap;
  for (const l of lines) {
    if (l.gapBefore) pending = true;
    if (!linePasses(l, level, query)) continue;
    if (pending) els.push(gapEl(l.seq));
    pending = false;
    els.push(lineEl(l));
  }
  return { els, gap: pending };
}

function shownCount(): number {
  return level === 'all' && !query.trim() ? log.lines.length : log.lines.filter((l) => linePasses(l, level, query)).length;
}

/** Count, empty text, jump button and status after any change */
function refreshMeta(): void {
  const v = view;
  if (!v) return;
  const n = log.lines.length;
  const shown = shownCount();
  v.count.textContent = shown === n ? tn('logs.countAll', n) : t('logs.count', { shown, n });
  // Before the first answer: "Reading the log…" (nothing while the first read failed: the error says why)
  v.empty.hidden = loaded ? shown > 0 : readError != null;
  v.empty.textContent = !loaded ? t('logs.loading') : n ? t('logs.noMatch') : t('logs.empty');
  v.list.hidden = shown === 0;
  v.jump.hidden = stick;
  const paused = poller.paused;
  v.status.className = 'lg-status' + (paused ? ' off' : '');
  v.statusText.textContent = t(paused ? 'logs.paused' : 'logs.live');
  v.pause.textContent = t(paused ? 'logs.resume' : 'logs.pause');
  v.pause.title = t(paused ? 'logs.resume.hint' : 'logs.pause.hint');
  v.pause.setAttribute('aria-pressed', String(paused));
}

function toBottom(): void {
  const v = view;
  if (!v) return;
  v.box.scrollTop = v.box.scrollHeight;
}

function redrawList(): void {
  const v = view;
  if (!v) return;
  const b = build(log.lines, false);
  v.list.replaceChildren(...b.els);
  v.gapPending = b.gap;
  showError();
  refreshMeta();
  if (stick) toBottom();
}

function appendLines(added: readonly LogLine[]): void {
  const v = view;
  if (!v) return;
  if (added.length) {
    const b = build(added, v.gapPending);
    v.list.append(...b.els);
    v.gapPending = b.gap;
    // Drop the drawn lines the page no longer keeps (over the cap)
    const first = log.lines[0]?.seq ?? Infinity;
    while (v.list.firstElementChild && Number((v.list.firstElementChild as HTMLElement).dataset.seq) < first) v.list.firstElementChild.remove();
  }
  showError();
  refreshMeta();
  if (stick) toBottom();
}

function showError(): void {
  const v = view;
  if (!v) return;
  v.err.hidden = readError == null;
  v.err.replaceChildren(...(readError == null ? [] : [
    h('p', null, t('logs.err', { reason: errorText(readError) })),
    poller.running ? h('p', { class: 'dim' }, t('logs.retrying')) : null,
  ].filter((x): x is HTMLParagraphElement => !!x)));
}

/** A text whose `backtick` parts are commands */
function withCode(text: string): Array<string | HTMLElement> {
  return text.split('`').map((s, i) => (i % 2 ? h('code', { class: 'lg-code' }, s) : s));
}

// ---------------------------------------------------------------- page

export function renderLogs(): HTMLElement {
  installOnce();

  const segs = LEVEL_FILTERS.map((f) => h('button', {
    type: 'button', 'aria-pressed': String(level === f),
    onclick: () => {
      if (level === f) return;
      level = f;
      for (const [i, b] of segs.entries()) b.setAttribute('aria-pressed', String(LEVEL_FILTERS[i] === level));
      redrawList();
    },
  }, t(FILTER_LABEL[f])));
  const search = h('input', {
    type: 'search', class: 'lg-search', id: 'lg-search', value: query, placeholder: t('logs.search'), 'aria-label': t('logs.search'),
    autocomplete: 'off', spellcheck: 'false',
    oninput: () => {
      query = search.value;
      redrawList();
    },
  });
  const pause = h('button', {
    class: 'btn lg-mini', id: 'lg-pause', type: 'button', 'aria-pressed': 'false',
    onclick: () => {
      poller.setPaused(!poller.paused);
      showError();
      refreshMeta();
    },
  });
  const statusText = h('span');
  const status = h('span', { class: 'lg-status' }, h('span', { class: 'lg-dot', 'aria-hidden': 'true' }), statusText);
  const count = h('span', { class: 'lg-count' });

  const list = h('ol', { class: 'lg-list' });
  const empty = h('p', { class: 'lg-empty', hidden: true });
  const box = h('div', { class: 'lg-box', tabindex: '0', role: 'region', 'aria-label': t('logs.box') }, list, empty);
  const jump = h('button', {
    class: 'btn lg-jump', type: 'button', hidden: true,
    onclick: () => {
      stick = true;
      toBottom();
      refreshMeta();
      box.focus({ preventScroll: true });
    },
  }, t('logs.jump'));
  box.addEventListener('scroll', () => {
    const was = stick;
    stick = atBottom(box.scrollTop, box.clientHeight, box.scrollHeight);
    if (!stick) savedTop = box.scrollTop;
    if (was !== stick && view) view.jump.hidden = stick;
  }, { passive: true });
  const err = h('div', { class: 'err lg-err', role: 'alert', hidden: true });

  view = { box, list, empty, count, jump, status, statusText, pause, err, gapPending: false };
  const b = build(log.lines, false);
  list.append(...b.els);
  view.gapPending = b.gap;
  showError();
  refreshMeta();

  const root = h('div', { class: 'page page-logs' },
    pageHead(t('logs.title')),
    h('p', { class: 'lg-lead' }, t('logs.lead')),
    h('section', { class: 'panel lg-panel', 'aria-label': t('logs.title') },
      h('div', { class: 'lg-bar' },
        h('div', { class: 'seg lg-seg', role: 'group', 'aria-label': t('logs.filter') }, segs),
        search,
        h('div', { class: 'lg-run' }, status, pause)),
      h('div', { class: 'lg-meta' }, count),
      err,
      h('div', { class: 'lg-wrap' }, box, jump),
      h('p', { class: 'lg-note' }, withCode(t('logs.note', { n: LOG_CAP })))));

  // After the page is in the document: back to where the list was (the bottom, or the saved position), and keep reading
  queueMicrotask(() => {
    if (!box.isConnected) return;
    if (stick) toBottom();
    else box.scrollTop = savedTop;
  });
  poller.start();
  return root;
}
