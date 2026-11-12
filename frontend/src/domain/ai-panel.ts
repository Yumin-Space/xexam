// The pure rules of the AI+ panel (v1.0, spec section 2): what a press, a chevron tap or a question change does, how far to
// scroll, how tall the body is, which footer slots are usable and when the cap callout shows. No DOM and no i18n (it runs
// under `node --test`): components/ai-panel.ts reads rectangles, the media query results and the entry view from the page
// and passes plain values in; it paints what these functions decide.
import { CALL_LIMIT, type CallBlock, type Entry, type EntryView } from './ai-study.ts';

// ---------------------------------------------------------------- identity

/** The (question, kind, place) an answer belongs to: the value of `data-aip-pair` and the shown pair of the panel */
export const pairOf = (a: { qid: string; kind: string; source: string }): string => `${a.qid}|${a.kind}|${a.source}`;

/** What the panel shows now, as far as the plans need to know: null `pair` = idle (nothing chosen yet) */
export interface PanelState {
  pair: string | null;
  /** `data-aip-dock` value the panel sits in ('practice' | 'rv-<n>'), null while idle on a page without a dock */
  dock: string | null;
  collapsed: boolean;
}

/** The answer a button or a card refers to */
export interface PanelTarget {
  pair: string;
  dock: string;
}

const busyPhase = (v: Pick<EntryView, 'phase'>): boolean => v.phase === 'loading' || v.phase === 'requesting' || v.phase === 'streaming';

/**
 * "Not asked yet" (2.3): nothing in memory, nothing saved, no note and not busy. An entry that a question change peeked and
 * found empty counts too. `null` = no entry at all. The body then says how to ask (text only); the press that finds this
 * asks once.
 */
export const isNotAsked = (v: Pick<EntryView, 'text' | 'phase' | 'note'> | null): boolean =>
  v == null || (v.text.trim() === '' && !busyPhase(v) && v.note == null);

// ---------------------------------------------------------------- the press (AI 提示 / AI 解释 / a card's AI 解析)

/**
 * `open` = `runner.open` (memory, else the saved answer, else one new ask); `ask` = `runner.regenerate` (the entry is
 * known to be empty: ask once); `none` = never a request.
 */
export type RunAction = 'open' | 'ask' | 'none';

export interface PressPlan {
  /** The panel takes this answer (another kind, question or card), `cur` is replaced */
  switchTo: boolean;
  /** The panel is expanded after the press */
  expand: boolean;
  action: RunAction;
  /** Scroll the panel into view afterwards (2.6): always after a press */
  scroll: boolean;
}

/**
 * What pressing an opener does (2.3, 2.4). Another answer (kind, question or card) → switch, expand, show it (`open`).
 * The answer the panel already shows (the pressed one) → it asks only when the entry is empty (`notAsked`), exactly like the
 * first press; otherwise it never asks again and never collapses: a collapsed panel expands, then it scrolls into view.
 * A filled entry shown again after a chevron expand is read through `open` (memory first; a GET at most), never an ask.
 */
export function pressPlan(state: PanelState, req: PanelTarget, notAsked: boolean): PressPlan {
  const same = state.pair != null && state.pair === req.pair && state.dock === req.dock;
  if (!same) return { switchTo: true, expand: true, action: 'open', scroll: true };
  if (notAsked) return { switchTo: false, expand: true, action: 'ask', scroll: true };
  // The panel already shows it and it is expanded: scroll only, no request at all
  return { switchTo: false, expand: true, action: state.collapsed ? 'open' : 'none', scroll: true };
}

// ---------------------------------------------------------------- the chevron / the head tap

export interface ChevronPlan {
  switchTo: boolean;
  collapsed: boolean;
  /** `peek`: show memory or the saved answer, never ask; `none`: nothing to load */
  action: 'peek' | 'none';
  /** Never after a chevron tap (2.6) */
  scroll: false;
}

/**
 * The chevron or a tap on the head (2.3, 2.4): never a model request, never a scroll. `req` = the card the tap happened in
 * when it is not the live panel's own head (the collapsed stub of another card): the panel moves there, expanded, and peeks.
 * No `req` (the live panel's chevron): it toggles; expanding with an answer chosen peeks, collapsing leaves a running answer
 * going (nothing is stopped here).
 */
export function chevronPlan(state: PanelState, req: PanelTarget | null): ChevronPlan {
  if (req && !(state.pair === req.pair && state.dock === req.dock)) {
    return { switchTo: true, collapsed: false, action: 'peek', scroll: false };
  }
  const collapsed = !state.collapsed;
  return { switchTo: false, collapsed, action: !collapsed && state.pair != null ? 'peek' : 'none', scroll: false };
}

// ---------------------------------------------------------------- question change and re-anchoring

export interface QuestionChangePlan {
  /** `keep`: nothing changes; `follow`: the panel takes the new question with the same kind; `forget`: back to idle / stubs */
  change: 'keep' | 'follow' | 'forget';
  /** `follow` loads with `runner.peek` (memory, else GET saved): never an automatic model request */
  peek: boolean;
}

