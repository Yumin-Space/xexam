// The AI+ panel (v1.0, spec section 2): one panel element for the whole app, docked INSIDE the page under the practice question
// card (`[data-aip-dock="practice"]`) or under the answer line of a result / record-detail card (`[data-aip-dock="rv-<n>"]`).
// It is collapsed by default; AI 提示 / AI 解释 / a card's AI 解析 expand it and show that answer; the chevron collapses it. On
// result / detail pages every card whose dock does not hold the live panel shows a collapsed bar of its own (a stub).
// Changing question on the practice page keeps the panel as it is and follows the same kind on the new question (the saved
// answer, or a short "not asked yet" text): a question change never sends a request.
// The decisions (what a press / a chevron tap / a question change does, how far to scroll, which footer slot is usable, when
// the cap callout shows) are pure (domain/ai-panel.ts); the engine (domain/ai-run.ts) owns the entries, the stream and the
// polling; this file only builds the DOM, paints the shown entry and wires the handlers.
//   · installAiPanel(deps)   once, from main.ts (hashchange closes; sign-out clears);
//   · openAiPanel(args)      from the AI 提示 / AI 解释 / AI 解析 buttons;
//   · reanchorAiPanel()      from main.ts right after every redraw: moves the panel into the new dock, follows a question change;
//   · closeAiPanel(), clearAiPanelData(), forgetStudyChats()
import { t } from '../i18n/index.ts';
import { entryView, emptyEntry, CALL_LIMIT, type EntryView } from '../domain/ai-study.ts';
import { createRunner, type Runner, type RunSpec } from '../domain/ai-run.ts';
import { busyView, chatNameOf, popTitleKey, replyLang, type RunningInfo, type StudyKind, type StudySource } from '../domain/ai-plus.ts';
import {
  capNote, cardAnchorPlan, chevronPlan, createAlertGate, focusAfterPress, followsText, footerSlots, headVisible, isNotAsked, pairOf, pressPlan,
  questionChangePlan, regenerateAllowed, savedInChat, scrollDelta, type PanelState,
} from '../domain/ai-panel.ts';
import type { BadgeState } from '../domain/badge-state.ts';
import '../styles/ai-md.css';
import '../styles/ai-panel.css';
import { callBlock, noteCallBlock, onCallBlockChange, recheckCallBlock } from '../state/ai.ts';
import { onClearUserData } from '../state/store.ts';
import { copyText } from '../utils/clipboard.ts';
import { h, scrollToY } from '../utils/dom.ts';
import { BAR_MQ, PHONE_MQ, TOUCH_MQ, matches } from '../utils/media.ts';
import { toast } from '../utils/toast.ts';
import { aiBadge } from './ai-badge.ts';
import { fillMarkdown } from './ai-render.ts';
import { miniIcon, type MiniIcon } from './mini-icons.ts';
import { withReason } from './icon-button.ts';
import { holdTip } from './tooltip.ts';
import { announce, focusOpener, noteIsAlert, noteText, phaseKey, truncatedText, replaceKeepingFocus, stateText, type StudyUiDeps } from './ai-study-ui.ts';

/** What the page hands over when a button is clicked */
export interface PanelOpenArgs {
  kind: StudyKind;
  qid: string;
  source: StudySource;
  /** The number shown in the title ("Question 3") */
  no: number;
  /** `data-focus` key of the button that was clicked */
  anchorKey: string;
  /** studyKey(...) of domain/ai-study.ts */
  cacheKey: string;
  /** studySpec(...) of domain/ai-spec.ts */
  spec: RunSpec;
  /** `${mode}|${sessionKey}|`: entries with this key prefix belong to the session (to know the session's chat id) */
  sessionPrefix: string;
  /** The button sits in the bottom action bar */
  fromBar: boolean;
  /** Activated with the keyboard (`event.detail === 0`) */
  byKeyboard: boolean;
  /** `data-aip-dock` value of the place the panel belongs to: 'practice' or 'rv-<n>' */
  dock: string;
  /** The page's own question for a running answer of this session ([查看]); null = it is not on this page */
  resolveRunning?: (r: RunningInfo) => PanelOpenArgs | null;
}

/** A button that can rebuild its PanelOpenArgs (so a question change can follow the same kind on the new question) */
export type AipOpenerEl = HTMLElement & { __aipArgs?: () => PanelOpenArgs };

/** A dock of a result / record-detail card carries the card's AI 解析 arguments (spec 2.12 b); the practice dock carries none */
export type AipDockEl = HTMLElement & { __aipArgs?: () => PanelOpenArgs };

const OPENER = '[data-aip-opener]';
const PAINT_MS = 100;
const COPY_MS = 1500;

