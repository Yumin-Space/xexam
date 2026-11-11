// The AI popover (v0.9): the small closable window of an AI hint, an AI explanation or an AI analysis, looking like the
// hamburger menu's dropdown (`.nav-pop`). One element for the whole app, appended to `document.body`, so redrawing `#app`
// never destroys it. The decisions (placement, what an opener click does, what to show for an answer) are pure
// (domain/ai-popover.ts, domain/ai-study.ts) and the engine (domain/ai-run.ts) owns the entries, the stream and the polling;
// this file only builds the DOM, paints the entry of the open popover and wires the global handlers. Spec 7.8.
//   · installAiPop(deps)   once, from main.ts: the global handlers (Esc, outside pointerdown, hashchange, resize, scroll) and
//                          the sign-out hook (onClearUserData);
//   · openAiPop(args)      from the opener buttons of the practice page and the result pages;
//   · reanchorAiPop()      from main.ts right after every `app.replaceChildren(...)`;
//   · closeAiPop(), clearAiPopData()
import { t } from '../i18n/index.ts';
import { entryView, emptyEntry, type EntryView } from '../domain/ai-study.ts';
import { createRunner, type Runner, type RunSpec } from '../domain/ai-run.ts';
import { busyView, popTitleKey, replyLang, type RunningInfo, type StudyKind, type StudySource } from '../domain/ai-plus.ts';
import {
  escapeClosesPopover, focusOutCloses, focusTitleOnOpen, KEEP_SELECTOR, openerStillMatches, outsidePointerCloses, placePopover, popWidth, togglePlan,
} from '../domain/ai-popover.ts';
import '../styles/ai-md.css';
import '../styles/ai-pop.css';
import { onClearUserData } from '../state/store.ts';
import { copyText } from '../utils/clipboard.ts';
import { h } from '../utils/dom.ts';
import { toast } from '../utils/toast.ts';
import { fillMarkdown } from './ai-render.ts';
import { announce, focusOpener, noteIsAlert, noteText, phaseKey, replaceKeepingFocus, safeInsets, stateText, type StudyUiDeps } from './ai-study-ui.ts';

/** What the page hands over when an opener is clicked */
export interface PopOpenArgs {
  kind: StudyKind;
  qid: string;
  source: StudySource;
  /** The number shown in the title ("Question 3") */
  no: number;
  /** `data-focus` key of the opener that was clicked (the popover is anchored to it and returns focus to it) */
  anchorKey: string;
  /** studyKey(...) of domain/ai-study.ts */
  cacheKey: string;
  /** studySpec(...) of domain/ai-spec.ts */
  spec: RunSpec;
  /** `${mode}|${sessionKey}|`: entries with this key prefix belong to the session (to know the session's chat id) */
  sessionPrefix: string;
  /** The opener sits in the bottom action bar: open upwards */
  fromBar: boolean;
  /** Activated with the keyboard (`event.detail === 0`): focus moves to the title */
  byKeyboard: boolean;
  /** The page's own question for a running answer of this session ([查看]); null = it is not on this page */
  resolveRunning?: (r: RunningInfo) => PopOpenArgs | null;
}

const OPENER = '[data-aip-opener]';
const PAINT_MS = 100;
const COPY_MS = 2500;

let deps: StudyUiDeps | null = null;
let runner: Runner | null = null;
let cur: PopOpenArgs | null = null;
let pop: HTMLElement | null = null;
let sr: HTMLElement | null = null;
const ui: { title?: HTMLElement; note?: HTMLElement; body?: HTMLElement; state?: HTMLElement; acts?: HTMLElement; link?: HTMLAnchorElement } = {};
let paintTimer: number | null = null;
let lastPaintAt = 0;
let lastText: string | null = null;
let lastPhase: string | null = null;
let copyState: { text: string; timer: number } | null = null;
let raf = 0;
let actsSig = '';
let insets: { top: number; bottom: number } | null = null;

const pairOf = (a: { qid: string; kind: string; source: string }): string => `${a.qid}|${a.kind}|${a.source}`;
const viewOf = (): EntryView => entryView((cur && runner?.entry(cur.cacheKey)) || emptyEntry(), replyLang(deps ? deps.uiLang() : 'en'));
const sessionChat = (): number | null => (cur && runner ? runner.chatIdWhere(cur.sessionPrefix) : null);