/**
 * Practice page after Previous / Next / a tile / the arrow keys (2.3): the panel stays as it is and follows the same kind on
 * the new question. `next` = the opener found by `data-focus` after the redraw (null = it is gone → forget). A different kind
 * cannot happen by construction; if it does the panel forgets rather than ask for something else.
 */
export function questionChangePlan(cur: { pair: string; kind: string } | null, next: { pair: string; kind: string } | null): QuestionChangePlan {
  if (!cur) return { change: 'keep', peek: false };
  if (!next || next.kind !== cur.kind) return { change: 'forget', peek: false };
  if (next.pair === cur.pair) return { change: 'keep', peek: false };
  return { change: 'follow', peek: true };
}

/**
 * Result / record-detail cards after a redraw (2.12 b): the dock the panel sits in must still exist and its answer must be
 * the shown pair, else the panel forgets what it showed and the cards go back to their stubs.
 */
export function cardAnchorPlan(shownPair: string, dockPair: string | null): 'keep' | 'forget' {
  return dockPair != null && dockPair === shownPair ? 'keep' : 'forget';
}

// ---------------------------------------------------------------- scrolling

/** The two media queries of 2.6 as plain numbers (mirrors `PHONE_MQ` of utils/media.ts) */
export const isPhone = (width: number, height: number): boolean => width <= 600 || (width > height && height <= 500);
/** `SHORT_LANDSCAPE_MQ`: landscape and at most 500 px tall */
export const isShortLandscape = (width: number, height: number): boolean => width > height && height <= 500;

export interface ScrollInput {
  /** The panel head's top and the panel's bottom edge, relative to the window's top */
  headTop: number;
  panelBottom: number;
  /** Window height */
  vh: number;
  /** `--top-h` of `:root` (what is pinned at the top; 0 when nothing is) */
  topH: number;
  phone: boolean;
  /** What is pinned at the bottom and covers the page (the action bar under BAR_MQ); default 0 */
  bottomH?: number;
}

/** The phone rule's gap between the free area's top and the panel head */
export const PHONE_GAP = 8;

/**
 * How many px to scroll the page (positive = down) after a press (2.6); 0 = do not scroll.
 *  · PHONE: the head's top goes 8 px under the top of the free area (`--top-h`); if the head is already between that line
 *    and 25 % of the window height, no scroll.
 *  · wider: the minimum that makes the head and as much of the body as fits visible (`block: 'nearest'`): nothing when it is
 *    already fully visible; a panel taller than the free area aligns its head with the top of the free area.
 */
export function scrollDelta(i: ScrollInput): number {
  const line = i.topH + PHONE_GAP;
  if (i.phone) {
    const lowest = Math.max(line, i.vh * 0.25);
    if (i.headTop >= line && i.headTop <= lowest) return 0;
    return Math.round(i.headTop - line);
  }
  const top = i.topH;
  const bottom = i.vh - (i.bottomH ?? 0);
  const height = i.panelBottom - i.headTop;
  if (height > bottom - top) return Math.round(i.headTop - top);
  if (i.headTop < top) return Math.round(i.headTop - top);
  if (i.panelBottom > bottom) return Math.round(i.panelBottom - bottom);
  return 0;
}

/**
 * Focus after a press (2.6, 2.8): opened with the keyboard → the panel title; with the pointer → the title only when the
 * head was off screen before the scroll (so a screen reader lands on the answer), else the pressed button keeps focus.
 */
export function focusAfterPress(byKeyboard: boolean, headWasVisible: boolean): 'title' | 'keep' {
  return byKeyboard || !headWasVisible ? 'title' : 'keep';
}

/** The head is "visible" for the focus rule when its top is inside the free area and above the window's bottom */
export const headVisible = (headTop: number, vh: number, topH: number): boolean => headTop >= topH && headTop < vh;

// ---------------------------------------------------------------- body height

export type BodyMode = 'fixed' | 'auto' | 'short';

export interface BodySize {
  mode: BodyMode;
  /** `fixed`: the height; the others: undefined (auto) */
  height?: number;
  minHeight?: number;
  maxHeight?: number;
}

const clampN = (v: number, lo: number, hi: number): number => Math.min(hi, Math.max(lo, v));

/**
 * The body's box by window size (2.6; CSS is the source, this mirrors it for the tests): not phone → fixed
 * `clamp(180px, 32vh, 340px)`; phone portrait (≤ 600 px wide) → auto, 72 px at least, at most `clamp(160px, 38vh, 300px)`;
 * short landscape → auto, at most `max(120px, 100dvh - 220px)` (top row, panel head and footer, the bar and a margin). Short landscape wins when both match.
 */
export function bodySize(width: number, height: number): BodySize {
  if (isShortLandscape(width, height)) return { mode: 'short', maxHeight: Math.max(120, height - 220) };
  if (width <= 600) return { mode: 'auto', minHeight: 72, maxHeight: clampN(height * 0.38, 160, 300) };
  return { mode: 'fixed', height: clampN(height * 0.32, 180, 340) };
}

/** PHONE: a swipe that reaches the end of the panel text moves the page on (`overscroll-behavior-y: auto`); wider: `contain` */
export const bodyOverscroll = (phone: boolean): 'auto' | 'contain' => (phone ? 'auto' : 'contain');