let deps: StudyUiDeps | null = null;
let runner: Runner | null = null;
/** The answer the panel shows; null = idle (practice page, nothing chosen yet) */
let cur: PanelOpenArgs | null = null;
let collapsed = true;
/** The `data-aip-dock` key the live panel sits in; null = not mounted */
let mountedKey: string | null = null;
let panel: HTMLElement | null = null;
let sr: HTMLElement | null = null;
const ui: {
  head?: HTMLElement; title?: HTMLElement; state?: HTMLElement; dot?: HTMLElement; ask?: HTMLButtonElement; askLbl?: HTMLElement; chev?: HTMLButtonElement;
  inner?: HTMLElement; noteRow?: HTMLElement; note?: HTMLElement; noteActs?: HTMLElement; body?: HTMLElement; acts?: HTMLElement; copy?: HTMLButtonElement;
} = {};
let paintTimer: number | null = null;
let lastPaintAt = 0;
let lastText: string | null = null;
let lastPhase: string | null = null;
let lastNoteSig = '';
let bodyKey = '';
let actsSig = '';
let noteActsSig = '';
let copyStatus: 'idle' | 'ok' | 'fail' = 'idle';
let copyTimer: number | null = null;
const alertGate = createAlertGate();
/**
 * After a press on a phone the panel often grows a moment later (the saved answer arrives, the text streams in), and a
 * short page could not scroll it far enough at the press. Until the reader touches, scrolls or types, or FOLLOW_MS pass, a
 * paint that changed the panel's height repeats the press scroll (same rule, scrollDelta; downwards only). Never after a
 * chevron tap, a question change or a re-render: only a press starts it.
 */
const FOLLOW_MS = 3000;
let follow: { key: string; until: number; height: number } | null = null;
const stopFollow = (): void => {
  follow = null;
};

const nowSec = (): number => Math.floor(Date.now() / 1000);
const viewFor = (a: PanelOpenArgs | null): EntryView =>
  entryView((a && runner?.entry(a.cacheKey)) || emptyEntry(), replyLang(deps ? deps.uiLang() : 'en'), nowSec());
const viewOf = (): EntryView => viewFor(cur);
const sessionChat = (): number | null => (cur && runner ? runner.chatIdWhere(cur.sessionPrefix) : null);
const stateNow = (): PanelState => ({ pair: cur ? pairOf(cur) : null, dock: cur ? cur.dock : mountedKey, collapsed });
const isCardKey = (k: string | null): boolean => k != null && k !== 'practice';
const dockEl = (key: string): AipDockEl | null => document.querySelector<AipDockEl>(`[data-aip-dock="${key}"]`);
/** The label of the shown kind (AI 提示 / AI 解释 / AI 解析) */
const labelOf = (a: PanelOpenArgs): string => t(a.kind === 'hint' ? 'aiplus.hint' : a.source === 'practice' ? 'aiplus.explain' : 'aiplus.analyze');
/** The text the cap refusal says now (the callout and Regenerate's tip) */
const capText = (): string => {
  const q = callBlock();
  return q && deps ? deps.errorText({ status: 429, code: CALL_LIMIT, quota: q }) : '';
};

/** Resets what the paints remember, so the next paint redraws everything for another answer */
function resetView(): void {
  lastText = null;
  lastPhase = null;
  lastNoteSig = '';
  bodyKey = '';
  noteActsSig = '';
  actsSig = '';
}

// ---------------------------------------------------------------- install / open / close

export function installAiPanel(d: StudyUiDeps): void {
  deps = d;
  runner = createRunner({
    setTimer: (fn, ms) => window.setTimeout(fn, ms),
    clearTimer: (hd) => window.clearTimeout(hd as number),
    onChange: (k) => {
      if (cur && k === cur.cacheKey) schedulePaint();
      refreshStubs(k);
    },
    onRefuse: () => d.refreshAi(true),
    blocked: () => callBlock(),
    onCallLimit: (q) => noteCallBlock(q),
  });
  // The cap ended or changed (timer, status read): the callout and the Regenerate slot follow without a reload
  onCallBlockChange(() => { if (cur) schedulePaint(); });
  // The reader takes over the scrolling: the press scroll stops following the panel
  for (const ev of ['touchstart', 'wheel', 'keydown']) window.addEventListener(ev, stopFollow, { capture: true, passive: true });
  window.addEventListener('hashchange', () => closeAiPanel(false));
  onClearUserData(clearAiPanelData);
}

/**
 * A button was pressed (AI 提示, AI 解释, a card's AI 解析). The plan (domain/ai-panel.ts pressPlan): another answer → switch,
 * expand, show it; the answer already shown → ask only when its entry is empty, else just expand and bring the panel into view.
 */
export function openAiPanel(args: PanelOpenArgs): void {
  if (!deps || !runner) return;
  const e = runner.entry(args.cacheKey);
  const notAsked = isNotAsked(e ? viewFor(args) : null);
  const plan = pressPlan(stateNow(), { pair: pairOf(args), dock: args.dock }, notAsked);
  const stubFocus = focusInStub();
  cur = args;
  collapsed = !plan.expand;
  if (plan.switchTo) resetView();
  if (!mount()) return;
  fillStubs();
  markOpeners();
  if (plan.switchTo) deps.refreshAi(false);
  if (plan.action === 'open') void runner.open(args.cacheKey, args.spec);
  else if (plan.action === 'ask') void runner.regenerate(args.cacheKey, args.spec);
  paint();
  if (plan.scroll) afterPress(args.byKeyboard, stubFocus);
}

