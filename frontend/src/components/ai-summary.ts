// The AI summary overlay (v0.9, v1.0 P-30): the large window of a record's AI 总结, a modal <dialog> that covers most of the
// page below its title. Header: the title at the left, 重新生成 (or 停止) and × at the right (icon buttons; the label of the
// first one is CSS-hidden under PHONE); the body scrolls inside the window and the page behind does not move; the footer is one
// row (state text, Copy, the "saved in AI Chat" text with a tip naming the chat; never a link). Closed by ×, Esc and a click
// on the backdrop. Same engine as the AI+ panel (domain/ai-run.ts), its own cache. Opened from the records list, Home and
// the record detail page.
//   · installAiSummary(deps)   once, from main.ts: hashchange closes it, the sign-out hook forgets everything;
//   · openAiSummary(args)      from the records table buttons and Home's button;
//   · closeAiSummary(), dropAiSummary(recordId, at) (a record was deleted: its cache entry goes, an open window stays),
//     clearAiSummaryData().
import { t } from '../i18n/index.ts';
import { emptyEntry, entryView, isNothingToSummarise, summaryKey, type EntryView } from '../domain/ai-study.ts';
import { createRunner, type Runner } from '../domain/ai-run.ts';
import { atDigits, basedKey, chatNameOf, replyLang } from '../domain/ai-plus.ts';
import { savedInChat } from '../domain/ai-panel.ts';
import { summarySpec } from '../domain/ai-spec.ts';
import { backdropCloses, focusTitleOnOpen, overlayTop, pickFocusTarget } from '../domain/ai-overlay.ts';
import '../styles/ai-md.css';
import '../styles/ai-summary.css';
import { onClearUserData } from '../state/store.ts';
import { callBlock, noteCallBlock, onCallBlockChange, recheckCallBlock } from '../state/ai.ts';
import { copyText } from '../utils/clipboard.ts';
import { h } from '../utils/dom.ts';
import { fillMarkdown } from './ai-render.ts';
import { iconButton } from './icon-button.ts';
import { miniIcon, type MiniIcon } from './mini-icons.ts';
import { announce, focusOpener, kindWord, noteIsAlert, noteText, phaseKey, truncatedText, replaceKeepingFocus, safeInsets, stateText, type StudyUiDeps } from './ai-study-ui.ts';
import type { AiCallBlock } from '../types/api.ts';

export interface SummaryOpenArgs {
  /** The record: its id, its time as the list shows it (`YYYY-MM-DD HH:MM`) and its kind */
  rec: { id: number; at: string; kind: 'practice' | 'exam' };
  /** `data-focus` key of the opener (`rec-ai-<id>` / `home-ai`): focus returns to it when the window closes */
  anchorKey: string;
  byKeyboard: boolean;
}

const PAINT_MS = 100;
/** Copy feedback on the button (icon becomes a tick, tip reads "Copied") */
const COPY_MS = 1500;

