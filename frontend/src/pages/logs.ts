// System log page (#/logs, v0.7, administrators only; main.ts shows the "Not allowed" page to everyone else). It shows the
// recent log lines the backend keeps in memory (GET /api/logs, at most 2000, gone after a restart) in a monospace list:
//   · it reads new lines every 0.5 seconds while it is open and the browser tab is visible, one request at a time (the poll
//     loop of domain/logs.ts: the next request only after the previous answer; longer waits after failed reads), and stops
//     when the page is left or the tab hidden; Pause / Resume stop and restart it;
//   · new lines are appended at the bottom; the list follows them while it is scrolled to the end, and stays where it is
//     when the administrator scrolled up (a "Jump to latest" button then brings it back);
//   · a level filter (All / Warnings / Errors) and a text search change only what is shown;
//   · a gap (lines dropped while the page was paused or hidden for long) is marked in the list.
// v1.0 (phones first, LG-1…5): on a phone the lines have no box of their own, they use the page scroll (a swipe on them moves the
// page), the toolbar is sticky under the top bar and a floating "Jump to latest" button appears when the end is out of view;
// the toolbar is one row (level segments with icons and counts, a search icon that expands into the field, the live dot and the
// Pause / Resume icon); the lead and the note on what is kept are an (i) tip; an entry reads as a level dot + icon, the
// message and the last segment of its target, a tap opens the full time, target and message, and a date divider row marks a new
// day. Wider screens keep the framed box and the one-line entries.
// The page draws its list itself: a poll never redraws the whole page (the search field keeps its focus and text), and a
// redraw of the page (another state change) keeps the lines, the filters and the scroll position.
import '../styles/logs.css';
import { ApiError } from '../api/client.ts';
import { fetchLogs } from '../api/logs.ts';
import { iconButton } from '../components/icon-button.ts';
import { miniIcon, type MiniIcon } from '../components/mini-icons.ts';
import { pageHead } from '../components/common.ts';
import { infoTip } from '../components/tooltip.ts';
import {
  EMPTY_LOG, LEVEL_FILTERS, LOG_CAP, atBottom, createPoller, dateOf, endVisible, lastSegment, levelCounts, levelOf, linePasses, mergeChunk,
  type Level, type LevelFilter, type LogLine, type LogState,
} from '../domain/logs.ts';
import { errorText } from '../i18n/errors.ts';
import { t, tn, type MsgKey } from '../i18n/index.ts';
import { onRouteChange, onSignedIn, render } from '../state/actions.ts';
import { S, isAdmin } from '../state/store.ts';
import { h, keepInView, scrollToY } from '../utils/dom.ts';
import { PHONE_MQ, matches } from '../utils/media.ts';

const FILTER_LABEL: Record<LevelFilter, MsgKey> = { all: 'logs.filter.all', warn: 'logs.filter.warn', error: 'logs.filter.error' };
const FILTER_ICON: Record<LevelFilter, MiniIcon> = { all: 'list', warn: 'warn', error: 'xCircle' };
/** The icon of an entry's level on phones (debug and trace have the dot only) */
const LEVEL_ICON: Record<Level, MiniIcon | null> = { error: 'xCircle', warn: 'warn', info: 'info', debug: null, trace: null };

// ---------------------------------------------------------------- page-local state (kept while the app runs)

let log: LogState = EMPTY_LOG;
let level: LevelFilter = 'all';
let query = '';
/** The search field is open (it opens with the search icon; a kept query keeps it open) */
let searchOpen = false;
/** The first answer has arrived */
let loaded = false;
/** Why the last read failed (cleared by the next good one) */
let readError: unknown = null;
/** The list follows new lines (it is scrolled to the end) */
let stick = true;
/** The scroll position of the box while not at the bottom (restored after a redraw of the page; the box is not used on phones) */
let savedTop = 0;
/** The page is drawn for a phone (LG-1: no inner box, the page scrolls) */
let phone = false;