/** Closes it (a route change, sign-out): the panel goes, the cards get their bars back. A generation is never stopped by closing */
export function closeAiPanel(returnFocus: boolean): void {
  const was = cur;
  cur = null;
  collapsed = true;
  if (paintTimer != null) window.clearTimeout(paintTimer);
  paintTimer = null;
  if (copyTimer != null) window.clearTimeout(copyTimer);
  copyTimer = null;
  copyStatus = 'idle';
  resetView();
  panel?.remove();
  mountedKey = null;
  sr?.replaceChildren();
  fillStubs();
  markOpeners();
  if (was && returnFocus) focusOpener(was.anchorKey);
}

/** Sign-out / another user: aborts streams, stops polling, forgets every answer, closes the panel */
export function clearAiPanelData(): void {
  closeAiPanel(false);
  runner?.clearAll();
}

/** A chat was deleted (`chatId`), or all of them (null): their saved answers are forgotten, so none is offered as saved */
export function forgetStudyChats(chatId: number | null): void {
  runner?.dropWhere((e) => chatId == null || e.chatId === chatId);
}

/**
 * Right after every redraw: the docks and buttons are new elements. Practice: the opener found by `data-focus` says which
 * question the page shows now; the panel follows it with the same kind (peek only, never a request). Cards: the dock must
 * still exist and carry the shown answer, else the panel forgets it. Then the panel moves into the new dock.
 */
export function reanchorAiPanel(): void {
  if (!deps || !runner) return;
  if (cur) {
    if (cur.dock === 'practice') {
      const opener = document.querySelector<AipOpenerEl>(`[data-focus="${cur.anchorKey}"]`);
      const next = opener?.__aipArgs?.() ?? null;
      const plan = questionChangePlan(
        { pair: pairOf(cur), kind: cur.kind },
        opener ? { pair: next ? pairOf(next) : opener.getAttribute('data-aip-pair') ?? '', kind: next?.kind ?? cur.kind } : null,
      );
      if (plan.change === 'forget') {
        cur = null;
        collapsed = true;
        resetView();
      } else {
        if (next) cur = next;
        if (plan.change === 'follow') {
          resetView();
          if (plan.peek && cur) void runner.peek(cur.cacheKey, cur.spec);
        }
      }
    } else {
      const args = dockEl(cur.dock)?.__aipArgs?.() ?? null;
      if (cardAnchorPlan(pairOf(cur), args ? pairOf(args) : null) === 'keep' && args) {
        cur = args;
      } else {
        cur = null;
        collapsed = true;
        resetView();
      }
    }
  }
  const ok = mount();
  fillStubs();
  markOpeners();
  if (ok) paint();
}

// ---------------------------------------------------------------- docks and stubs

/** Puts the live panel into its dock (the shown answer's dock, or the practice page's idle dock); false = no dock on this page */
function mount(): boolean {
  const key = cur ? cur.dock : 'practice';
  const dock = dockEl(key);
  if (!dock) {
    cur = null;
    collapsed = true;
    panel?.remove();
    mountedKey = null;
    return false;
  }
  ensurePanel();
  mountedKey = key;
  // The card opener's pair must be set before markOpeners() runs, or its pressed look lags one card behind
  if (cur && ui.ask && isCardKey(key)) {
    ui.ask.setAttribute('data-aip-pair', pairOf(cur));
    ui.ask.setAttribute('data-focus', cur.anchorKey);
  }
  if (panel && panel.parentElement !== dock) {
    const top = ui.body?.scrollTop ?? 0;
    dock.querySelector(':scope > .aip-stub')?.remove();
    dock.append(panel);
    if (ui.body) ui.body.scrollTop = top;
  }
  return true;
}

/** Result / record-detail cards: every usable card dock without the live panel shows a collapsed bar of its own */
function fillStubs(): void {
  for (const dock of document.querySelectorAll<AipDockEl>('[data-aip-dock]')) {
    const key = dock.getAttribute('data-aip-dock') ?? '';
    if (!isCardKey(key)) continue;
    const args = dock.__aipArgs?.();
    if (!args) continue;
    const stub = dock.querySelector(':scope > .aip-stub');
    if (panel && dock.contains(panel)) {
      stub?.remove();
    } else if (!stub) {
      dock.append(buildStub(dock, args));
    }
  }
}

/** A runner entry changed: the bar of the card that owns it (when it is not the live panel) says so in its head */
function refreshStubs(key: string): void {
  for (const stub of document.querySelectorAll<HTMLElement>('.aip-stub')) {
    if (stub.getAttribute('data-stub-key') !== key) continue;
    const el = stub.querySelector<HTMLElement>('.aip-head-state');
    const dot = stub.querySelector<HTMLElement>('.aip-dot');
    const v = viewForKey(key);
    if (el) el.textContent = v ? stateText(v) : '';
    if (dot) dot.hidden = !(v && (v.phase === 'requesting' || v.phase === 'streaming'));
  }
}