// ---------------------------------------------------------------- install / open / close

export function installAiPop(d: StudyUiDeps): void {
  deps = d;
  runner = createRunner({
    setTimer: (fn, ms) => window.setTimeout(fn, ms),
    clearTimer: (hd) => window.clearTimeout(hd as number),
    onChange: (k) => { if (cur && k === cur.cacheKey) schedulePaint(); },
    onRefuse: () => d.refreshAi(true),
  });
  document.addEventListener('keydown', (e) => {
    if (e.key === 'Escape' && escapeClosesPopover(cur != null, document.querySelector('dialog[open]') != null)) {
      e.preventDefault();
      closeAiPop(true);
    }
  });
  document.addEventListener('pointerdown', (e) => {
    if (!cur || !pop) return;
    const target = e.target instanceof Element ? e.target : null;
    if (outsidePointerCloses(!!target && pop.contains(target), !!target?.closest(OPENER), !!target?.closest(KEEP_SELECTOR))) closeAiPop(false);
  }, true);
  window.addEventListener('hashchange', () => closeAiPop(false));
  window.addEventListener('resize', () => {
    insets = null;
    schedulePosition();
  });
  window.addEventListener('scroll', schedulePosition, true);
  onClearUserData(clearAiPopData);
}

/** Opens, moves, switches or closes the popover for a click on an opener (domain/ai-popover.ts togglePlan) */
export function openAiPop(args: PopOpenArgs): void {
  if (!deps || !runner) return;
  const plan = togglePlan(cur, { kind: args.kind, qid: args.qid, source: args.source, anchorKey: args.anchorKey });
  if (plan === 'close') {
    closeAiPop(true);
    return;
  }
  cur = args;
  ensurePop();
  lastText = null;
  lastPhase = plan === 'move' ? lastPhase : null;
  markOpeners();
  paint();
  position();
  if (plan !== 'move') {
    deps.refreshAi(false);
    void runner.open(args.cacheKey, args.spec);
  }
  if (focusTitleOnOpen(args.byKeyboard ? 0 : 1)) ui.title?.focus();
}

/** Closes it. `returnFocus`: Esc and × put the focus back on the opener; an outside click or a route change does not */
export function closeAiPop(returnFocus: boolean): void {
  const was = cur;
  cur = null;
  if (paintTimer != null) window.clearTimeout(paintTimer);
  paintTimer = null;
  if (copyState) window.clearTimeout(copyState.timer);
  copyState = null;
  pop?.remove();
  pop = null;
  sr?.replaceChildren();
  markOpeners();
  // The generation is never stopped by closing: it goes on in the background and is saved
  if (was && returnFocus) focusOpener(was.anchorKey);
}

/** Sign-out / another user: aborts streams, stops polling, forgets every answer, closes the popover */
export function clearAiPopData(): void {
  closeAiPop(false);
  runner?.clearAll();
}

/** A chat was deleted (`chatId`), or all of them (null): their saved answers are forgotten, so none is offered as saved */
export function forgetStudyChats(chatId: number | null): void {
  runner?.dropWhere((e) => chatId == null || e.chatId === chatId);
}

/** Right after every page redraw (main.ts): the openers are new elements, so mark them again and re-anchor; gone → close */
export function reanchorAiPop(): void {
  if (!cur) return;
  const opener = document.querySelector(`[data-focus="${cur.anchorKey}"]`);
  // Gone, or now belonging to another question / kind / place (the arrow keys moved on): the content would be stale
  if (!opener || !openerStillMatches(opener.getAttribute('data-aip-pair'), pairOf(cur))) {
    closeAiPop(false);
    return;
  }
  markOpeners();
  position();
}

// ---------------------------------------------------------------- building