/** The elements of the drawn page that a poll updates */
interface View {
  box: HTMLElement;
  list: HTMLOListElement;
  empty: HTMLElement;
  count: HTMLElement;
  jump: HTMLButtonElement;
  status: HTMLElement;
  statusText: HTMLElement;
  /** Where the Pause / Resume button sits (rebuilt only when its state changes) */
  runSlot: HTMLElement;
  pausedShown: boolean | null;
  /** The level segments: the button and its count */
  segs: Array<{ btn: HTMLButtonElement; n: HTMLElement; f: LevelFilter }>;
  err: HTMLElement;
  /** A gap was seen after the last drawn line, but no line was shown after it yet */
  gapPending: boolean;
  /** Phones: the date of the last drawn line (a divider row comes when it changes) */
  lastDate: string;
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
  // Phones: the lines use the page scroll, so the page's scroll says whether the reader is at the end
  window.addEventListener('scroll', () => {
    if (!view || !phone || !onPage()) return;
    const was = stick;
    stick = atEnd();
    if (was !== stick) view.jump.hidden = stick;
  }, { passive: true });
  // A phone turned sideways or a window resized across the phone breakpoint: draw the page in the other form
  if (typeof window.matchMedia === 'function') {
    window.matchMedia(PHONE_MQ).addEventListener('change', () => {
      if (view && onPage() && phone !== matches(PHONE_MQ)) render();
    });
  }
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

/** One entry on a wide screen: time, level tag, target, message on one wrapping line */
function lineEl(l: LogLine): HTMLLIElement {
  const lv = levelOf(l.level);
  const time = h('span', { class: 'lg-time' }, h('span', { class: 'lg-date' }, `${dateOf(l.time) ? dateOf(l.time) + ' ' : ''}`), dateOf(l.time) ? l.time.slice(11) : l.time);
  return h('li', { class: `lg-line lv-${lv}`, 'data-seq': String(l.seq) },
    time, ' ',
    h('span', { class: `lg-lv lv-${lv}` }, lv.toUpperCase()), ' ',
    l.target ? [h('span', { class: 'lg-target' }, l.target), ' '] : null,
    h('span', { class: 'lg-msg' }, l.message));
}

/**
 * One entry on a phone (LG-4): a level dot + icon (the level word stays for screen readers), the message, the last segment of
 * the target muted at the end; a tap opens the full time and target under it (the message is then not clamped).
 */
function phoneLineEl(l: LogLine): HTMLLIElement {
  const lv = levelOf(l.level);
  const ic = LEVEL_ICON[lv];
  const detail = h('div', { class: 'lg-detail', hidden: true },
    h('span', { class: 'lg-time' }, l.time),
    l.target ? h('span', { class: 'lg-target' }, l.target) : null);
  const row = h('button', {
    type: 'button', class: 'lg-row', 'aria-expanded': 'false',
    onclick: () => {
      const open = row.getAttribute('aria-expanded') !== 'true';
      row.setAttribute('aria-expanded', String(open));
      detail.hidden = !open;
      li.classList.toggle('open', open);
    },
  },
  h('span', { class: 'lg-ic', 'aria-hidden': 'true' }, ic ? miniIcon(ic, 'lg-ic-i') : null),
  h('span', { class: 'sr-only' }, `${lv.toUpperCase()}: `),
  h('span', { class: 'lg-msg' }, l.message),
  l.target ? h('span', { class: 'lg-tail' }, lastSegment(l.target)) : null);
  const li = h('li', { class: `lg-line lg-phone lv-${lv}`, 'data-seq': String(l.seq) }, row, detail);
  return li;
}

function gapEl(seq: number): HTMLLIElement {
  return h('li', { class: 'lg-gap', 'data-seq': String(seq) }, t('logs.gap', { n: LOG_CAP }));
}

/** A date divider row (phones: the lines show no date, LG-4) */
function dateEl(date: string, seq: number): HTMLLIElement {
  return h('li', { class: 'lg-day', 'data-seq': String(seq), role: 'separator', 'aria-label': date }, h('span', null, date));
}

/** The elements for `lines` under the current filters; `gap`: a gap is pending before the first shown line */
function build(lines: readonly LogLine[], gap: boolean, lastDate: string): { els: HTMLLIElement[]; gap: boolean; lastDate: string } {
  const els: HTMLLIElement[] = [];
  let pending = gap;
  let date = lastDate;
  for (const l of lines) {
    if (l.gapBefore) pending = true;
    if (!linePasses(l, level, query)) continue;
    if (pending) els.push(gapEl(l.seq));
    pending = false;
    if (phone) {
      const d = dateOf(l.time);
      if (d && d !== date) {
        els.push(dateEl(d, l.seq));
        date = d;
      }
    }
    els.push(phone ? phoneLineEl(l) : lineEl(l));
  }
  return { els, gap: pending, lastDate: date };
}

function shownCount(): number {
  return level === 'all' && !query.trim() ? log.lines.length : log.lines.filter((l) => linePasses(l, level, query)).length;
}

/** Whether the reader is at the end of the lines (the box's bottom, or on a phone the last line within reach of the viewport) */
function atEnd(): boolean {
  const v = view;
  if (!v) return true;
  if (phone) return v.list.hidden || endVisible(v.list.getBoundingClientRect().bottom, window.innerHeight);
  return atBottom(v.box.scrollTop, v.box.clientHeight, v.box.scrollHeight);
}

function toBottom(): void {
  const v = view;
  if (!v) return;
  if (!phone) {
    v.box.scrollTop = v.box.scrollHeight;
    return;
  }
  if (v.list.hidden) return;
  const r = v.list.getBoundingClientRect();
  const y = window.scrollY + r.bottom - window.innerHeight + 12;
  if (y > window.scrollY) scrollToY(y, false);
}

/** The Pause / Resume icon button: the icon shows what a tap does */
function pauseButton(paused: boolean): HTMLButtonElement {
  const b = iconButton({
    icon: paused ? 'play' : 'pause', label: t(paused ? 'logs.resume' : 'logs.pause'), focusKey: 'lg-pause',
    onClick: () => {
      poller.setPaused(!poller.paused);
      showError();
      refreshMeta();
    },
  });
  b.id = 'lg-pause';
  return b;
}

/** Count, empty text, jump button, level counts and status after any change */
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
  const counts = levelCounts(log.lines);
  for (const s of v.segs) {
    s.n.textContent = String(counts[s.f]);
    s.btn.setAttribute('aria-label', t('logs.filter.named', { name: t(FILTER_LABEL[s.f]), n: counts[s.f] }));
  }
  const paused = poller.paused;
  v.status.className = 'lg-status' + (paused ? ' off' : '');
  v.statusText.textContent = t(paused ? 'logs.paused' : 'logs.live');
  if (v.pausedShown !== paused) {
    const had = v.runSlot.contains(document.activeElement);
    v.runSlot.replaceChildren(pauseButton(paused));
    v.pausedShown = paused;
    if (had) document.getElementById('lg-pause')?.focus({ preventScroll: true });
  }
}