const viewForKey = (key: string): EntryView | null => {
  const e = runner?.entry(key);
  return e && deps ? entryView(e, replyLang(deps.uiLang()), nowSec()) : null;
};

/** The card's collapsed bar: exactly the card head (badge, state, AI 解析, chevron) */
function buildStub(dock: AipDockEl, args: PanelOpenArgs): HTMLElement {
  const v = viewForKey(args.cacheKey);
  const fresh = (): PanelOpenArgs => dock.__aipArgs?.() ?? args;
  const busy = v != null && (v.phase === 'requesting' || v.phase === 'streaming');
  const head = h('div', { class: 'aip-panel-head', onclick: (e: MouseEvent) => { if (!onButton(e)) onChevron(fresh()); } },
    badge(),
    h('span', { class: 'aip-head-state' }, v ? stateText(v) : ''),
    h('span', { class: 'aip-dot', 'aria-hidden': 'true', hidden: !busy }),
    askButton(fresh).el,
    chevButton(true, () => onChevron(fresh())));
  return h('section', {
    class: 'aip-panel collapsed aip-stub aip-card', 'aria-label': t(popTitleKey(args.kind, args.source), { n: args.no }), 'data-stub-key': args.cacheKey,
  }, head);
}

/** The decorative badge of a panel head (bright; the head's text says what it is) */
function badge(): HTMLElement | null {
  const b = aiBadge({ kind: 'on' } as BadgeState);
  b?.setAttribute('aria-hidden', 'true');
  b?.removeAttribute('role');
  b?.removeAttribute('aria-label');
  return b;
}

/** A click that landed on a button or link inside the head (the head's own pointer toggle ignores those) */
const onButton = (e: Event): boolean => e.target instanceof Element && e.target.closest('button, a') != null;

/** `button.aip-ask`: the card's AI 解析 opener (icon + text), built from the card's arguments */
function askButton(args: () => PanelOpenArgs | null): { el: HTMLButtonElement; lbl: HTMLElement } {
  const lbl = h('span', { class: 'ibtn-lbl' }, t('aiplus.analyze'));
  const el = h('button', {
    class: 'ibtn ibtn-ai ibtn-label aip-ask', type: 'button', 'aria-expanded': 'false', 'data-aip-opener': '',
    onclick: (e: MouseEvent) => {
      const a = args();
      if (a) openAiPanel({ ...a, byKeyboard: e.detail === 0 });
    },
  }, miniIcon('book', 'ibtn-ic'), lbl);
  const a0 = args();
  if (a0) {
    el.setAttribute('data-aip-pair', pairOf(a0));
    el.setAttribute('data-focus', a0.anchorKey);
  }
  (el as AipOpenerEl).__aipArgs = () => args() as PanelOpenArgs;
  return { el, lbl };
}

/** The chevron button (a CSS-drawn chevron that turns); its label is the state it will switch to */
function chevButton(isCollapsed: boolean, onClick: () => void): HTMLButtonElement {
  const label = t(isCollapsed ? 'aiplus.panel.expand' : 'aiplus.panel.collapse');
  return h('button', {
    class: 'aip-chev', type: 'button', 'aria-expanded': String(!isCollapsed), 'aria-label': label, title: matches(TOUCH_MQ) ? null : label,
    onclick: (e: MouseEvent) => {
      e.stopPropagation();
      onClick();
    },
  }, h('span', { class: 'aip-chev-i', 'aria-hidden': 'true' }));
}

/** Where the keyboard focus sits when it is inside a card's collapsed bar (the bar is replaced when the panel moves in) */
function focusInStub(): 'ask' | 'chev' | null {
  const a = document.activeElement;
  if (!a || a.closest('.aip-stub') == null) return null;
  return a.closest('.aip-ask') ? 'ask' : a.closest('.aip-chev') ? 'chev' : null;
}

// ---------------------------------------------------------------- building