function ensurePop(): void {
  if (pop) return;
  actsSig = '';
  ui.title = h('h2', { id: 'aip-pop-h', tabindex: '-1' });
  ui.note = h('p', { id: 'aip-pop-note', class: 'aip-note', hidden: true });
  ui.body = h('div', { id: 'aip-pop-body', class: 'aip-pop-body ai-md', tabindex: '0', role: 'region' });
  ui.state = h('span', { class: 'aip-state' });
  ui.acts = h('div', { class: 'aip-pop-acts' });
  ui.link = h('a', {
    class: 'aip-chatlink', href: '#/ai',
    onclick: (e: MouseEvent) => {
      e.preventDefault();
      const id = runner?.entry(cur?.cacheKey ?? '')?.chatId ?? null;
      if (id != null) {
        closeAiPop(false);
        deps?.openChat(id);
      }
    },
  }, t('aiplus.openChat'));
  pop = h('div', { id: 'aip-pop', class: 'aip-pop', role: 'dialog', 'aria-labelledby': 'aip-pop-h' },
    h('div', { class: 'aip-pop-head' }, ui.title,
      h('button', { class: 'aip-x', type: 'button', 'aria-label': t('aiplus.close'), title: t('aiplus.close'), onclick: () => closeAiPop(true) }, '×')),
    ui.note, ui.body,
    h('div', { class: 'aip-pop-foot' }, ui.state, ui.acts, ui.link));
  // Tab out of either end returns to the opener (the popover sits at the end of <body>, far from the question)
  pop.addEventListener('keydown', (e) => {
    if (e.key !== 'Tab' || !pop || !cur) return;
    const tabbable = [...pop.querySelectorAll<HTMLElement>('button:not([disabled]), a[href], [tabindex="0"]')].filter((el) => !el.hidden && el.closest('[hidden]') == null);
    const first = tabbable[0];
    const last = tabbable[tabbable.length - 1];
    const on = document.activeElement;
    if (e.shiftKey ? on === ui.title || on === first : on === last) {
      e.preventDefault();
      closeAiPop(true);
    }
  });
  // Focus leaving for a known element outside the popover and its openers closes it; a lost focus (a redraw) does not
  pop.addEventListener('focusout', (e) => {
    const rel = e.relatedTarget instanceof Element ? e.relatedTarget : null;
    if (!pop || !cur) return;
    if (focusOutCloses(rel != null, !!rel && pop.contains(rel), !!rel?.closest(OPENER), !!rel?.closest(KEEP_SELECTOR))) closeAiPop(false);
  });
  document.body.append(pop);
  if (!sr) {
    sr = h('div', { id: 'aip-sr', class: 'sr-only', role: 'status', 'aria-live': 'polite' });
    document.body.append(sr);
  }
}

/** aria-expanded / aria-controls on the openers of the open pair (they are new elements after every redraw) */
function markOpeners(): void {
  for (const el of document.querySelectorAll<HTMLElement>(OPENER)) {
    const on = cur != null && el.getAttribute('data-aip-pair') === pairOf(cur);
    el.setAttribute('aria-expanded', String(on));
    if (on) el.setAttribute('aria-controls', 'aip-pop');
    else el.removeAttribute('aria-controls');
  }
}

// ---------------------------------------------------------------- painting

function schedulePaint(): void {
  if (!cur || paintTimer != null) return;
  const wait = Math.max(0, PAINT_MS - (Date.now() - lastPaintAt));
  paintTimer = window.setTimeout(() => {
    paintTimer = null;
    paint();
  }, wait);
}

function paint(): void {
  if (!cur || !pop || !deps || !ui.title || !ui.note || !ui.body || !ui.state || !ui.acts) return;
  lastPaintAt = Date.now();
  const v = viewOf();
  const title = t(popTitleKey(cur.kind, cur.source), { n: cur.no });
  if (ui.title.textContent !== title) ui.title.textContent = title;
  ui.body.setAttribute('aria-label', title);
  ui.body.setAttribute('aria-busy', String(v.phase === 'streaming' || v.phase === 'requesting'));

  // The note: other language, withheld, failed regenerate, busy, error
  const note = v.note ? noteText(v.note, deps, sessionChat()) : '';
  ui.note.hidden = note === '';
  ui.note.textContent = note;
  if (noteIsAlert(v.note)) ui.note.setAttribute('role', 'alert');
  else ui.note.removeAttribute('role');

  // The text (only when it changed: re-parsing a long answer every paint is wasted work)
  if (v.text !== lastText) {
    lastText = v.text;
    const scroller = ui.body;
    const atBottom = scroller.scrollHeight - scroller.scrollTop - scroller.clientHeight < 24;
    fillMarkdown(scroller, v.text);
    // While it streams, follow the text unless the reader scrolled up
    if (v.phase === 'streaming' && atBottom) scroller.scrollTop = scroller.scrollHeight;
  }

  ui.state.textContent = copyState ? copyState.text : stateText(v);
  paintActions(v);
  ui.link!.hidden = (runner?.entry(cur.cacheKey)?.chatId ?? null) == null;

  const say = announce(lastPhase, v, deps);
  lastPhase = phaseKey(v);
  if (say && sr) sr.textContent = say;
  position();
}