function redrawList(): void {
  const v = view;
  if (!v) return;
  const b = build(log.lines, false, '');
  v.list.replaceChildren(...b.els);
  v.gapPending = b.gap;
  v.lastDate = b.lastDate;
  showError();
  refreshMeta();
  if (stick) toBottom();
}

function appendLines(added: readonly LogLine[]): void {
  const v = view;
  if (!v) return;
  if (added.length) {
    const b = build(added, v.gapPending, v.lastDate);
    v.list.append(...b.els);
    v.gapPending = b.gap;
    v.lastDate = b.lastDate;
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

/** The lead and the note on what is kept as one plain text for the (i) tip (the `backtick` marks of the command are dropped) */
function tipText(): string {
  return `${t('logs.lead')} ${t('logs.note', { n: LOG_CAP }).replace(/`/g, '')}`;
}

// ---------------------------------------------------------------- page

export function renderLogs(): HTMLElement {
  installOnce();
  phone = matches(PHONE_MQ);
  if (query.trim()) searchOpen = true;

  const segs: View['segs'] = LEVEL_FILTERS.map((f) => {
    const n = h('span', { class: 'lg-seg-n', 'aria-hidden': 'true' });
    const btn = h('button', {
      type: 'button', class: 'lg-segb', 'aria-pressed': String(level === f),
      onclick: () => {
        if (level === f) return;
        level = f;
        for (const s of segs) s.btn.setAttribute('aria-pressed', String(s.f === level));
        redrawList();
      },
    }, miniIcon(FILTER_ICON[f], 'lg-seg-i'), h('span', { class: 'lg-seg-l' }, t(FILTER_LABEL[f])), n);
    return { btn, n, f };
  });

  const search = h('input', {
    type: 'search', class: 'lg-search', id: 'lg-search', value: query, placeholder: t('logs.search'), 'aria-label': t('logs.search'),
    autocomplete: 'off', spellcheck: 'false',
    oninput: () => {
      query = search.value;
      redrawList();
    },
    onkeydown: (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        e.preventDefault();
        collapse();
      }
    },
  });
  keepInView(search);
  const openBtn = iconButton({
    icon: 'search', label: t('logs.search'), focusKey: 'lg-search-open',
    onClick: () => {
      searchOpen = true;
      syncSearch();
      search.focus();
    },
  });
  const closeBtn = iconButton({ icon: 'x', label: t('logs.search.close'), onClick: () => collapse() });
  const searchBox = h('div', { class: 'lg-searchbox' }, search, closeBtn);
  const syncSearch = (): void => {
    openBtn.hidden = searchOpen;
    searchBox.hidden = !searchOpen;
  };
  /** Closing the field also clears the search, so no filter is left that nobody sees */
  const collapse = (): void => {
    const had = query !== '';
    searchOpen = false;
    query = '';
    search.value = '';
    syncSearch();
    if (had) redrawList();
    openBtn.focus({ preventScroll: true });
  };
  syncSearch();

  const statusText = h('span', { class: 'lg-status-t' });
  const status = h('span', { class: 'lg-status' }, h('span', { class: 'lg-dot', 'aria-hidden': 'true' }), statusText);
  const runSlot = h('span', { class: 'lg-run-btn' });
  const count = h('span', { class: 'lg-count' });
  const tip = infoTip('lg-tip', t('logs.tip.aria'), tipText(), 'lg-info');

  const list = h('ol', { class: 'lg-list' });
  const empty = h('p', { class: 'lg-empty', hidden: true });
  // On a phone the box has no frame, no scroller and no tab stop: the lines use the page scroll (LG-1)
  const box = h('div', { class: 'lg-box', tabindex: phone ? null : '0', role: 'region', 'aria-label': t('logs.box') }, list, empty);
  const jump = iconButton({
    icon: 'down', label: t('logs.jump'), showLabel: !phone, focusKey: 'lg-jump',
    onClick: () => {
      stick = true;
      toBottom();
      refreshMeta();
    },
  });
  jump.classList.add('lg-jump');
  jump.hidden = true;
  box.addEventListener('scroll', () => {
    if (phone) return;
    const was = stick;
    stick = atBottom(box.scrollTop, box.clientHeight, box.scrollHeight);
    if (!stick) savedTop = box.scrollTop;
    if (was !== stick && view) view.jump.hidden = stick;
  }, { passive: true });
  const err = h('div', { class: 'err lg-err', role: 'alert', hidden: true });

  view = { box, list, empty, count, jump, status, statusText, runSlot, pausedShown: null, segs, err, gapPending: false, lastDate: '' };
  const b = build(log.lines, false, '');
  list.append(...b.els);
  view.gapPending = b.gap;
  view.lastDate = b.lastDate;
  showError();
  refreshMeta();

  const root = h('div', { class: 'page page-logs' },
    pageHead(t('logs.title')),
    h('section', { class: 'panel lg-panel', 'aria-label': t('logs.title') },
      h('div', { class: 'lg-bar' },
        h('div', { class: 'seg lg-seg', role: 'group', 'aria-label': t('logs.filter') }, segs.map((s) => s.btn)),
        h('div', { class: 'lg-searchslot' }, openBtn, searchBox),
        h('div', { class: 'lg-run' }, status, runSlot)),
      h('div', { class: 'lg-meta' }, count, tip.el),
      err,
      h('div', { class: 'lg-wrap' }, box, jump)));

  // After the page is in the document: back to where the list was (the end, or the saved position of the box), and keep reading
  queueMicrotask(() => {
    if (!box.isConnected) return;
    if (phone) {
      if (stick) toBottom();
      else jump.hidden = false;
    } else if (stick) toBottom();
    else box.scrollTop = savedTop;
  });
  poller.start();
  return root;
}