function ensurePanel(): void {
  if (panel) return;
  actsSig = '';
  const ask = askButton(() => (mountedKey && isCardKey(mountedKey) ? dockEl(mountedKey)?.__aipArgs?.() ?? cur : null));
  ui.ask = ask.el;
  ui.askLbl = ask.lbl;
  ui.title = h('h2', { id: 'aip-panel-h', class: 'aip-panel-title', tabindex: '-1' });
  ui.state = h('span', { class: 'aip-head-state' });
  ui.dot = h('span', { class: 'aip-dot', 'aria-hidden': 'true', hidden: true });
  ui.chev = chevButton(true, () => onChevron(null));
  ui.chev.setAttribute('aria-controls', 'aip-panel-in');
  ui.head = h('div', { class: 'aip-panel-head', onclick: (e: MouseEvent) => { if (!onButton(e)) onChevron(null); } },
    badge(), ui.title, ui.state, ui.dot, ui.ask, ui.chev);
  ui.note = h('p', { id: 'aip-panel-note', class: 'aip-note' });
  ui.noteActs = h('div', { class: 'aip-note-acts' });
  ui.noteRow = h('div', { class: 'aip-noterow', hidden: true }, ui.note, ui.noteActs);
  ui.body = h('div', { id: 'aip-panel-body', class: 'aip-panel-body ai-md', tabindex: '0', role: 'region' });
  ui.acts = h('div', { class: 'aip-panel-foot' });
  ui.inner = h('div', { id: 'aip-panel-in', class: 'aip-panel-in', hidden: true }, ui.noteRow, ui.body, ui.acts);
  panel = h('section', { id: 'aip-panel', class: 'aip-panel collapsed', 'aria-labelledby': 'aip-panel-h' }, ui.head, ui.inner);
  // Escape inside the panel collapses it and puts the focus on the chevron (the panel is part of the page, no global handler)
  panel.addEventListener('keydown', (e) => {
    if (e.key !== 'Escape' || collapsed) return;
    e.preventDefault();
    e.stopPropagation();
    collapsed = true;
    markOpeners();
    paint();
    ui.chev?.focus();
  });
  if (!sr) {
    sr = h('div', { id: 'aip-sr', class: 'sr-only', role: 'status', 'aria-live': 'polite' });
    document.body.append(sr);
  }
}

/**
 * The chevron or a tap on a head (domain/ai-panel.ts chevronPlan): never a request, never a scroll. `target` = the card whose
 * collapsed bar was tapped (the panel moves there, expanded, and shows the saved analysis or the "not asked" text); null = the
 * live panel's own head (it toggles). A running answer is not stopped by collapsing.
 */
function onChevron(target: PanelOpenArgs | null): void {
  if (!deps || !runner) return;
  const plan = chevronPlan(stateNow(), target ? { pair: pairOf(target), dock: target.dock } : null);
  const stubFocus = target ? focusInStub() : null;
  if (plan.switchTo && target) {
    cur = target;
    resetView();
    if (!mount()) return;
  }
  collapsed = plan.collapsed;
  fillStubs();
  markOpeners();
  if (plan.action === 'peek' && cur) void runner.peek(cur.cacheKey, cur.spec);
  paint();
  if (stubFocus) ui.chev?.focus({ preventScroll: true });
}

/** aria-expanded / aria-controls and the pressed look on the openers whose answer the expanded panel shows */
function markOpeners(): void {
  for (const el of document.querySelectorAll<HTMLElement>(OPENER)) {
    const on = cur != null && !collapsed && el.getAttribute('data-aip-pair') === pairOf(cur);
    el.setAttribute('aria-expanded', String(on));
    el.classList.toggle('aip-on', on);
    if (on) el.setAttribute('aria-controls', 'aip-panel-in');
    else el.removeAttribute('aria-controls');
  }
}

// ---------------------------------------------------------------- scrolling and focus after a press

const topInset = (): number => parseFloat(getComputedStyle(document.documentElement).getPropertyValue('--top-h')) || 0;

/** How much of the window's bottom the pinned action bar covers (BAR_MQ: it is sticky under the panel); 0 when it is not on screen */
function bottomInset(vh: number): number {
  if (!matches(BAR_MQ)) return 0;
  const bar = document.querySelector('.pfoot');
  return bar ? Math.max(0, Math.min(vh - bar.getBoundingClientRect().top, bar.getBoundingClientRect().height)) : 0;
}

/** After a press (never after a chevron tap, a question change or a re-render): scroll the panel into view, then place the focus */
function afterPress(byKeyboard: boolean, stubFocus: 'ask' | 'chev' | null): void {
  if (!panel || !ui.head) return;
  const vh = window.innerHeight;
  const topH = topInset();
  const head = ui.head.getBoundingClientRect();
  const wasVisible = headVisible(head.top, vh, topH);
  const delta = scrollDelta({
    headTop: head.top, panelBottom: panel.getBoundingClientRect().bottom, vh, topH, phone: matches(PHONE_MQ), bottomH: bottomInset(vh),
  });
  if (delta !== 0) scrollToY(window.scrollY + delta, true);
  follow = cur && matches(PHONE_MQ) ? { key: cur.cacheKey, until: Date.now() + FOLLOW_MS, height: panel.getBoundingClientRect().height } : null;
  if (focusAfterPress(byKeyboard, wasVisible) === 'title') ui.title?.focus({ preventScroll: true });
  else if (stubFocus) (stubFocus === 'ask' ? ui.ask : ui.chev)?.focus({ preventScroll: true });
}

