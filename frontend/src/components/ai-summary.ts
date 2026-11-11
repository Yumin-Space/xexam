// The AI summary overlay (v0.9): the large window of a record's AI 总结, a modal <dialog> that covers most of the page below
// its title. Header: the title at the left, 重新生成 (or 停止) and × at the right; the body scrolls inside the window and the
// page behind does not move. Closed by ×, Esc and a click on the backdrop. Same engine as the popover (domain/ai-run.ts), its
// own cache. Spec 7.10.
//   · installAiSummary(deps)   once, from main.ts: hashchange closes it, the sign-out hook forgets everything;
//   · openAiSummary(args)      from the records table buttons and Home's button;
//   · closeAiSummary(), dropAiSummary(recordId, at) (a record was deleted: its cache entry goes, an open window stays),
//     clearAiSummaryData().
import { t } from '../i18n/index.ts';
import { emptyEntry, entryView, isNothingToSummarise, summaryKey, type EntryView } from '../domain/ai-study.ts';
import { createRunner, type Runner } from '../domain/ai-run.ts';
import { atDigits, basedKey, replyLang } from '../domain/ai-plus.ts';
import { summarySpec } from '../domain/ai-spec.ts';
import { backdropCloses, focusTitleOnOpen, overlayTop, pickFocusTarget } from '../domain/ai-popover.ts';
import '../styles/ai-md.css';
import '../styles/ai-pop.css';
import { onClearUserData } from '../state/store.ts';
import { copyText } from '../utils/clipboard.ts';
import { h } from '../utils/dom.ts';
import { fillMarkdown } from './ai-render.ts';
import { announce, focusOpener, kindWord, noteIsAlert, noteText, phaseKey, replaceKeepingFocus, safeInsets, stateText, type StudyUiDeps } from './ai-study-ui.ts';

export interface SummaryOpenArgs {
  /** The record: its id, its time as the list shows it (`YYYY-MM-DD HH:MM`) and its kind */
  rec: { id: number; at: string; kind: 'practice' | 'exam' };
  /** `data-focus` key of the opener (`rec-ai-<id>` / `home-ai`): focus returns to it when the window closes */
  anchorKey: string;
  byKeyboard: boolean;
}

const PAINT_MS = 100;
const COPY_MS = 2500;

let deps: StudyUiDeps | null = null;
let runner: Runner | null = null;
let cur: SummaryOpenArgs | null = null;
let curKey = '';
let dlg: HTMLDialogElement | null = null;
const ui: { title?: HTMLElement; meta?: HTMLElement; note?: HTMLElement; body?: HTMLElement; state?: HTMLElement; head?: HTMLElement; foot?: HTMLElement; sr?: HTMLElement } = {};
let paintTimer: number | null = null;
let lastPaintAt = 0;
let lastText: string | null = null;
let lastPhase: string | null = null;
let copyState: { text: string; timer: number } | null = null;
let downOnDialog = false;
let actsSig = '';

const specOf = (a: SummaryOpenArgs) => summarySpec(deps!.transport, a.rec, () => deps!.uiLang());
const viewOf = (): EntryView => entryView((runner && runner.entry(curKey)) || emptyEntry(), replyLang(deps ? deps.uiLang() : 'en'));

export function installAiSummary(d: StudyUiDeps): void {
  deps = d;
  runner = createRunner({
    setTimer: (fn, ms) => window.setTimeout(fn, ms),
    clearTimer: (hd) => window.clearTimeout(hd as number),
    onChange: (k) => { if (cur && k === curKey) schedulePaint(); },
    onRefuse: () => d.refreshAi(true),
  });
  window.addEventListener('hashchange', () => closeAiSummary(false));
  onClearUserData(clearAiSummaryData);
}

export function openAiSummary(args: SummaryOpenArgs): void {
  if (!deps || !runner) return;
  closeAiSummary(false);
  cur = args;
  curKey = summaryKey(args.rec.id, atDigits(args.rec.at));
  lastText = null;
  lastPhase = null;
  build();
  deps.refreshAi(false);
  void runner.open(curKey, specOf(args));
  paint();
  if (focusTitleOnOpen(args.byKeyboard ? 0 : 1)) ui.title?.focus();
}