let deps: StudyUiDeps | null = null;
let runner: Runner | null = null;
let cur: SummaryOpenArgs | null = null;
let curKey = '';
let dlg: HTMLDialogElement | null = null;
const ui: {
  title?: HTMLElement; meta?: HTMLElement; noteRow?: HTMLElement; note?: HTMLElement; noteActs?: HTMLElement; body?: HTMLElement;
  state?: HTMLElement; head?: HTMLElement; foot?: HTMLElement; sr?: HTMLElement;
} = {};
let paintTimer: number | null = null;
let lastPaintAt = 0;
let lastText: string | null = null;
let lastPhase: string | null = null;
let copyBtn: HTMLButtonElement | null = null;
let copyTimer: number | null = null;
let lastNoteSig = '';
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
    blocked: () => callBlock(),
    onCallLimit: (q) => noteCallBlock(q),
  });
  // The cap ended or changed (timer, status read): the note and the Regenerate slot follow without a reload
  onCallBlockChange(() => { if (cur) schedulePaint(); });
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
  lastNoteSig = '';
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
  if (copyTimer != null) window.clearTimeout(copyTimer);
  copyTimer = null;
  copyBtn = null;
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
  // The meta line is part of the header at every width; CSS puts it under the title, or (short landscape) between title and buttons
  ui.meta = h('p', { id: 'aip-sum-meta', class: 'aip-sum-meta', onclick: (e: Event) => (e.currentTarget as HTMLElement).classList.toggle('open') });
  ui.note = h('p', { id: 'aip-sum-note', class: 'aip-note' });
  ui.noteActs = h('span', { class: 'aip-sum-noteacts' });
  ui.noteRow = h('div', { class: 'aip-sum-noterow', hidden: true }, ui.note, ui.noteActs);
  ui.body = h('div', { id: 'aip-sum-body', class: 'aip-sum-body ai-md', tabindex: '0', role: 'region', 'aria-label': t('aiplus.sum.title') });
  ui.state = h('span', { class: 'aip-state' });
  ui.foot = h('div', { class: 'aip-sum-foot' });
  ui.sr = h('div', { id: 'aip-sum-sr', class: 'sr-only', role: 'status', 'aria-live': 'polite' });
  const x = iconButton({ icon: 'x', label: t('aiplus.close'), onClick: () => closeAiSummary(true) });
  x.classList.add('aip-sum-x');
  const d = h('dialog', { id: 'aip-sum', class: 'aip-sum', 'aria-labelledby': 'aip-sum-h', 'aria-describedby': 'aip-sum-meta' },
    h('div', { class: 'aip-sum-in' },
      h('header', { class: 'aip-sum-head' }, ui.title, ui.meta, h('div', { class: 'aip-sum-right' }, ui.head, x)),
      ui.noteRow, ui.body,
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
  // The window starts below the page's title and covers the rest (phones: the CSS sets its own top, see ai-summary.css)
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

/** An icon + label button of the header: the label is a span that CSS hides under PHONE (the accessible name stays) */
function headAction(icon: MiniIcon, label: string, onClick: () => void, describedBy?: string): HTMLButtonElement {
  const b = iconButton({ icon, label, onClick, ...(describedBy ? { describedBy } : {}) });
  b.classList.add('aip-sum-act');
  b.append(h('span', { class: 'lbl', 'aria-hidden': 'true' }, label));
  return b;
}

/**
 * The footer's right end: a short muted text, not a button (same keys and look as the AI+ panel's). The tip (hover, focus, tap)
 * names the chat as AI Chat lists it: the server's title (the time) plus the localised summary suffix ("<time>-AI总结"); nothing
 * navigates. Without a chat it keeps its place, invisible.
 */
function savedNote(visible: boolean, title: string | null): HTMLElement {
  const id = 'aip-sum-saved-tip';
  const name = title ? chatNameOf({ title, kind: 'summary' }, (k) => t(k)) : null;
  const tipText = visible && name ? t('aiplus.savedTip', { title: name }) : null;
  const txt = h('span', { class: 'aip-sum-saved', tabindex: tipText ? '0' : null, 'aria-describedby': tipText ? id : null }, t('aiplus.saved'));
  const tip = tipText ? h('span', { class: 'tip', role: 'tooltip', id }, tipText) : null;
  return h('span', { class: `has-tip tip-tap aip-sum-chat${visible ? '' : ' aip-gone'}` }, txt, tip);
}

/** Copy feedback on the button itself: the icon turns into a tick and the name reads "Copied" for a moment (no toast under a modal) */
function paintCopyButton(state: 'idle' | 'ok' | 'fail'): void {
  const b = copyBtn;
  if (!b) return;
  const icon: MiniIcon = state === 'ok' ? 'check' : state === 'fail' ? 'xCircle' : 'copy';
  const label = state === 'ok' ? t('aiplus.panel.copied') : state === 'fail' ? t('ai.copyFailed') : t('ai.copy');
  const old = b.querySelector('svg');
  old?.replaceWith(miniIcon(icon, old.getAttribute('class') ?? undefined));
  b.setAttribute('aria-label', label);
  if (b.hasAttribute('title')) b.setAttribute('title', label);
  b.dataset.state = state;
}

const isCapNote = (n: EntryView['note']): boolean => n?.kind === 'error' && n.code === 'ai.callLimit';

function paint(): void {
  if (!cur || !dlg || !deps || !ui.body || !ui.note || !ui.noteRow || !ui.noteActs || !ui.meta || !ui.state || !ui.head || !ui.foot) return;
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
  const regen = v.canRegenerate && !refusal;
  const block: AiCallBlock | null = callBlock();

  // The note: a refused or failed request, else (while the cap blocks calls and Regenerate would be offered) the cap callout
  let note = v.note && !refusal ? noteText(v.note, deps, null) : v.truncated && !v.note ? truncatedText() : '';
  let cap = !!v.note && !refusal && isCapNote(v.note);
  if (block && regen && (note === '' || v.note?.kind === 'otherLang')) {
    note = deps.errorText({ status: 429, code: 'ai.callLimit', quota: block });
    cap = true;
  }
  const noteSig = `${cap ? 'cap' : 'note'}|${note}`;
  if (noteSig !== lastNoteSig) {
    // An alert is announced once, when it appears (a cap callout that only stands there while blocked is quiet)
    const alertNow = noteIsAlert(v.note) && !refusal && note !== '';
    lastNoteSig = noteSig;
    ui.note.className = cap ? 'aip-note callout warn' : 'aip-note';
    ui.note.replaceChildren(...(cap ? [miniIcon('clock', 'aip-note-ico'), h('span', null, note)] : [note]));
    if (alertNow) ui.note.setAttribute('role', 'alert');
    else ui.note.removeAttribute('role');
  }
  ui.noteRow.hidden = note === '';

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

  ui.state.textContent = stateText(v);

  // Header and footer buttons: rebuilt only when their signature changes (never on a streaming repaint, which would drop the
  // focus of Stop / Copy and lose a click); handlers read the current state when clicked
  const r = runner;
  const key = curKey;
  const chatId = entry?.chatId ?? null;
  const running = v.note?.kind === 'busy' && v.note.running ? v.note.running : null;
  const capped = regen && block != null;
  const sig = [key, v.text.trim() === '', v.canStop, regen, capped, running ? running.chatId : '-', chatId ?? '-', savedInChat(entry), entry?.chatTitle ?? ''].join('|');
  if (sig !== actsSig) {
    actsSig = sig;
    // Was the focus on a header / footer / note button? The boxes are rebuilt below; when none keeps a usable button for it, the
    // focus goes to the header's Stop / Regenerate (or the title), never to <body> behind the modal
    const act = document.activeElement;
    const hadFocus = !!act && (ui.head.contains(act) || ui.foot.contains(act) || ui.noteActs.contains(act));
    const now = (): SummaryOpenArgs | null => cur;
    const head: HTMLElement[] = [];
    if (v.canStop && r) {
      head.push(headAction('stop', t('ai.stop'), () => { const a = now(); if (a) void r.stop(key, specOf(a)); }));
    } else if (regen && r) {
      // While the cap blocks calls: aria-disabled (never `disabled`), described by the visible cap callout. A tap re-checks the
      // block first (an administrator may have raised the limits): still blocked, the callout is the answer (a toast would sit
      // behind the modal) and the live region says it; free again, the action runs once.
      const go = (): void => { const a = now(); if (a) void r.regenerate(key, specOf(a)); };
      const b = headAction('refresh', t('aiplus.regenerate'), () => {
        if (!callBlock()) { go(); return; }
        void recheckCallBlock().then((still) => {
          if (!cur || key !== curKey) return;
          if (!still) { go(); return; }
          const q = callBlock();
          if (q && ui.sr && deps) ui.sr.textContent = deps.errorText({ status: 429, code: 'ai.callLimit', quota: q });
        });
      }, capped ? 'aip-sum-note' : undefined);
      if (capped) b.setAttribute('aria-disabled', 'true');
      head.push(b);
    }
    replaceKeepingFocus(ui.head, head);

    // Footer, one row: Copy, then the muted "saved in AI Chat" text (kept in place, invisible, until a chat exists)
    const foot: HTMLElement[] = [];
    const copy = iconButton({
      icon: 'copy', label: t('ai.copy'),
      onClick: async () => {
        const ok = await copyText(viewOf().text);
        // Under a modal dialog a toast is neither seen nor announced: the button and the live region carry it
        const msg = t(ok ? 'ai.copied' : 'ai.copyFailed');
        if (copyTimer != null) window.clearTimeout(copyTimer);
        copyTimer = window.setTimeout(() => { copyTimer = null; paintCopyButton('idle'); }, COPY_MS);
        paintCopyButton(ok ? 'ok' : 'fail');
        if (ui.sr) ui.sr.textContent = msg;
      },
    });
    copy.disabled = v.text.trim() === '';
    copyBtn = copy;
    foot.push(copy);
    foot.push(savedNote(savedInChat(entry), entry?.chatTitle ?? null));
    replaceKeepingFocus(ui.foot, foot);

    // The busy flow's button is text (the meaning is unclear as an icon), in the note row
    const acts: HTMLElement[] = [];
    if (running && r) {
      acts.push(h('button', { class: 'btn aip-sum-busy', type: 'button', onclick: () => { const a = now(); if (a) void r.stopAndAsk(key, specOf(a), running.chatId); } }, t('aiplus.busy.stopAndAsk')));
    }
    replaceKeepingFocus(ui.noteActs, acts);
    if (hadFocus && !dlg.contains(document.activeElement)) {
      const first = ui.head.querySelector<HTMLElement>('button:not(:disabled)');
      pickFocusTarget(first, ui.title)?.focus();
    }
  }

  const say = announce(lastPhase, v, deps);
  lastPhase = phaseKey(v);
  if (say && ui.sr) ui.sr.textContent = say;
}