/** The panel grew after a press (see `follow`): scroll it into view again, downwards only */
function followPress(): void {
  const f = follow;
  if (!f || !panel || !ui.head) return;
  if (!cur || cur.cacheKey !== f.key || collapsed || Date.now() > f.until) {
    follow = null;
    return;
  }
  const height = panel.getBoundingClientRect().height;
  if (Math.abs(height - f.height) < 1) return;
  f.height = height;
  const vh = window.innerHeight;
  const delta = scrollDelta({
    headTop: ui.head.getBoundingClientRect().top, panelBottom: panel.getBoundingClientRect().bottom, vh, topH: topInset(), phone: true,
    bottomH: bottomInset(vh),
  });
  if (delta > 0) scrollToY(window.scrollY + delta, true);
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
  if (!panel || !deps || !runner || !ui.head || !ui.title || !ui.state || !ui.dot || !ui.ask || !ui.askLbl || !ui.chev || !ui.inner || !ui.body) return;
  lastPaintAt = Date.now();
  const card = isCardKey(mountedKey);
  panel.classList.toggle('collapsed', collapsed);
  panel.classList.toggle('aip-card', card);
  ui.inner.hidden = collapsed;
  ui.chev.setAttribute('aria-expanded', String(!collapsed));
  const chevLabel = t(collapsed ? 'aiplus.panel.expand' : 'aiplus.panel.collapse');
  ui.chev.setAttribute('aria-label', chevLabel);
  if (!matches(TOUCH_MQ)) ui.chev.title = chevLabel;
  ui.askLbl.textContent = t('aiplus.analyze');
  ui.ask.hidden = !(card && cur);
  if (card && cur) {
    ui.ask.setAttribute('data-aip-pair', pairOf(cur));
    ui.ask.setAttribute('data-focus', cur.anchorKey);
  }

  const v = cur ? viewOf() : null;
  const title = cur ? t(popTitleKey(cur.kind, cur.source), { n: cur.no }) : t('aiplus.panel.idleTitle');
  // The visible title is the kind plus a number span, which hides on phones (the top row already shows the position)
  const noText = cur ? t('aiplus.pop.no', { n: cur.no }) : '';
  const cut = noText && title.endsWith(' · ' + noText) ? title.length - noText.length - 3 : -1;
  if (ui.title.getAttribute('data-full') !== title) {
    ui.title.setAttribute('data-full', title);
    ui.title.textContent = cut >= 0 ? title.slice(0, cut) : title;
    if (cut >= 0) ui.title.append(h('span', { class: 'aip-title-no' }, ' · ' + noText));
  }
  ui.title.classList.toggle('sr-only', card);
  ui.body.setAttribute('aria-label', title);
  ui.body.setAttribute('aria-busy', String(!!v && (v.phase === 'streaming' || v.phase === 'requesting')));
  ui.state.textContent = v ? stateText(v) : '';
  ui.dot.hidden = !(v && (v.phase === 'requesting' || v.phase === 'streaming'));

  paintNote(v);
  paintBody(v);
  paintActions(v);
  paintNoteActs(v);
  followPress();

  const say = v ? announce(lastPhase, v, deps) : null;
  lastPhase = v ? phaseKey(v) : null;
  if (say && sr) sr.textContent = say;
}

/** The one note of the note row: busy, other language, failed regenerate, error, or the cap callout (spec 2.7) */
function paintNote(v: EntryView | null): void {
  if (!ui.note || !ui.noteRow || !deps) return;
  let text = '';
  let cap = false;
  let alertish = false;
  if (v) {
    const cn = capNote({ block: callBlock(), expanded: !collapsed, view: v });
    if (cn.show) {
      cap = true;
      alertish = cn.alert;
      text = cn.source === 'refused' && v.note ? noteText(v.note, deps, sessionChat()) : capText();
    } else if (v.note) {
      text = noteText(v.note, deps, sessionChat());
      alertish = noteIsAlert(v.note);
    } else if (v.truncated) {
      text = truncatedText();
    }
  }
  const sig = `${cap ? 'cap' : 'note'}|${text}`;
  if (sig !== lastNoteSig) {
    lastNoteSig = sig;
    // An alert is announced once, when it appears; a callout that only stands there while blocked stays quiet
    const again = text !== '' && alertish ? alertGate.take(sig) : (alertGate.take(null), false);
    ui.note.className = cap ? 'aip-note callout warn' : 'aip-note';
    ui.note.replaceChildren(...(cap ? [miniIcon('clock', 'aip-note-ico'), h('span', null, text)] : [text]));
    if (again) ui.note.setAttribute('role', 'alert');
    else ui.note.removeAttribute('role');
  }
  ui.noteRow.hidden = text === '' && ui.noteActs?.childElementCount === 0;
}