/** While streaming the body follows the text unless the reader scrolled up (24 px from the bottom, as in the demo) */
export const FOLLOW_PX = 24;
export const followsText = (scrollHeight: number, scrollTop: number, clientHeight: number): boolean =>
  scrollHeight - scrollTop - clientHeight < FOLLOW_PX;

// ---------------------------------------------------------------- footer slots (2.5)

export interface FooterInput {
  /** Nothing chosen yet (idle): every slot is disabled, no request is possible */
  idle: boolean;
  view: Pick<EntryView, 'text' | 'phase' | 'note' | 'canRegenerate' | 'canStop'> | null;
  /** The per-user call caps block model calls (`callBlock()` is set) */
  blocked: boolean;
  /** The entry has a chat id (the answer is saved in a chat) */
  hasChat: boolean;
  /** The chat's real title from the server; null = not known */
  chatTitle?: string | null;
}

export interface FooterSlots {
  copy: { disabled: boolean };
  /** `disabled`: no use at all; `ariaDisabled`: only the cap blocks it (a tap re-checks the block first); `writing`: the slot says 正在回答… and spins */
  regenerate: { disabled: boolean; ariaDisabled: boolean; writing: boolean };
  stop: { disabled: boolean };
  /** The muted "saved in AI Chat" text (not a button): invisible but in place (`visibility: hidden`) until a chat exists; `title` feeds its tip */
  chat: { visible: boolean; title: string | null };
}

/**
 * The fixed footer slots (2.5), always all there; what cannot be used now is disabled. Copy needs text; Regenerate needs
 * `canRegenerate`, no answer being written and no cap block (a block alone makes it `aria-disabled`, not `disabled`); Stop
 * needs an answer being written; the "saved in AI Chat" text needs a chat.
 */
export function footerSlots(i: FooterInput): FooterSlots {
  const v = i.view;
  if (i.idle || !v) {
    return { copy: { disabled: true }, regenerate: { disabled: true, ariaDisabled: false, writing: false }, stop: { disabled: true }, chat: { visible: false, title: null } };
  }
  const writing = v.canStop;
  const usable = v.canRegenerate && !writing && v.phase !== 'empty';
  return {
    copy: { disabled: v.text.trim() === '' },
    regenerate: { disabled: !usable, ariaDisabled: usable && i.blocked, writing },
    stop: { disabled: !writing },
    chat: { visible: i.hasChat, title: i.hasChat ? i.chatTitle || null : null },
  };
}

/**
 * This entry's answer is saved in a chat (feeds `FooterInput.hasChat`): a chat id AND an answer of this entry (one being
 * written counts, the server saves it from its start). A chat id alone is not enough: the saved route returns the session's
 * chat also for a question not asked yet (that chat holds only other answers).
 */
export const savedInChat = (e: Pick<Entry, 'chatId' | 'attempt' | 'shown'> | undefined): boolean =>
  e != null && e.chatId != null && (e.attempt != null || e.shown != null);

/** The Regenerate guard (2.5): a press while the entry is requesting or streaming is ignored */
export const regenerateAllowed = (e: { requesting: boolean; attempt: { status: string } | null } | undefined): boolean =>
  !(e && (e.requesting || e.attempt?.status === 'streaming'));

// ---------------------------------------------------------------- the cap note (2.7, 3.6)

export interface CapNoteInput {
  /** `callBlock()`: null when no cap blocks calls */
  block: CallBlock | null;
  expanded: boolean;
  view: Pick<EntryView, 'text' | 'phase' | 'note'> | null;
}

export interface CapNote {
  show: boolean;
  /** `role="alert"` (announced once: see `createAlertGate`) */
  alert: boolean;
  /** `refused`: the entry's own failure note (a press was refused); `standing`: the callout of a blocked, not-asked entry */
  source: 'none' | 'refused' | 'standing';
}

/**
 * When the cap callout shows in the note row (2.7):
 *  · a refused press (the entry's note is an `ai.callLimit` error) → shown with `role="alert"`;
 *  · no press: while a block is set, the panel is expanded and the entry is `notAsked` (any press would need a model call) →
 *    shown WITHOUT alert; it never shows on an entry with a saved answer (that stays readable) or while another note exists.
 */
export function capNote(i: CapNoteInput): CapNote {
  const n = i.view?.note;
  if (n && n.kind === 'error' && n.code === CALL_LIMIT) return { show: true, alert: true, source: 'refused' };
  if (i.block && i.expanded && isNotAsked(i.view)) return { show: true, alert: false, source: 'standing' };
  return { show: false, alert: false, source: 'none' };
}

/**
 * "Once, when it appears" for `role="alert"`: `take(id)` is true the first time an id shows and false while the same id stays;
 * `take(null)` (no note) resets, so the same note appearing again later is announced again.
 */
export function createAlertGate(): { take(id: string | null): boolean } {
  let last: string | null = null;
  return {
    take(id) {
      if (id == null) {
        last = null;
        return false;
      }
      if (id === last) return false;
      last = id;
      return true;
    },
  };
}