/**
 * The buttons are rebuilt only when what they show changes (a signature), never on a streaming repaint: a rebuilt Stop would lose
 * focus and a press straddling the repaint would lose its click. Handlers read the current state when clicked, not when built.
 */
function paintActions(v: EntryView): void {
  if (!cur || !runner || !deps || !ui.acts) return;
  const r = runner;
  const running = v.note?.kind === 'busy' ? v.note.running : null;
  const bv = running ? busyView(running, sessionChat()) : null;
  const other = running && bv && bv.kind === 'study' && bv.canShow ? cur.resolveRunning?.(running) ?? null : null;
  const sig = [cur.cacheKey, v.text.trim() === '', v.canStop, v.canRegenerate, running ? running.chatId : '-', other ? other.anchorKey : '-'].join('|');
  if (sig === actsSig) return;
  actsSig = sig;
  const now = (): PopOpenArgs | null => cur;
  const btns: HTMLElement[] = [];
  btns.push(h('button', {
    class: 'btn quiet aip-mini', type: 'button', disabled: v.text.trim() === '',
    onclick: async () => {
      const ok = await copyText(viewOf().text);
      const msg = t(ok ? 'ai.copied' : 'ai.copyFailed');
      toast(msg, COPY_MS);
      if (copyState) window.clearTimeout(copyState.timer);
      copyState = { text: msg, timer: window.setTimeout(() => { copyState = null; paint(); }, COPY_MS) };
      paint();
    },
  }, t('ai.copy')));
  if (v.canStop) {
    btns.push(h('button', { class: 'btn quiet aip-mini', type: 'button', onclick: () => { const a = now(); if (a) void r.stop(a.cacheKey, a.spec); } }, t('ai.stop')));
  } else if (v.canRegenerate) {
    btns.push(h('button', { class: 'btn quiet aip-mini', type: 'button', onclick: () => { const a = now(); if (a) void r.regenerate(a.cacheKey, a.spec); } }, t('aiplus.regenerate')));
  }
  if (running) {
    btns.push(h('button', { class: 'btn aip-mini', type: 'button', onclick: () => { const a = now(); if (a) void r.stopAndAsk(a.cacheKey, a.spec, running.chatId); } }, t('aiplus.busy.stopAndAsk')));
    if (other) btns.push(h('button', { class: 'btn quiet aip-mini', type: 'button', onclick: () => openAiPop({ ...other, byKeyboard: false }) }, t('aiplus.busy.show')));
  }
  replaceKeepingFocus(ui.acts, btns);
}

// ---------------------------------------------------------------- placement

function schedulePosition(): void {
  if (!cur || raf) return;
  raf = window.requestAnimationFrame(() => {
    raf = 0;
    position();
  });
}

function position(): void {
  if (!cur || !pop) return;
  const opener = document.querySelector<HTMLElement>(`[data-focus="${cur.anchorKey}"]`);
  if (!opener) return;
  const o = opener.getBoundingClientRect();
  const vw = document.documentElement.clientWidth;
  const vh = window.innerHeight;
  const size = pop.getBoundingClientRect();
  const p = placePopover({
    opener: { top: o.top, bottom: o.bottom, left: o.left, right: o.right },
    pop: { width: popWidth(vw), height: size.height }, vw, vh, inset: (insets ??= safeInsets()), preferAbove: cur.fromBar,
  });
  pop.style.top = `${Math.round(p.top)}px`;
  pop.style.left = `${Math.round(p.left)}px`;
  pop.classList.toggle('above', p.placement === 'above');
}