/** The body: the idle text, the "not asked" text, the pending line, or the answer (re-parsed only when it changed) */
function paintBody(v: EntryView | null): void {
  const body = ui.body;
  if (!body || !deps) return;
  const set = (key: string, node: HTMLElement): void => {
    lastText = null;
    if (bodyKey === key) return;
    bodyKey = key;
    body.replaceChildren(node);
  };
  if (!cur || !v) {
    set('empty', h('p', { class: 'aip-panel-empty' }, t('aiplus.panel.empty')));
    body.classList.remove('aip-wait');
    body.removeAttribute('data-wait');
    return;
  }
  const busy = v.phase === 'loading' || v.phase === 'requesting' || v.phase === 'streaming';
  const notAsked = isNotAsked(v);
  if (notAsked) {
    set(`notAsked|${labelOf(cur)}`, h('p', { class: 'aip-panel-empty' }, t('aiplus.panel.notAsked', { label: labelOf(cur) })));
  } else if (v.text.trim() === '' && busy) {
    // Asked, nothing written yet (a reasoning model may think for a while): say so in the panel, not only in the head
    const line = stateText(v);
    set(`pending|${line}`, h('p', { class: 'aip-panel-empty aip-panel-pending' }, miniIcon('refresh', 'aip-ami'), line));
  } else if (v.text !== lastText || bodyKey !== 'text') {
    bodyKey = 'text';
    lastText = v.text;
    const atBottom = followsText(body.scrollHeight, body.scrollTop, body.clientHeight);
    fillMarkdown(body, v.text);
    // While it streams, follow the text unless the reader scrolled up
    if (v.phase === 'streaming' && atBottom) body.scrollTop = body.scrollHeight;
  }
  // Waiting for a new answer (Regenerate, or a reasoning model still thinking) while the old one is on screen: dim the old
  // text and say so at the top, so the press visibly does something
  const waiting = !notAsked && v.text.trim() !== '' && (v.phase === 'requesting' || (v.phase === 'streaming' && v.thinking === true));
  body.classList.toggle('aip-wait', waiting);
  if (waiting) body.setAttribute('data-wait', t('aiplus.panel.regenerating'));
  else body.removeAttribute('data-wait');
}

/** A footer slot: icon + label (`span.lbl`, hidden by the PHONE CSS) on one button; the label is always the accessible name */
function slotButton(o: { icon: MiniIcon; label: string; slot: string; disabled?: boolean; kids?: HTMLElement; onClick?: () => void; extra?: string }): HTMLButtonElement {
  const btn = h('button', {
    class: `ibtn ibtn-quiet ibtn-label aip-slot${o.extra ? ` ${o.extra}` : ''}`, type: 'button', 'data-slot': o.slot, 'aria-label': o.label, disabled: o.disabled,
    onclick: o.onClick,
  }, miniIcon(o.icon, 'ibtn-ic'), h('span', { class: 'lbl', 'aria-hidden': 'true' }, o.kids ?? o.label));
  tipFor(btn, o.label);
  return btn;
}

/** The tap-and-hold tip of an icon-only slot (touch screens and phone widths); a mouse on a wide screen reads the visible label */
function tipFor(btn: HTMLElement, text: string): void {
  if (matches(TOUCH_MQ) || matches(PHONE_MQ)) holdTip(btn, text);
}

/** Copy's feedback is on the button itself: icon → check, label / tip → 已复制 for 1.5 s (or xCircle on a failure) */
function paintCopy(): void {
  const b = ui.copy;
  if (!b) return;
  const label = copyStatus === 'ok' ? t('aiplus.panel.copied') : t('ai.copy');
  b.querySelector('svg')?.replaceWith(miniIcon(copyStatus === 'ok' ? 'check' : copyStatus === 'fail' ? 'xCircle' : 'copy', 'ibtn-ic'));
  const lbl = b.querySelector('.lbl');
  if (lbl) lbl.textContent = label;
  b.setAttribute('aria-label', label);
  tipFor(b, label);
}

async function doCopy(): Promise<void> {
  const ok = await copyText(viewOf().text);
  copyStatus = ok ? 'ok' : 'fail';
  if (copyTimer != null) window.clearTimeout(copyTimer);
  copyTimer = window.setTimeout(() => {
    copyTimer = null;
    copyStatus = 'idle';
    paintCopy();
  }, COPY_MS);
  paintCopy();
  if (sr) sr.textContent = t(ok ? 'ai.copied' : 'ai.copyFailed');
}

/** Regenerate (guarded: a press while the entry is requesting or streaming is ignored); the new answer streams from the top */
function runRegenerate(): void {
  const a = cur;
  if (!a || !runner || !regenerateAllowed(runner.entry(a.cacheKey))) return;
  if (ui.body) ui.body.scrollTop = 0;
  void runner.regenerate(a.cacheKey, a.spec);
}

/**
 * The footer, fixed slots (spec 2.5): Copy · Regenerate / 正在回答… · Stop, then the muted "saved in AI Chat" text at the right end (a tip names the chat; no navigation). All are always
 * there; what cannot be used now is disabled. Rebuilt only when what they show changes (a signature), never on a streaming repaint:
 * a rebuilt Stop would lose focus and a press straddling the repaint would lose its click. Handlers read the current state when clicked.
 */