/** Closes the window. The generation goes on in the background and is saved (the chat keeps it) */
export function closeAiSummary(returnFocus = true): void {
  const was = cur;
  cur = null;
  if (paintTimer != null) window.clearTimeout(paintTimer);
  paintTimer = null;
  if (copyState) window.clearTimeout(copyState.timer);
  copyState = null;
  const d = dlg;
  dlg = null;
  if (d) {
    // `close` is only a cleanup here: remove the lock and the element
    d.close();
    d.remove();
  }
  document.documentElement.classList.remove('aip-lock');
  if (was && returnFocus) focusOpener(was.anchorKey);
}

/** A record was deleted: its cached summary goes; an open window stays open (the chat is kept) */
export function dropAiSummary(recordId: number, at: string): void {
  runner?.drop(summaryKey(recordId, atDigits(at)));
}

/** A chat was deleted (`chatId`), or all of them (null): their saved summaries are forgotten */
export function forgetStudyChats(chatId: number | null): void {
  runner?.dropWhere((e) => chatId == null || e.chatId === chatId);
}

export function clearAiSummaryData(): void {
  closeAiSummary(false);
  runner?.clearAll();
}

// ---------------------------------------------------------------- building

function build(): void {
  if (!cur || !deps) return;
  actsSig = '';
  ui.title = h('h2', { id: 'aip-sum-h', tabindex: '-1' }, t('aiplus.sum.title'));
  ui.head = h('div', { class: 'aip-sum-acts' });
  ui.meta = h('p', { id: 'aip-sum-meta', class: 'aip-sum-meta' });
  ui.note = h('p', { id: 'aip-sum-note', class: 'aip-note', hidden: true });
  ui.body = h('div', { id: 'aip-sum-body', class: 'aip-sum-body ai-md', tabindex: '0', role: 'region', 'aria-label': t('aiplus.sum.title') });
  ui.state = h('span', { class: 'aip-state' });
  ui.foot = h('div', { class: 'aip-sum-foot' });
  ui.sr = h('div', { id: 'aip-sum-sr', class: 'sr-only', role: 'status', 'aria-live': 'polite' });
  const x = h('button', { class: 'aip-x', type: 'button', 'aria-label': t('aiplus.close'), title: t('aiplus.close'), onclick: () => closeAiSummary(true) }, '×');
  const d = h('dialog', { id: 'aip-sum', class: 'aip-sum', 'aria-labelledby': 'aip-sum-h', 'aria-describedby': 'aip-sum-meta' },
    h('div', { class: 'aip-sum-in' },
      h('header', { class: 'aip-sum-head' }, ui.title, h('div', { class: 'aip-sum-right' }, ui.head, x)),
      ui.meta, ui.note, ui.body,
      h('footer', { class: 'aip-sum-bar' }, ui.state, ui.foot),
      ui.sr));
  // Esc: the native cancel closes the dialog; the cleanup (lock, focus) runs through closeAiSummary
  d.addEventListener('cancel', (e) => {
    e.preventDefault();
    closeAiSummary(true);
  });
  // A click outside closes it only when the pointer went down outside too (a selection dragged out of the text does not)
  d.addEventListener('pointerdown', (e) => { downOnDialog = e.target === d; });
  d.addEventListener('click', (e) => {
    if (backdropCloses(downOnDialog, e.target === d)) closeAiSummary(true);
    downOnDialog = false;
  });
  // Closed from outside (closeDialogs() on an auth problem): the same cleanup
  d.addEventListener('close', () => {
    if (dlg === d) closeAiSummary(false);
  });
  document.body.append(d);
  dlg = d;
  // The window starts below the page's title and covers the rest
  const h1 = document.querySelector('#app h1');
  const bottom = h1 ? h1.getBoundingClientRect().bottom : null;
  const top = overlayTop(bottom != null && bottom > 0 ? bottom : null, window.innerHeight, safeInsets().top);
  d.style.setProperty('--aip-top', `${Math.round(top)}px`);
  document.documentElement.classList.add('aip-lock');
  d.showModal();
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
  if (!cur || !dlg || !deps || !ui.body || !ui.note || !ui.meta || !ui.state || !ui.head || !ui.foot) return;
  lastPaintAt = Date.now();
  const v = viewOf();
  const entry = runner?.entry(curKey);
  ui.body.setAttribute('aria-busy', String(v.phase === 'streaming' || v.phase === 'requesting'));

  // Meta: "<time> · <kind> · Based on …" (the sample is known once an answer exists)
  const parts = [cur.rec.at, kindWord(cur.rec.kind)];
  if (v.sample) parts.push(t(basedKey(v.sample), { used: v.sample.used, usable: v.sample.usable, wrong: v.sample.wrong }));
  ui.meta.textContent = parts.join(' · ');

  // Nothing to summarise is told in the body, everything else in the note
  const refusal = v.note?.kind === 'error' && isNothingToSummarise(v.note.code);
  const note = v.note && !refusal ? noteText(v.note, deps, null) : '';
  ui.note.hidden = note === '';
  ui.note.textContent = note;
  if (noteIsAlert(v.note) && !refusal) ui.note.setAttribute('role', 'alert');
  else ui.note.removeAttribute('role');

  const text = refusal && v.note?.kind === 'error' ? noteText(v.note, deps, null) : v.text;
  if (text !== lastText) {
    lastText = text;
    const scroller = ui.body;
    const atBottom = scroller.scrollHeight - scroller.scrollTop - scroller.clientHeight < 24;
    fillMarkdown(scroller, text);
    if (v.phase === 'streaming' && atBottom) scroller.scrollTop = scroller.scrollHeight;
  }
  if (refusal) ui.body.setAttribute('role', 'alert');
  else ui.body.setAttribute('role', 'region');

  ui.state.textContent = copyState ? copyState.text : stateText(v);

  // Header and footer buttons: rebuilt only when their signature changes (never on a streaming repaint, which would drop the
  // focus of Stop / Copy and lose a click); handlers read the current state when clicked
  const r = runner;
  const key = curKey;
  const chatId = entry?.chatId ?? null;
  const running = v.note?.kind === 'busy' && v.note.running ? v.note.running : null;
  const regen = v.canRegenerate && !refusal;
  const sig = [key, v.text.trim() === '', v.canStop, regen, running ? running.chatId : '-', chatId ?? '-'].join('|');
  if (sig !== actsSig) {
    actsSig = sig;
    // Was the focus on a header / footer button? Both boxes are rebuilt below; when neither keeps a usable button for it, the
    // focus goes to the header's Stop / Regenerate (or the title), never to <body> behind the modal
    const act = document.activeElement;
    const hadFocus = !!act && (ui.head.contains(act) || ui.foot.contains(act));
    const now = (): SummaryOpenArgs | null => cur;
    const head: HTMLElement[] = [];
    if (v.canStop && r) head.push(h('button', { class: 'btn quiet aip-mini', type: 'button', onclick: () => { const a = now(); if (a) void r.stop(key, specOf(a)); } }, t('ai.stop')));
    else if (regen && r) head.push(h('button', { class: 'btn quiet aip-mini', type: 'button', onclick: () => { const a = now(); if (a) void r.regenerate(key, specOf(a)); } }, t('aiplus.regenerate')));
    replaceKeepingFocus(ui.head, head);

    const foot: HTMLElement[] = [];
    foot.push(h('button', {
      class: 'btn quiet aip-mini', type: 'button', disabled: v.text.trim() === '',
      onclick: async () => {
        const ok = await copyText(viewOf().text);
        // Under a modal dialog a toast is neither seen nor announced: the state line and the live region carry it
        const msg = t(ok ? 'ai.copied' : 'ai.copyFailed');
        if (copyState) window.clearTimeout(copyState.timer);
        copyState = { text: msg, timer: window.setTimeout(() => { copyState = null; paint(); }, COPY_MS) };
        if (ui.sr) ui.sr.textContent = msg;
        paint();
      },
    }, t('ai.copy')));
    if (running && r) {
      foot.push(h('button', { class: 'btn aip-mini', type: 'button', onclick: () => { const a = now(); if (a) void r.stopAndAsk(key, specOf(a), running.chatId); } }, t('aiplus.busy.stopAndAsk')));
    }
    if (chatId != null) {
      foot.push(h('a', {
        class: 'aip-chatlink', href: '#/ai',
        onclick: (e: MouseEvent) => {
          e.preventDefault();
          closeAiSummary(false);
          deps?.openChat(chatId);
        },
      }, t('aiplus.openChat')));
    }
    replaceKeepingFocus(ui.foot, foot);
    if (hadFocus && !dlg.contains(document.activeElement)) {
      const first = ui.head.querySelector<HTMLElement>('button:not(:disabled)');
      pickFocusTarget(first, ui.title)?.focus();
    }
  }

  const say = announce(lastPhase, v, deps);
  lastPhase = phaseKey(v);
  if (say && ui.sr) ui.sr.textContent = say;
}