function paintActions(v: EntryView | null): void {
  if (!ui.acts || !deps || !runner) return;
  const entry = cur ? runner.entry(cur.cacheKey) : undefined;
  const chatTitle = entry?.chatTitle ?? null;
  const block = callBlock();
  const slots = footerSlots({ idle: !cur, view: v, blocked: block != null, hasChat: savedInChat(entry), chatTitle });
  const sig = [
    deps.uiLang(), cur ? cur.cacheKey : '-', slots.copy.disabled, slots.regenerate.disabled, slots.regenerate.ariaDisabled, slots.regenerate.writing,
    slots.stop.disabled, slots.chat.visible, slots.chat.title ?? '', block ? `${block.cap}:${block.retryAt}` : '-',
  ].join('|');
  if (sig === actsSig) return;
  actsSig = sig;
  const r = runner;
  const btns: HTMLElement[] = [];

  ui.copy = slotButton({ icon: 'copy', label: t('ai.copy'), slot: 'copy', disabled: slots.copy.disabled, onClick: () => { void doCopy(); } });
  btns.push(ui.copy);

  // The Regenerate slot holds both labels in one grid cell, so it is as wide as the wider one; while writing it says 正在回答… and spins
  const writing = slots.regenerate.writing;
  const regenLabel = t(writing ? 'ai.writing' : 'aiplus.regenerate');
  const regen = slotButton({
    icon: 'refresh', label: regenLabel, slot: 'regenerate', disabled: slots.regenerate.disabled, extra: writing ? 'aip-busy' : '',
    kids: h('span', { class: 'aip-two' },
      h('span', { class: writing ? 'off' : 'on' }, t('aiplus.regenerate')),
      h('span', { class: writing ? 'on' : 'off' }, t('ai.writing'))),
    ...(slots.regenerate.ariaDisabled ? {} : { onClick: runRegenerate }),
  });
  if (slots.regenerate.ariaDisabled) {
    // The caps block model calls: aria-disabled (never `disabled`), the cap text as tip. A tap first re-checks the block (an
    // administrator may have raised the limits): still blocked → the cap text as a toast; free again → the regenerate runs.
    btns.push(withReason(regen, capText(), {
      beforeToast: () => {
        void recheckCallBlock().then((still) => {
          if (!cur) return;
          if (still) toast(capText());
          else runRegenerate();
        });
        return true;
      },
    }));
  } else {
    btns.push(regen);
  }

  btns.push(slotButton({
    icon: 'stop', label: t('ai.stop'), slot: 'stop', disabled: slots.stop.disabled,
    onClick: () => { const a = cur; if (a) void r.stop(a.cacheKey, a.spec); },
  }));
  btns.push(savedNote(slots.chat.visible, slots.chat.title, cur?.sessionPrefix.startsWith('exam|') ? 'exam' : 'practice'));
  replaceKeepingFocus(ui.acts, btns);
  paintCopy();
}

/**
 * The right end of the footer: a short muted text, not a button ("saved in AI Chat"). The answer is saved in a chat; the tip (hover,
 * keyboard focus, tap) names the chat as AI Chat lists it (the server's title plus the kind suffix). It never navigates (that would leave the practice / exam).
 * Without a chat it keeps its place, invisible, so nothing moves.
 */
function savedNote(visible: boolean, title: string | null, kind: 'practice' | 'exam'): HTMLElement {
  const id = 'aip-saved-tip';
  // No known title: no tip element at all (an empty .tip would still pop up as a small dark box) and nothing to focus for
  const tipText = visible && title ? t('aiplus.savedTip', { title: chatNameOf({ title, kind }, (k) => t(k)) }) : null;
  const txt = h('span', { class: 'aip-saved', 'data-slot': 'chat', tabindex: tipText ? '0' : null, 'aria-describedby': tipText ? id : null }, t('aiplus.saved'));
  const tip = tipText ? h('span', { class: 'tip', role: 'tooltip', id }, tipText) : null;
  return h('span', { class: `has-tip tip-tap aip-chat${visible ? '' : ' aip-gone'}` }, txt, tip);
}

/** The busy flow's two text buttons live in the note row (meaning unclear as icons): "stop it, write this one" and "show it" */
function paintNoteActs(v: EntryView | null): void {
  if (!ui.noteActs || !ui.noteRow || !runner || !deps) return;
  const r = runner;
  const running = v?.note?.kind === 'busy' ? v.note.running : null;
  const bv = running ? busyView(running, sessionChat()) : null;
  const other = running && bv && bv.kind === 'study' && bv.canShow && cur ? cur.resolveRunning?.(running) ?? null : null;
  const sig = [deps.uiLang(), cur ? cur.cacheKey : '-', running ? running.chatId : '-', other ? other.anchorKey : '-'].join('|');
  if (sig !== noteActsSig) {
    noteActsSig = sig;
    const btns: HTMLElement[] = [];
    if (running) {
      btns.push(h('button', {
        class: 'btn aip-note-btn', type: 'button',
        onclick: () => { const a = cur; if (a) void r.stopAndAsk(a.cacheKey, a.spec, running.chatId); },
      }, t('aiplus.busy.stopAndAsk')));
      if (other) btns.push(h('button', { class: 'btn quiet aip-note-btn', type: 'button', onclick: () => openAiPanel({ ...other, byKeyboard: false }) }, t('aiplus.busy.show')));
    }
    replaceKeepingFocus(ui.noteActs, btns);
  }
  ui.noteRow.hidden = (ui.note?.textContent ?? '') === '' && ui.noteActs.childElementCount === 0;
}
