// AI Chat (#/ai, v0.8). main.ts opens it in mode 'full' when the user may chat now, 'readOnly' when AI+ or AI is off but
// the user has chats or memories (reading and every delete stay; nothing new can be written), else "Not allowed".
//   · An always-visible line under the title says that administrators can view and delete the user's AI chats and
//     memories and see their usage (no pop-up, never dismissible).
//   · Two views: Chats (the chat list, and the conversation with its composer) and Memory (notes the AI uses in every chat).
//     Wider than 900 px the owner's fixed-height box; up to 900 px (v1.0) a full-height chat screen: the document is locked
//     (html.no-doc-scroll), only the lists scroll, the composer is a fixed footer lifted by the on-screen keyboard (--kb),
//     and the page never slides up when the keyboard opens (CH-13). From 720 px the list and the conversation sit side by
//     side; narrower, one at a time with a back button.
//   · Answers stream in (server-sent events over a POST, api/client.ts postStream). The page draws itself in place while an
//     answer streams (only the answer being written is redrawn, at most every 100 ms); render() of the whole page happens
//     only on navigation and menu changes. The composer element is kept across those, so the text, the caret and an input
//     method's composition survive; Enter never sends while an input method is composing (domain/ai-chat.ts shouldSend).
//   · Answers keep being written on the server when the page is left or the connection drops: the page reads the chat again
//     every 2 s until it is finished (a lost stream, the tab coming back, a reload while an answer runs). Stop needs no id.
//   · Assistant text is a small Markdown subset drawn with text nodes only (components/ai-render.ts).
//   · Round 2: every send, retry and memory suggestion carries the interface language of Settings (the AI replies in it);
//     a reasoning model's thinking shows "Thinking…" until its first text
//     (the free mode is a Settings choice for administrators); a suspended AI+ function makes the page read only.
import { githubLine } from '../components/github-line.ts';
import '../styles/ai.css';
import {
  addMemory, deleteAllChats, deleteAllMemories, deleteChat, deleteMemory, fetchChat, fetchChats, fetchMemories, retryAnswer,
  sendMessage, stopAnswer, suggestMemory, updateMemory,
} from '../api/ai.ts';
import { ApiError, isAbortError, type StreamEvent, type StreamOptions } from '../api/client.ts';
import { badgeReasonText } from '../components/ai-badge.ts';
import { fillMarkdown, markdownEl } from '../components/ai-render.ts';
import { iconButton } from '../components/icon-button.ts';
import { miniIcon, type MiniIcon } from '../components/mini-icons.ts';
import { holdTip } from '../components/tooltip.ts';
import {
  applyEvent, atEnd, chatOf, clearsComposer, composerLines, composerStatus, emptyChat, END_PX, fmtAiTime, footerOf, ignoreSendClick, isTerminal, localFail,
  isBusyElsewhere, isThinking, localSend, localStop, LOCK_MAX_WIDTH, locksDocument, MIN_MSGS_PX, noAccessKey, pollResult, readOnlyReason, questionOf,
  retryTarget, sendBody, shouldSend, showsEmptyHint, showsTwoPanes, type ChatView, type ViewMessage,
} from '../domain/ai-chat.ts';
import { chatDisplayTitle, popTitleKey } from '../domain/ai-plus.ts';
import { memoryLength, wantsMemory } from '../domain/ai-memory.ts';
import { shownCode } from '../domain/ai-settings.ts';
import { aiCodeText, callLimitText, errorText } from '../i18n/errors.ts';
import { localeTag, t, tn, uiLang } from '../i18n/index.ts';
import { clockText } from '../i18n/wait.ts';
import { parseHash } from '../router.ts';
import { onRouteChange, onSignedIn, refreshAi, render } from '../state/actions.ts';
import { aiFreeMode, aiStatus, callBlock, noteCallBlock, onCallBlockChange, recheckCallBlock } from '../state/ai.ts';
import { S, isAdmin, onClearUserData } from '../state/store.ts';
import type { AiCallBlock, AiChatKind, AiChatSummary, AiMemory, AiMessage } from '../types/api.ts';
import { forgetStudyChats as forgetPanelChats } from '../components/ai-panel.ts';
import { forgetStudyChats as forgetSummaryChats } from '../components/ai-summary.ts';
import { copyText } from '../utils/clipboard.ts';
import { ask } from '../utils/dialog.ts';
import { h, keepFocus, undoViewportPan, type Child } from '../utils/dom.ts';
import { liftToastAbove, toast } from '../utils/toast.ts';

export type ChatMode = 'full' | 'readOnly';

const POLL_MS = 2000;
const PAINT_MS = 100;
const ico = (name: MiniIcon, cls = 'ai-mi'): SVGSVGElement => miniIcon(name, cls);

// ---------------------------------------------------------------- page-local state (kept while the app runs)

let mode: ChatMode = 'full';
let tab: 'chats' | 'memory' = 'chats';
/** The page was set up since it was opened (the chats are read, a running answer re-attached) */
let entered = false;

let chats: AiChatSummary[] | null = null;
let chatsError: unknown = null;
let chatsLoading = false;
let chatsSeq = 0;

let view: ChatView = emptyChat();
/** A chat is being opened */
let opening = false;
let openError: unknown = null;
/** Phones: the pane shown; null = chosen from the state */
let pane: 'list' | 'conv' | null = null;

/** Why the last send did not start (shown above the composer; the text stays) */
let sendError: string | null = null;
/** The running stream of this page (null: none, or detached) */
let stream: AbortController | null = null;
let pollTimer: number | null = null;
let polling = false;
let paintTimer: number | null = null;
let lastPaint = 0;
/** The conversation follows the newest text (it is scrolled to the end) */
let stick = true;
/** An input method is composing in the composer (cleared one tick after compositionend) */
let composing = false;
/** When the Send button last turned into Stop (Date.now()): a click right after it is part of the click that sent */
let stopSince = 0;
/** The administrator notice (CH-3) shows its full text instead of one line */
let noticeOpen = false;
/** The "more" menu of the chat list (CH-6) is open */
let listMenu = false;

interface Composer {
  form: HTMLFormElement;
  input: HTMLTextAreaElement;
  label: HTMLLabelElement;
  count: HTMLElement;
  btn: HTMLButtonElement;
  /** The status slot above the input (CH-7): a send error, the request-cap callout, or "another chat is busy" */
  status: HTMLElement;
  latest: HTMLButtonElement;
  latestText: HTMLElement;
  /** The fixed footer (CH-13): the form, and the administrator notice line under it */
  dock: HTMLElement;
  notice: HTMLElement;
}
/** Kept across renders of the page, so its text, caret, undo history and composition survive */
let composer: Composer | null = null;
/** What the status slot shows now (it is redrawn only when this changes, so an alert is announced once) */
let statusSig = '';
/** Copy feedback on the icon (CH-5): message id -> 'ok' | 'fail' for 1.5 s */
const copyState = new Map<number, 'ok' | 'fail'>();
const COPY_MS = 1500;

let memories: AiMemory[] | null = null;
let memError: unknown = null;
let memLoading = false;
let memWanted = true;
let memDraft = '';
let memBusy = false;
let memFormError: string | null = null;
let memEdit: { id: number; text: string } | null = null;

/** "Save to memory?" under an answer: the question it is for, the proposed note, and its state */
let suggestion: { messageId: number; text: string; busy: boolean; error: string | null } | null = null;

/** The open chat is in the address (#/ai/<id>; a new chat not sent yet is #/ai/new), so a reload opens it again
 *  (owner's report); replaceState, so the back button is not filled with chats and no route change runs */
function setChatInUrl(id: number | 'new' | null, push = false): void {
  const want = id != null ? `#/ai/${id}` : '#/ai';
  if (location.hash === want) return;
  if (push) {
    // One-pane phone, list -> conversation: a page change, so the system back gesture returns to the list
    history.pushState(history.state, '', want);
    pushedConv = true;
  } else {
    history.replaceState(history.state, '', want);
    if (id == null) pushedConv = false;
  }
}

/** This page pushed a history entry for the open conversation (one-pane phone), so Back there returns to the list */
let pushedConv = false;

/** The open chat was deleted: the address no longer names it (a pushed conversation entry is popped, so Back is not wasted) */
function leaveChatUrl(): void {
  if (pushedConv) {
    pushedConv = false;
    history.back();
  } else {
    setChatInUrl(null);
  }
}

/** One pane at a time (a phone up to 719 px) */
const onePane = (): boolean => phoneScreen() && !showsTwoPanes(window.innerWidth);

/** Shows the chat list pane (the back button, the system back gesture) */
function goList(): void {
  pane = 'list';
  pushedConv = false;
  setChatInUrl(null);
  paint();
  announce(t('ai.tab.chats'));
  document.getElementById(view.chatId != null ? `ai-c-${view.chatId}` : 'ai-new')?.focus();
}

/** The chat the address asks for: an id, 'new' for a new chat not sent yet, or null for the list */
function chatInUrl(): number | 'new' | null {
  const r = parseHash(location.hash);
  if (r.name !== 'ai') return null;
  if (r.chatId != null) return r.chatId;
  return r.fresh ? 'new' : null;
}

/** Codes after which the page reads the AI status again: AI+ switched off, taken away or the AI+ function suspended, or
 *  ai-service no longer connected to the main service (the page then follows: read only) */
const ACCESS_CODES: readonly string[] = ['ai.disabled', 'ai.notAllowed', 'ai.suspended', 'ai.backendDown'];

const onPage = (): boolean => S.route.name === 'ai' && !S.authView && S.phase === 'ready';
const full = (): boolean => mode === 'full';
const sending = (): boolean => stream != null || view.pending != null || view.streamingId != null || view.running;

/** The user's limits from the AI status (the service's current safety limits) */
function limits(): { messageChars: number; memoryChars: number; memories: number } {
  const st = aiStatus();
  if (st.state !== 'ready') return { messageChars: 16000, memoryChars: 200, memories: 50 };
  return { messageChars: st.messageChars ?? st.maxMessageChars, memoryChars: st.memoryChars ?? st.memoryMaxChars, memories: st.memoryLimit };
}

function resetAll(): void {
  detachStream();
  stopPolling();
  watchBusy(false);
  tab = 'chats';
  entered = false;
  chats = null;
  chatsError = null;
  chatsLoading = false;
  chatsSeq += 1;
  view = emptyChat();
  opening = false;
  openError = null;
  pane = null;
  sendError = null;
  stick = true;
  composer = null;
  statusSig = '';
  copyState.clear();
  dockObserver?.disconnect();
  dockObserver = null;
  lockDocument(false);
  listMenu = false;
  noticeOpen = false;
  memories = null;
  memError = null;
  memLoading = false;
  memWanted = true;
  memDraft = '';
  memBusy = false;
  memFormError = null;
  memEdit = null;
  suggestion = null;
}

onRouteChange((from, to) => {
  // #/ai/<id> typed or followed while AI Chat is open: open that chat
  if (from.name === 'ai' && to.name === 'ai' && to.chatId != null && to.chatId !== view.chatId) void openChat(to.chatId);
  if (from.name === 'ai' && to.name === 'ai' && to.fresh && view.chatId != null) newChat();
  if (from.name === 'ai' && to.name !== 'ai') {
    lockDocument(false);
    listMenu = false;
    // The answer keeps being written on the server; coming back re-attaches to it
    detachStream();
    stopPolling();
    watchBusy(false);
    entered = false;
    pane = null;
    pushedConv = false;
    sendError = null;
    memEdit = null;
    memFormError = null;
  }
});
// Signing out (or another user): nothing of this user stays in memory
onClearUserData(resetAll);
onSignedIn(() => {
  entered = false;
  chatsError = null;
  memError = null;
});

let installed = false;
function installOnce(): void {
  if (installed) return;
  installed = true;
  // Back to the tab while an answer was being written: the stream may have died while the tab was hidden
  document.addEventListener('visibilitychange', () => {
    if (document.visibilityState === 'visible' && onPage() && (view.streamingId != null || view.running) && !polling) void checkAttached();
  });
  // Leaving #/ai: the document scrolls again (also covers a route change that did not go through onRouteChange)
  // The system back gesture from a pushed conversation (one-pane phone): the list pane, not another page
  window.addEventListener('popstate', () => {
    if (!onPage() || tab !== 'chats' || !onePane() || chatInUrl() != null) return;
    if (panesClass() === 'ai-panes on-conv') goList();
    else pushedConv = false;
  });
  window.addEventListener('hashchange', () => {
    if (parseHash(location.hash).name !== 'ai') lockDocument(false);
  });
  // Crossing 900 px (a tablet turned): the page is drawn again with or without the lock
  if (typeof window.matchMedia === 'function') {
    window.matchMedia(LOCK_MQ).addEventListener('change', () => {
      if (onPage()) render();
      else lockDocument(false);
    });
  }
  // The request cap came or went (C-6): the callout, Send, Retry and Remember follow without a reload
  onCallBlockChange(() => {
    if (!onPage()) return;
    if (tab === 'chats') paintMsgs();
    syncComposer();
  });
  // The "more" menu of the chat list: a tap outside it or Esc closes it
  document.addEventListener('pointerdown', (e) => {
    if (!listMenu || !(e.target instanceof Node)) return;
    if (e.target instanceof Element && e.target.closest('.ai-more-wrap')) return;
    listMenu = false;
    if (onPage()) paint();
  });
  document.addEventListener('keydown', (e) => {
    if (e.key !== 'Escape' || !listMenu || !onPage()) return;
    listMenu = false;
    paint();
    document.getElementById('ai-more')?.focus();
  });
}

// ---------------------------------------------------------------- the phone screen (CH-1, CH-13)

/** The document is locked up to this width: the one media query of the full-height chat screen */
const LOCK_MQ = `(max-width: ${LOCK_MAX_WIDTH}px)`;
const phoneScreen = (): boolean => locksDocument(window.innerWidth);

/** Observes the composer's footer: its height is what the conversation pane reserves (--composer-h) */
let dockObserver: ResizeObserver | null = null;
/** Observes the message list: when its height changes (keyboard, composer growth) and it was at the end, it stays at the end */
let listObserver: ResizeObserver | null = null;
let panWatching = false;
let panFrame = 0;

/**
 * Locks the document (html.no-doc-scroll: only the lists scroll) or frees it. Every AI Chat render at up to 900 px locks it;
 * leaving the page, or growing wider than 900 px, frees it.
 */
function lockDocument(on: boolean): void {
  const el = document.documentElement;
  el.classList.toggle('no-doc-scroll', on);
  if (on) return;
  el.style.removeProperty('--composer-h');
  listObserver?.disconnect();
  listObserver = null;
  stopPanWatch();
}

function syncComposerHeight(): void {
  const el = document.documentElement;
  if (!composer || !el.classList.contains('no-doc-scroll')) return;
  el.style.setProperty('--composer-h', `${Math.ceil(composer.dock.offsetHeight)}px`);
}

/** Keeps the newest message in view when the list's height changes and the list was at its end (no smooth scroll) */
function watchList(): void {
  listObserver?.disconnect();
  listObserver = null;
  const el = scroller();
  if (!el || typeof ResizeObserver === 'undefined') return;
  listObserver = new ResizeObserver(() => {
    if (stick) el.scrollTop = el.scrollHeight;
  });
  listObserver.observe(el);
}

function onViewportMove(): void {
  if (panFrame) return;
  panFrame = window.requestAnimationFrame(() => {
    panFrame = 0;
    undoViewportPan();
    if (composer) autoGrow(composer.input);
  });
}

/** While the composer has the focus: undo the pan at once, on every viewport change and 300 ms after the focus */
function startPanWatch(): void {
  undoViewportPan();
  window.setTimeout(undoViewportPan, 300);
  if (panWatching) return;
  panWatching = true;
  window.visualViewport?.addEventListener('resize', onViewportMove);
  window.visualViewport?.addEventListener('scroll', onViewportMove);
}

function stopPanWatch(): void {
  if (panWatching) {
    panWatching = false;
    window.visualViewport?.removeEventListener('resize', onViewportMove);
    window.visualViewport?.removeEventListener('scroll', onViewportMove);
  }
  // Once the keyboard has gone
  window.setTimeout(undoViewportPan, 300);
}

// ---------------------------------------------------------------- loading

async function loadChats(): Promise<void> {
  chatsLoading = true;
  const mine = ++chatsSeq;
  try {
    const list = await fetchChats();
    if (mine !== chatsSeq) return;
    chats = list;
    chatsError = null;
  } catch (e) {
    if (mine !== chatsSeq) return;
    chatsError = e;
  } finally {
    if (mine === chatsSeq) chatsLoading = false;
  }
  if (onPage()) paint();
}

async function loadMemories(): Promise<void> {
  memWanted = false;
  memLoading = true;
  try {
    const r = await fetchMemories();
    memories = r.memories;
    memError = null;
  } catch (e) {
    memError = e;
  } finally {
    memLoading = false;
  }
  if (onPage() && tab === 'memory') render();
}

/** First draw after the page was opened: read the chats, and re-attach to an answer that is still being written */
function enter(): void {
  entered = true;
  if (!chatsLoading) void loadChats();
  memWanted = true;
  const st = aiStatus();
  const running = st.state === 'ready' ? st.running : null;
  const wanted = chatInUrl();
  if (running && view.chatId !== running.chatId && !stream) {
    void openChat(running.chatId);
  } else if (wanted === 'new' && view.chatId == null && !stream) {
    // A reload while a new chat was open but not sent yet: open the new chat again (free mode is the server's state)
    queueMicrotask(() => newChat());
  } else if (typeof wanted === 'number' && view.chatId !== wanted && !stream) {
    // A reload (or a link) with #/ai/<id>: open that chat again
    void openChat(wanted);
  } else if ((view.running || view.streamingId != null) && !stream && view.chatId != null) {
    startPolling();
  }
}

// ---------------------------------------------------------------- the conversation

function detachStream(): void {
  const s = stream;
  stream = null;
  s?.abort();
  // The answer keeps running on the server: read the status again so "another chat is answering" shows at once
  if (s) void refreshAi(true).then(() => {
    if (onPage()) syncComposer();
  });
}

function stopPolling(): void {
  polling = false;
  if (pollTimer != null) window.clearTimeout(pollTimer);
  pollTimer = null;
}

/** Reads the chat every 2 s until its answer is finished (the stream was lost, or the answer runs elsewhere) */
function startPolling(): void {
  if (polling || view.chatId == null) return;
  polling = true;
  void pollTick();
}

async function pollTick(): Promise<void> {
  pollTimer = null;
  const id = view.chatId;
  if (!polling || id == null || !onPage()) {
    polling = false;
    return;
  }
  try {
    const d = await fetchChat(id, true);
    if (!polling || view.chatId !== id) return;
    const wasRunning = view.running || view.streamingId != null;
    view = pollResult(view, d);
    paint();
    if (!view.running) {
      polling = false;
      if (wasRunning) announceEnd(view.messages[view.messages.length - 1]);
      void loadChats();
      return;
    }
  } catch (e) {
    if (e instanceof ApiError && e.status === 404) {
      // The chat was deleted meanwhile (another tab, an administrator, the retention)
      polling = false;
      view = emptyChat();
      paint();
      void loadChats();
      return;
    }
    if (e instanceof ApiError && (e.status === 401 || e.status === 403)) {
      polling = false;
      return;
    }
  }
  if (polling && onPage()) pollTimer = window.setTimeout(() => void pollTick(), POLL_MS);
  else polling = false;
}

/** The tab came back while an answer was being written: is the stream still alive? */
async function checkAttached(): Promise<void> {
  const id = view.chatId;
  if (id == null) return;
  try {
    const d = await fetchChat(id, true);
    if (view.chatId !== id) return;
    if (!d.chat.running) {
      // Finished while the tab was hidden; a stream that is still open has nothing more to say
      detachStream();
      view = pollResult(view, d);
      paint();
      announceEnd(view.messages[view.messages.length - 1]);
      void loadChats();
    } else if (!stream) {
      startPolling();
    }
  } catch {
    /* the next look tries again */
  }
}

async function openChat(id: number): Promise<void> {
  detachStream();
  stopPolling();
  suggestion = null;
  sendError = null;
  openError = null;
  const summary = chats?.find((c) => c.id === id);
  view = { ...emptyChat(), chatId: id, title: summary?.title ?? '', kind: summary?.kind ?? 'chat' };
  setChatInUrl(id, onePane() && panesClass() === 'ai-panes on-list');
  opening = true;
  pane = 'conv';
  stick = true;
  paint();
  // One pane at a time: the focus goes to the conversation title (CH-9)
  if (phoneScreen() && !showsTwoPanes(window.innerWidth)) document.getElementById('ai-conv-t')?.focus({ preventScroll: true });
  try {
    const d = await fetchChat(id);
    if (view.chatId !== id) return;
    view = chatOf(d);
    if (view.running) startPolling();
  } catch (e) {
    if (view.chatId !== id) return;
    openError = e;
    if (e instanceof ApiError && e.status === 404) {
      // The chat is gone (deleted, or cleared by an administrator): back to the list address
      setChatInUrl(null);
      void loadChats();
    }
  } finally {
    if (view.chatId === id) opening = false;
  }
  if (!onPage()) return;
  stick = true;
  paint();
  toBottom();
}

function newChat(): void {
  detachStream();
  stopPolling();
  const fromList = onePane() && panesClass() === 'ai-panes on-list';
  view = emptyChat();
  setChatInUrl('new', fromList);
  opening = false;
  openError = null;
  suggestion = null;
  sendError = null;
  pane = 'conv';
  stick = true;
  paint();
  announce(t('ai.new'));
  // New chat keeps the focus on the composer (CH-9)
  composer?.input.focus();
}

function announce(text: string): void {
  const sr = document.getElementById('ai-sr');
  if (!sr) return;
  sr.textContent = '';
  window.setTimeout(() => {
    sr.textContent = text;
  }, 60);
}

function reasonText(code: string): string {
  return aiCodeText(shownCode(code, isAdmin()));
}

/** What the screen reader hears when an answer ends */
function announceEnd(m: ViewMessage | undefined): void {
  if (!m || m.role !== 'assistant') return;
  const f = footerOf(m);
  if (!f) announce(t('ai.status.done'));
  else announce(footerText(m) ?? t('ai.status.done'));
}

function onStreamEvent(ev: StreamEvent): void {
  const before = view;
  view = applyEvent(view, ev);
  switch (ev.event) {
    case 'start': {
      // The message is saved: only now the composer is cleared. Not for a retry: its question is already in the chat, and
      // the composer may hold the next question the user is writing
      if (clearsComposer(before, ev)) clearComposer();
      sendError = null;
      pane = 'conv';
      // A new chat now has its id: put it in the address, so a reload keeps it open
      setChatInUrl(view.chatId);
      stick = true;
      announce(t('ai.writing'));
      paint();
      toBottom();
      void refreshAi(true);
      void loadChats();
      return;
    }
    case 'delta':
      schedulePaint();
      return;
    case 'thinking': {
      // Once per answer, before its first text: "Thinking…" under it (and for screen readers)
      const id = view.streamingId;
      const was = before.messages.find((m) => m.id === id)?.thinking === true;
      const now = view.messages.find((m) => m.id === id)?.thinking === true;
      if (now && !was) {
        paintStreaming();
        announce(t('ai.thinking'));
      }
      return;
    }
    default:
      if (!isTerminal(ev.event)) return;
  }
  const done = before.streamingId;
  paint();
  const msg = done != null ? view.messages.find((m) => m.id === done) : undefined;
  announceEnd(msg);
  void loadChats();
  // AI switched off, AI+ taken away or the AI+ function suspended meanwhile: the page follows (read only, or "Not allowed")
  if (msg?.status === 'error' && msg.error != null && ACCESS_CODES.includes(msg.error)) void refreshAi(true);
  // The question asked to remember something: offer a note (saved only when the user saves it)
  if (ev.event === 'done' && full() && done != null) {
    const q = questionOf(view, done);
    if (q && wantsMemory(q.content) && !callBlock()) void suggest(q.id);
  }
  // The first chat makes the user one "with AI data" (the menu keeps AI Chat even if AI+ is switched off later)
  void refreshAi(true);
}

async function runStream(call: (o: StreamOptions) => Promise<void>, text: string, retry: boolean): Promise<void> {
  if (sending()) return;
  sendError = null;
  if (suggestion && !retry) suggestion = null;
  view = localSend(view, text, retry);
  const ctrl = new AbortController();
  stream = ctrl;
  let started = false;
  let ended = false;
  stick = true;
  syncComposer();
  try {
    await call({
      signal: ctrl.signal,
      onEvent: (ev) => {
        if (stream !== ctrl) return;
        // Signed out meanwhile: stop listening (the answer goes on on the server)
        if (S.authView) {
          detachStream();
          return;
        }
        if (ev.event === 'start') started = true;
        if (isTerminal(ev.event)) ended = true;
        onStreamEvent(ev);
      },
    });
    if (stream === ctrl && started && !ended) startPolling();
  } catch (e) {
    if (stream !== ctrl || isAbortError(e)) return;
    if (!started) {
      // Nothing was saved: the text stays in the composer, the reason above it
      view = localFail(view);
      if (e instanceof ApiError && e.code === 'ai.busy') {
        // Another answer runs: read the status, so "another chat is writing [Stop it]" shows (watchBusy clears it when it
        // ends). Running in this very chat (another tab or device): follow it here, with Stop in the Send slot
        sendError = null;
        void refreshAi(true).then(() => {
          if (!onPage()) return;
          const st = aiStatus();
          if (st.state === 'ready' && st.running && st.running.chatId === view.chatId && !stream) startPolling();
          syncComposer();
        });
      } else if (e instanceof ApiError && e.quota) {
        // A request cap: the callout in the status slot says it (not a red send error); the text stays in the composer
        sendError = null;
        noteCallBlock(e.quota);
        announce(errorText(e));
      } else {
        sendError = errorText(e);
      }
      if (e instanceof ApiError && e.code != null && ACCESS_CODES.includes(e.code)) void refreshAi(true);
      if (e instanceof ApiError && e.code === 'ai.notFound' && view.chatId != null) {
        view = emptyChat();
        void loadChats();
      }
      paint();
    } else {
      // The stream was lost (a phone switching apps, a tunnel blip): the answer goes on; read the chat until it is done
      startPolling();
    }
  } finally {
    if (stream === ctrl) stream = null;
    // Remember / Retry follow the end of the send
    if (onPage() && tab === 'chats') paintMsgs();
    syncComposer();
  }
}

/** The text of the cap that blocks AI calls now (for the tip, the toast and the callout) */
const capText = (q: AiCallBlock): string => callLimitText(q, isAdmin(), Date.now() / 1000);

/** The cap callout in the composer: one short line pair (CH-7 allows 2 lines); capText() is its tip and what a reader hears */
function capShort(q: AiCallBlock): string {
  const cap = t(q.cap === 'minute' ? 'ai.cap.minute' : q.cap === 'hour' ? 'ai.cap.hour' : 'ai.cap.day', { n: q.limit.toLocaleString(localeTag()) });
  const left = Math.max(1, Math.ceil(q.retryAt - Date.now() / 1000));
  const when = q.cap === 'day' ? t('ai.cap.shortDay') : left < 60 ? t('ai.cap.shortSoon') : t('ai.cap.shortWhen', { time: clockText(q.retryAt, Date.now() / 1000) });
  return t(isAdmin() ? 'ai.cap.shortAdmin' : 'ai.cap.short', { cap, when });
}

/**
 * Runs `action` unless the request cap blocks AI calls (C-6): then the tap first asks the server again (an administrator may
 * have raised the limits); still blocked, the cap text is the answer (a toast), free again, the action runs once.
 */
function guarded(action: () => void): () => void {
  return () => {
    if (!callBlock()) {
      action();
      return;
    }
    void recheckCallBlock().then((still) => {
      const q = callBlock();
      if (still && q) toast(capText(q), 6000);
      else if (onPage()) action();
    });
  };
}

/** A control that needs a model call, while the cap blocks calls: aria-disabled (never `disabled`), the cap text as its tip */
function applyCap(btn: HTMLButtonElement): void {
  const q = callBlock();
  if (!q || !full()) return;
  btn.setAttribute('aria-disabled', 'true');
  btn.setAttribute('aria-describedby', 'ai-cap-note');
  holdTip(btn, capText(q));
}

function send(): void {
  if (!full() || !composer || sending()) return;
  const text = composer.input.value.trim();
  if (!text) return;
  if (callBlock()) {
    guarded(send)();
    return;
  }
  const max = limits().messageChars;
  if (memoryLength(text) > max) {
    sendError = aiCodeText('ai.tooLong', max);
    syncComposer();
    return;
  }
  // The reply language is the interface language of Settings; free mode is the administrator's state on the server
  const body = sendBody({ chatId: view.chatId, text, lang: uiLang() });
  void runStream((o) => sendMessage(body, o), text, false);
}

function retry(): void {
  const chatId = view.chatId;
  if (!full() || chatId == null || retryTarget(view) == null) return;
  const lang = uiLang();
  // The Retry button goes away when the answer starts: keep the focus on the composer (it stays focused while an answer streams)
  if (document.activeElement?.id === 'ai-retry') composer?.input.focus();
  void runStream((o) => retryAnswer(chatId, lang, o), '', true);
}

async function stop(): Promise<void> {
  const hadStart = view.streamingId != null;
  const chatId = view.chatId;
  detachStream();
  view = localStop(view);
  paint();
  announce(t('ai.status.stopped'));
  try {
    await stopAnswer(chatId);
  } catch {
    /* the answer may have just ended; the chat is read again below */
  }
  if (!onPage()) return;
  if (!hadStart) void loadChats();
  // The server saves the text written up to the stop: read it
  if (view.chatId != null) startPolling();
}

/** "Stop it" in the status slot: stops the answer that runs in another chat (no id), then reads the status again */
async function stopElsewhere(): Promise<void> {
  try {
    await stopAnswer(null);
  } catch {
    /* the answer may have just ended */
  }
  await refreshAi(true);
  if (onPage()) syncComposer();
}

/** Brings the memory suggestion bar into the free area of the conversation scroller (only its scrollTop moves, never the page) */
function revealSuggest(): void {
  const sc = scroller();
  const bar = sc?.querySelector<HTMLElement>('.ai-suggest');
  if (!sc || !bar) return;
  const s = sc.getBoundingClientRect();
  const b = bar.getBoundingClientRect();
  // The fixed composer covers the bottom of the scroller on phones: keep clear of it
  const dock = document.getElementById('ai-composer');
  const dockTop = dock && getComputedStyle(dock).position === 'fixed' ? dock.getBoundingClientRect().top : Infinity;
  const bottom = Math.min(s.bottom, dockTop);
  if (b.bottom > bottom) sc.scrollTop += b.bottom - bottom + 8;
  if (b.top < s.top) sc.scrollTop -= s.top - b.top + 8;
}

async function suggest(messageId: number): Promise<void> {
  suggestion = { messageId, text: '', busy: true, error: null };
  paintMsgs();
  revealSuggest();
  try {
    const text = await suggestMemory(messageId, uiLang());
    if (suggestion?.messageId !== messageId) return;
    suggestion = { messageId, text, busy: false, error: null };
    paintMsgs();
    revealSuggest();
    return;
  } catch (e) {
    if (suggestion?.messageId !== messageId) return;
    if (e instanceof ApiError && e.quota) {
      // A request cap: the callout in the composer's status slot says it; no suggestion bar
      suggestion = null;
      noteCallBlock(e.quota);
      announce(errorText(e));
      if (onPage()) paintMsgs();
      return;
    }
    suggestion = { messageId, text: '', busy: false, error: errorText(e) };
    if (e instanceof ApiError && e.code != null && ACCESS_CODES.includes(e.code)) void refreshAi(true);
  }
  if (onPage()) {
    paintMsgs();
    revealSuggest();
  }
}

async function saveSuggestion(): Promise<void> {
  const s = suggestion;
  if (!s || s.busy) return;
  const text = s.text.trim();
  if (!text) return;
  suggestion = { ...s, busy: true, error: null };
  paintMsgs();
  try {
    const m = await addMemory(text, 'chat');
    if (memories) memories = [m, ...memories];
    suggestion = null;
    toast(t('ai.mem.saved'));
    void refreshAi(true);
  } catch (e) {
    if (suggestion) suggestion = { ...s, busy: false, error: errorText(e) };
  }
  if (onPage()) paintMsgs();
}

/** The deleted chats' saved hints / explanations / summaries must not be offered as saved any more (null = every chat) */
function forgetStudyChats(chatId: number | null): void {
  forgetPanelChats(chatId);
  forgetSummaryChats(chatId);
}

async function removeChat(c: Pick<AiChatSummary, 'id' | 'title' | 'kind'>): Promise<void> {
  const title = chatTitleText(c);
  if (!(await ask({ title: t('ai.delete'), body: t('ai.delete.ask', { title }), ok: t('common.delete'), danger: true }))) return;
  const idx = chats?.findIndex((x) => x.id === c.id) ?? -1;
  let deleted = false;
  try {
    await deleteChat(c.id);
    deleted = true;
    forgetStudyChats(c.id);
    toast(t('ai.delete.done'));
    if (view.chatId === c.id) {
      detachStream();
      stopPolling();
      view = emptyChat();
      suggestion = null;
      pane = null;
      leaveChatUrl();
    }
  } catch (e) {
    toast(errorText(e), 6000);
  }
  await loadChats();
  void refreshAi(true);
  if (deleted && onPage()) {
    // The row is gone: the focus goes to the next row (else the previous one, else New chat) and the result is announced
    const rows = Array.from(document.querySelectorAll<HTMLElement>('.ai-chats .ai-chat-open'));
    const target = rows[Math.min(Math.max(idx, 0), rows.length - 1)] ?? document.getElementById('ai-new');
    target?.focus({ preventScroll: true });
    announce(t('ai.delete.done'));
  }
}

async function removeAllChats(): Promise<void> {
  const n = chats?.length ?? 0;
  if (!n) return;
  if (!(await ask({ title: t('ai.deleteAll'), body: tn('ai.deleteAll.ask', n), ok: t('common.delete'), danger: true }))) return;
  try {
    await deleteAllChats();
    forgetStudyChats(null);
    detachStream();
    stopPolling();
    view = emptyChat();
    suggestion = null;
    pane = null;
    leaveChatUrl();
    toast(t('ai.delete.done'));
  } catch (e) {
    toast(errorText(e), 6000);
  }
  await loadChats();
  void refreshAi(true);
}

// ---------------------------------------------------------------- scrolling

/** The element that scrolls the conversation: its own box (a fixed-height box on desktop, the free space of the full-height
 *  screen on phones); null only while the conversation pane is not drawn */
function scroller(): HTMLElement | null {
  return document.getElementById('ai-scroll');
}

function nearEnd(): boolean {
  const el = scroller();
  return el ? atEnd(el, END_PX) : true;
}

function toBottom(): void {
  const el = scroller();
  if (el) el.scrollTop = el.scrollHeight;
  stick = true;
  syncLatest();
}

function syncLatest(): void {
  if (!composer) return;
  composer.latest.hidden = stick || view.messages.length === 0 || tab !== 'chats';
}

function onScroll(): void {
  if (!onPage() || tab !== 'chats') return;
  stick = nearEnd();
  syncLatest();
}

// ---------------------------------------------------------------- drawing in place

/** The nodes of a list of children (as h() would append them) */
function nodes(kids: readonly Child[]): Node[] {
  const out: Node[] = [];
  const add = (k: Child): void => {
    if (k == null || k === false) return;
    if (Array.isArray(k)) k.forEach(add);
    else out.push(k instanceof Node ? k : document.createTextNode(String(k)));
  };
  kids.forEach(add);
  return out;
}

/** What the chat list shows: the list is redrawn only when this changes (a poll repaints every 2 s while an answer runs;
 *  redrawing the rows each time would stop a fling and swap the row under a finger) */
let listSig = '';
function listSignature(): string {
  const st = aiStatus();
  return JSON.stringify([uiLang(), full(), tab, listMenu, chatsError != null ? errorText(chatsError) : null, view.chatId, isThinking(view), st.state,
    st.state === 'ready' ? st.retentionDays : null, chats?.map((c) => [c.id, c.title, c.kind, c.running, c.updatedAt]) ?? null]);
}

function paint(): void {
  const panes = document.getElementById('ai-panes');
  if (!panes) return;
  keepFocus(() => {
    panes.className = panesClass();
    // FB-8: the page carries which pane shows, so a phone can drop the title row inside a conversation
    panes.closest('.page-ai')?.classList.toggle('conv-on', panes.classList.contains('on-conv'));
    const lb = document.getElementById('ai-list-body');
    const sig = listSignature();
    if (lb && sig !== listSig) {
      listSig = sig;
      // Redrawing the list must not throw the reader back to the top
      const keep = lb.scrollTop;
      const keepUl = lb.querySelector('.ai-chats')?.scrollTop ?? 0;
      document.getElementById('ai-list-head')?.replaceWith(listHead());
      lb.replaceChildren(...nodes(listBody()));
      if (keep) lb.scrollTop = keep;
      const ul = lb.querySelector('.ai-chats');
      if (ul && keepUl) ul.scrollTop = keepUl;
    }
    document.getElementById('ai-conv-head')?.replaceChildren(...nodes(convHead()));
    paintMsgs();
  });
  syncComposer();
}

/**
 * The title of a chat as shown (v0.9): the stored title, plus the translated suffix of a study chat ("<time>-练习"); an
 * AI Chat conversation without a title is "New chat".
 */
export function chatTitleText(c: { title: string; kind?: AiChatKind | null }): string {
  const { text, suffixKey } = chatDisplayTitle(c);
  return suffixKey ? `${text}-${t(suffixKey)}` : text || t('ai.new');
}

/** The title of a chat in a list */
export function chatTitleEl(c: { title: string; kind?: AiChatKind | null }): HTMLElement {
  return h('span', { class: 'ai-chat-t' }, chatTitleText(c));
}

/**
 * What a user message shows (v0.9): an AI Chat message is its own bubble; a study question (hint / explanation) is a label
 * line ("AI hint · question 3") above the bubble; a summary request is only its label ("AI summary · 8 of 9 wrong answers"),
 * never the machine text the model was given.
 */
export function userBody(m: AiMessage): Child[] {
  if (m.kind === 'summary') {
    const sm = m.sample;
    return [sm ? h('p', { class: 'ai-study-label' }, t('aiplus.chat.label.summary', { used: sm.used, wrong: sm.wrong })) : null];
  }
  const bubble = h('div', { class: 'ai-bubble' }, h('div', { class: 'ai-text' }, m.content));
  if (m.kind === 'hint' || m.kind === 'explain') {
    const label = t(popTitleKey(m.kind, m.source ?? 'practice'), { n: m.questionNo ?? '' });
    return [h('p', { class: 'ai-study-label' }, label), bubble];
  }
  return [bubble];
}

function paintMsgs(): void {
  const box = document.getElementById('ai-msgs-wrap');
  if (!box) return;
  const follow = stick;
  const sc = scroller();
  const top = sc?.scrollTop ?? 0;
  keepFocus(() => box.replaceChildren(...nodes(msgsBody())));
  lastPaint = Date.now();
  if (follow) toBottom();
  else if (sc) sc.scrollTop = top;
  syncLatest();
}

function schedulePaint(): void {
  if (paintTimer != null) return;
  const wait = Math.max(0, PAINT_MS - (Date.now() - lastPaint));
  paintTimer = window.setTimeout(() => {
    paintTimer = null;
    paintStreaming();
  }, wait);
}

/** Redraws only the answer being written (its text and status line), then follows it when the view is at the end */
function paintStreaming(): void {
  lastPaint = Date.now();
  const id = view.streamingId;
  const m = id != null ? view.messages.find((x) => x.id === id) : undefined;
  if (!m) return;
  const follow = stick;
  const md = document.getElementById(`ai-md-${m.id}`);
  const foot = document.getElementById(`ai-foot-${m.id}`);
  if (!md || !foot) {
    paintMsgs();
    return;
  }
  fillMarkdown(md, m.content);
  // "Thinking…" turns into "Writing…" with the first text
  const text = footerText(m);
  if (text != null && foot.textContent !== text) foot.textContent = text;
  paintListRunning();
  if (follow) toBottom();
}

/** The open chat's row in the list says the same as its answer: "Thinking…" while a reasoning model thinks, then
 *  "Writing…" (the list is otherwise drawn only when it is loaded, which may happen before the first "thinking" event) */
function paintListRunning(): void {
  const id = view.chatId;
  const c = id != null ? chats?.find((x) => x.id === id) : undefined;
  const at = c?.running ? document.querySelector(`#ai-c-${c.id} .ai-chat-at`) : null;
  if (!c || !at) return;
  const want = t(listThinking(c) ? 'ai.thinking' : 'ai.writing');
  if (at.textContent !== want) at.textContent = want;
}

// ---------------------------------------------------------------- the chat list

function panesClass(): string {
  const p = pane ?? (view.chatId != null || view.pending || view.messages.length || (full() && chats != null && chats.length === 0) ? 'conv' : 'list');
  return 'ai-panes on-' + p;
}

/** The chat's running answer is a reasoning model still thinking (no text yet): the list says what the message says */
const LANG_NAME = { en: 'ai.langName.en', zh: 'ai.langName.zh', ja: 'ai.langName.ja' } as const;

function listThinking(c: AiChatSummary): boolean {
  return c.id === view.chatId && isThinking(view);
}

/** The list pane's scroller: the rows, then the GitHub line as its last item (the header row stays above it) */
function listBody(): Child[] {
  return [...listRows(), githubLine()];
}

/** The header row of the list pane, above its scroller (so the scroller's edge never cuts its buttons' tap margins):
 *  [+ New chat] ... [more]; the more menu holds Delete all and the retention note (CH-6) */
function listHead(): HTMLElement {
  const st = aiStatus();
  const hasMenu = (chats?.length ?? 0) > 0 || st.state === 'ready';
  // A phone turned sideways shares this row with the tabs: there the button shows only its "+" (the label stays its name
  // and its tap-and-hold tip; CSS hides the text)
  const newBtn = full()
    ? h('button', { class: 'btn ai-new', id: 'ai-new', type: 'button', 'aria-label': t('ai.new'), onclick: () => newChat() },
      ico('plus'), h('span', { class: 'ai-new-l', 'aria-hidden': 'true' }, t('ai.new')))
    : null;
  if (newBtn) holdTip(newBtn, t('ai.new'));
  return h('div', { class: 'ai-list-head', id: 'ai-list-head' },
    newBtn ?? h('span', { class: 'ai-list-title' }, t('ai.tab.chats')),
    tabs('ai-ltab', 'ai-ltabs'),
    hasMenu ? moreMenu() : null);
}

function listRows(): Child[] {
  const out: Child[] = [];
  if (!chats) {
    out.push(chatsError
      ? h('div', { class: 'err', role: 'alert' }, h('p', null, t('ai.err.load', { reason: errorText(chatsError) })),
        h('div', { class: 'acts' }, h('button', { class: 'btn', type: 'button', onclick: () => void loadChats() }, t('common.retry'))))
      : h('p', { class: 'dim ai-list-note', role: 'status' }, t('ai.loading')));
    return out;
  }
  if (!chats.length) {
    out.push(h('p', { class: 'ai-list-note' }, t('ai.empty')));
  } else {
    out.push(h('ul', { class: 'ai-chats', 'aria-label': t('ai.tab.chats') }, chats.map((c) => {
      const title = chatTitleText(c);
      const on = c.id === view.chatId;
      return h('li', { class: 'ai-chat' + (on ? ' on' : '') },
        h('button', {
          class: 'ai-chat-open', id: `ai-c-${c.id}`, type: 'button', 'aria-current': on ? 'true' : null,
          onclick: () => void openChat(c.id),
        },
        chatTitleEl(c),
        h('span', { class: 'ai-chat-at' }, c.running ? t(listThinking(c) ? 'ai.thinking' : 'ai.writing') : fmtAiTime(c.updatedAt, localeTag()))),
        chatDeleteButton(c, title));
    })));
  }
  return out;
}

/** The red trash icon of a list row (44 px hit area on touch, tap-and-hold tip with the name) */
function chatDeleteButton(c: AiChatSummary, title: string): HTMLButtonElement {
  const label = t('ai.deleteOne.aria', { title });
  const btn = iconButton({ icon: 'trash', tone: 'danger', label, onClick: () => void removeChat(c) });
  btn.id = `ai-cdel-${c.id}`;
  btn.classList.add('ai-chat-del');
  return btn;
}

/** The "more" button of the list header and its small menu: Delete all chats (red text, confirm kept) and the retention note */
function moreMenu(): HTMLElement {
  const st = aiStatus();
  const menu = listMenu
    ? h('div', { class: 'ai-menu', id: 'ai-menu', role: 'group', 'aria-label': t('ai.more') },
      chats?.length
        ? h('button', {
          class: 'btn danger ai-delall', id: 'ai-delall', type: 'button',
          onclick: () => {
            listMenu = false;
            paint();
            void removeAllChats();
          },
        }, t('ai.deleteAll'))
        : null,
      st.state === 'ready' ? h('p', { class: 'ai-retention' }, t('ai.retention', { n: st.retentionDays })) : null)
    : null;
  const more = iconButton({
    icon: 'more', label: t('ai.more'),
    onClick: () => {
      listMenu = !listMenu;
      paint();
    },
  });
  more.id = 'ai-more';
  more.classList.add('ai-more');
  more.setAttribute('aria-haspopup', 'true');
  more.setAttribute('aria-expanded', String(listMenu));
  more.setAttribute('aria-controls', 'ai-menu');
  return h('div', { class: 'ai-more-wrap' }, more, menu);
}

// ---------------------------------------------------------------- the conversation pane

function convHead(): Child[] {
  const title = view.chatId == null ? t('ai.new') : chatTitleText(view);
  // [back icon + "All chats" (up to 900 px)] [title, at most 2 lines] [red trash icon]
  const back = iconButton({
    icon: 'back', label: t('ai.back'), showLabel: true,
    onClick: () => {
      // The conversation was pushed as an entry: going back pops it (the popstate listener shows the list)
      if (pushedConv) history.back();
      else goList();
    },
  });
  back.id = 'ai-back';
  back.classList.add('ai-back');
  let del: HTMLButtonElement | null = null;
  if (view.chatId != null) {
    const c = { id: view.chatId, title: view.title, kind: view.kind };
    del = iconButton({ icon: 'trash', tone: 'danger', label: t('ai.deleteOne.aria', { title }), onClick: () => void removeChat(c) });
    del.id = 'ai-del';
    del.classList.add('ai-del');
  }
  return [back, h('h2', { class: 'ai-conv-t', id: 'ai-conv-t', tabindex: '-1', title }, title), del];
}

function footerText(m: ViewMessage): string | null {
  const f = footerOf(m);
  if (!f) return null;
  switch (f.kind) {
    case 'thinking':
      return t('ai.thinking');
    case 'writing':
      return t('ai.writing');
    case 'stopped':
      return t('ai.status.stopped');
    case 'error':
      return t('ai.status.error', { reason: reasonText(f.code) });
    case 'noAnswer':
      return t('ai.status.noAnswer', { reason: reasonText(f.code) });
    default:
      return t('ai.status.truncated');
  }
}

function suggestionBar(): HTMLElement | null {
  const s = suggestion;
  if (!s) return null;
  const max = limits().memoryChars;
  if (s.busy && !s.text) return h('div', { class: 'ai-suggest', role: 'status' }, h('span', { class: 'dim' }, t('ai.mem.suggestBusy')));
  const input = h('input', {
    type: 'text', id: 'ai-suggest-input', class: 'ai-suggest-in', value: s.text, maxlength: String(max), disabled: s.busy,
    'aria-labelledby': 'ai-suggest-l',
    oninput: () => {
      if (suggestion) suggestion = { ...suggestion, text: input.value };
    },
    onkeydown: (e: KeyboardEvent) => {
      if (e.key === 'Enter' && !e.isComposing && e.keyCode !== 229) {
        e.preventDefault();
        void saveSuggestion();
      }
    },
  });
  // The proposed note can be edited (or written by hand when the proposal failed); nothing is saved without Save
  return h('div', { class: 'ai-suggest', role: 'group', 'aria-labelledby': 'ai-suggest-l' },
    h('span', { class: 'ai-suggest-l', id: 'ai-suggest-l' }, miniIcon('sparkle', 'ai-mi'), t('ai.mem.suggest')),
    input,
    s.error ? h('p', { class: 'ai-suggest-err', role: 'alert' }, s.error) : null,
    h('div', { class: 'ai-suggest-acts' },
      h('button', { class: 'btn primary ai-mini', id: 'ai-suggest-save', type: 'button', disabled: s.busy, onclick: () => void saveSuggestion() }, t('ai.mem.save')),
      h('button', {
        class: 'btn quiet ai-mini', id: 'ai-suggest-no', type: 'button', disabled: s.busy,
        onclick: () => {
          suggestion = null;
          paintMsgs();
          composer?.input.focus();
        },
      }, t('ai.mem.dismiss'))));
}

function showCopyState(btn: HTMLElement, st: 'ok' | 'fail' | null): void {
  btn.classList.remove('ok', 'fail');
  if (st) btn.classList.add(st);
  btn.replaceChildren(ico(st === 'ok' ? 'check' : st === 'fail' ? 'xCircle' : 'copy', 'ibtn-ic'));
}

/** Copy: an icon button; the feedback is on the icon (it turns into a check for 1.5 s), no toast; the live region says it (CH-5) */
function copyButton(m: AiMessage, text: string): HTMLElement {
  const state = copyState.get(m.id);
  const btn = iconButton({
    icon: state === 'ok' ? 'check' : state === 'fail' ? 'xCircle' : 'copy', label: t('ai.copyLabel'),
    onClick: async () => {
      const ok = await copyText(text);
      copyState.set(m.id, ok ? 'ok' : 'fail');
      announce(t(ok ? 'ai.copied' : 'ai.copyFailed'));
      // Only this button changes (in place): redrawing every message would replace a button another finger is holding
      showCopyState(btn, ok ? 'ok' : 'fail');
      window.setTimeout(() => {
        copyState.delete(m.id);
        showCopyState(btn, null);
      }, COPY_MS);
    },
  });
  btn.id = `ai-copy-${m.id}`;
  btn.classList.add('ai-copy');
  if (state) btn.classList.add(state);
  return btn;
}

/** What Copy copies: the text shown. A summary request shows only its label, so Copy copies that label, not the
 *  machine line the model received (v0.9). */
function copySource(m: AiMessage): string {
  if (m.role !== 'user' || m.kind !== 'summary') return m.content;
  const sm = m.sample;
  return sm ? t('aiplus.chat.label.summary', { used: sm.used, wrong: sm.wrong }) : '';
}

/** The line right above a message (owner's request): "You" or "AI" (with the model) and the time; right-aligned over a sent
 *  message's bubble. The buttons go on one line right under the message (`actsRow`). */
export function whoLine(m: AiMessage): HTMLElement {
  const user = m.role === 'user';
  return h('div', { class: 'ai-who' }, user ? t('ai.you') : t('ai.assistant'), !user && m.model ? h('span', { class: 'ai-model' }, m.model) : null,
    h('time', { class: 'ai-time', datetime: new Date(m.createdAt * 1000).toISOString() }, fmtAiTime(m.createdAt, localeTag())));
}

function actsRow(buttons: Child[]): HTMLElement | null {
  return buttons.some((b) => b != null && b !== false) ? h('div', { class: 'ai-meta' }, buttons) : null;
}

function messageEl(m: ViewMessage, retryId: number | null): HTMLElement {
  const user = m.role === 'user';
  const busy = m.status === 'streaming';
  const foot = user ? null : footerText(m);
  const bad = m.status === 'error';
  // The suggestion sits under the answer to the question it is for (or under the question while there is no answer yet)
  const next = view.messages[view.messages.indexOf(m) + 1];
  const showSuggest = suggestion != null && (
    (!user && questionOf(view, m.id)?.id === suggestion.messageId) || (user && m.id === suggestion.messageId && (!next || next.role !== 'assistant')));
  // Every message, sent or answered, has a Copy button (owner's request); not while the answer is still being written
  const chatKind = (m.kind ?? 'chat') === 'chat';
  const buttons: Child[] = [
    busy || copySource(m).trim() === '' ? null : copyButton(m, copySource(m)),
    user && full() && chatKind ? rememberButton(m) : null,
    !user && retryId === m.id && full() && chatKind ? retryButton() : null,
  ];
  return h('li', { class: 'ai-msg ' + (user ? 'user' : 'assistant'), id: `ai-m-${m.id}`, 'aria-busy': busy ? 'true' : null },
    whoLine(m),
    ...(user ? userBody(m) : [markdownEl(m.content, `ai-md-${m.id}`)]),
    foot ? h('p', { class: 'ai-foot' + (bad ? ' bad' : ''), id: `ai-foot-${m.id}` }, foot) : null,
    actsRow(buttons),
    showSuggest ? suggestionBar() : null);
}

/** Remember (user messages): asks the model for a note, so it follows the request cap (C-6) */
function rememberButton(m: AiMessage): HTMLElement {
  const btn = iconButton({ icon: 'memory', label: t('ai.mem.remember'), onClick: guarded(() => void suggest(m.id)) });
  btn.id = `ai-rem-${m.id}`;
  btn.classList.add('ai-remember');
  btn.disabled = sending() || suggestion?.busy === true;
  applyCap(btn);
  return btn;
}

/** Retry (a failed answer): another model call, so it follows the request cap (C-6) */
function retryButton(): HTMLElement {
  const btn = iconButton({ icon: 'refresh', label: t('ai.retry'), onClick: guarded(retry) });
  btn.id = 'ai-retry';
  btn.classList.add('ai-retry');
  applyCap(btn);
  return btn;
}

function msgsBody(): Child[] {
  if (opening && !view.messages.length) return [h('p', { class: 'dim ai-conv-note', role: 'status' }, t('ai.loading'))];
  if (openError) {
    return [h('div', { class: 'err', role: 'alert' }, h('p', null, t('ai.err.load', { reason: errorText(openError) })))];
  }
  const out: Child[] = [];
  if (view.running && !stream) out.push(h('p', { class: 'ai-conv-note ai-running' }, t('ai.running')));
  // A new chat with nothing in it yet: one muted line, with the language the AI will answer in (the Settings language)
  if (full() && showsEmptyHint(view)) {
    out.push(h('p', { class: 'ai-empty-hint', id: 'ai-empty-hint' }, t('ai.emptyChat', { lang: t(LANG_NAME[uiLang()]) })));
  }
  const retryId = retryTarget(view);
  out.push(h('ol', { class: 'ai-msgs', id: 'ai-msgs' }, view.messages.map((m) => messageEl(m, retryId))));
  return out;
}

// ---------------------------------------------------------------- the composer

/**
 * The full-height screen (CH-1 / CH-13): the px the input's text may take so that MIN_MSGS_PX of the conversation stay
 * visible between its head and the composer; Infinity elsewhere (desktop box, list pane shown). Read while the input has its
 * one-line height (autoGrow has just set it to auto).
 */
function textRoom(input: HTMLTextAreaElement, chrome: number): number {
  if (!composer || !document.documentElement.classList.contains('no-doc-scroll')) return Number.POSITIVE_INFINITY;
  const head = document.getElementById('ai-conv-head');
  if (!head || !head.getClientRects().length || !composer.dock.getClientRects().length) return Number.POSITIVE_INFINITY;
  const free = composer.dock.getBoundingClientRect().top - head.getBoundingClientRect().bottom;
  return free + input.offsetHeight - chrome - MIN_MSGS_PX;
}

function autoGrow(input: HTMLTextAreaElement): void {
  input.style.height = 'auto';
  const cs = window.getComputedStyle(input);
  const line = parseFloat(cs.lineHeight) || 22;
  const pad = (parseFloat(cs.paddingTop) || 0) + (parseFloat(cs.paddingBottom) || 0) + (parseFloat(cs.borderTopWidth) || 0) + (parseFloat(cs.borderBottomWidth) || 0);
  // CH-7: 4 lines on touch or in a window under 700 px tall, 8 on desktop, then the input scrolls inside; never more than
  // ~22% of the visual viewport, and on the full-height screen never so tall that less than MIN_MSGS_PX of the conversation
  // stays visible (the open keyboard, a short landscape screen)
  const room = window.visualViewport?.height ?? window.innerHeight;
  const lines = composerLines(window.matchMedia('(pointer: coarse)').matches, window.innerHeight, room, line, textRoom(input, pad));
  const max = line * lines + pad;
  const want = input.scrollHeight + (parseFloat(cs.borderTopWidth) || 0) + (parseFloat(cs.borderBottomWidth) || 0);
  input.style.height = `${Math.ceil(Math.min(want, max))}px`;
  input.style.overflowY = want > max ? 'auto' : 'hidden';
}

function clearComposer(): void {
  if (!composer) return;
  composer.input.value = '';
  autoGrow(composer.input);
  syncComposer();
}

function makeComposer(): Composer {
  const input = h('textarea', {
    id: 'ai-input', class: 'ai-input', rows: '1', autocomplete: 'off', spellcheck: 'true', 'aria-describedby': 'ai-count',
    oninput: () => {
      autoGrow(input);
      syncCount();
    },
    // CH-13: iOS pans the page to the focused field; put it back (while the field has the focus and once after it)
    onfocus: () => startPanWatch(),
    onblur: () => stopPanWatch(),
    oncompositionstart: () => {
      composing = true;
    },
    oncompositionend: () => {
      // Safari fires compositionend before the keydown of the Enter that committed the word: clear the flag a tick later
      window.setTimeout(() => {
        composing = false;
      }, 0);
    },
    onkeydown: (e: KeyboardEvent) => {
      const fine = window.matchMedia('(pointer: fine)').matches;
      if (!shouldSend(e, { composing, finePointer: fine })) return;
      e.preventDefault();
      // While an answer is written the text can be typed, but not sent
      if (!sending()) send();
    },
  });
  const label = h('label', { class: 'sr-only', for: 'ai-input' });
  const count = h('span', { class: 'ai-count', id: 'ai-count', hidden: true });
  // Send = the send icon; while an answer is written the same slot is Stop (syncComposer swaps icon, name and tip)
  const btn = iconButton({
    icon: 'send', label: t('ai.send'), tone: 'ai',
    onClick: (e: MouseEvent) => {
      // The first click of a double-click sends and turns this same button into Stop: the second click (and any click
      // within a moment of the switch) must not stop the answer it just started. Enter / Space on the focused button
      // (click count 0) still stops.
      if (ignoreSendClick(e.detail, btn.type === 'button', Date.now() - stopSince)) {
        e.preventDefault();
        return;
      }
      if (btn.type === 'button') {
        e.preventDefault();
        void stop();
      }
    },
  });
  // Pressing Send / Stop must not take the focus from the input (the keyboard would close after every send)
  let keptFocus = false;
  const holdInput = (e: Event): void => {
    keptFocus = document.activeElement === input;
    if (keptFocus) e.preventDefault();
  };
  btn.addEventListener('pointerdown', holdInput);
  btn.addEventListener('mousedown', holdInput);
  btn.addEventListener('click', () => {
    if (keptFocus && document.activeElement !== input) input.focus({ preventScroll: true });
    keptFocus = false;
  });
  btn.id = 'ai-send';
  btn.classList.add('ai-send');
  btn.type = 'submit';
  const status = h('div', { class: 'ai-status', id: 'ai-status', hidden: true });
  const latestText = h('span', null);
  const latest = h('button', {
    class: 'btn ai-latest', id: 'ai-latest', type: 'button', hidden: true,
    onclick: () => {
      toBottom();
      composer?.input.focus({ preventScroll: true });
    },
  }, ico('down'), latestText);
  const form = h('form', {
    class: 'ai-form', id: 'ai-form', novalidate: true,
    onsubmit: (e: SubmitEvent) => {
      e.preventDefault();
      if (sending()) return;
      send();
    },
  }, latest, status, label, input, h('div', { class: 'ai-composer-side' }, count, btn));
  // The fixed footer of CH-13: the form (full mode only) and the administrator notice line under it (CH-3)
  const notice = noticeButton('ai-notice-dock');
  const dock = h('div', { class: 'ai-composer', id: 'ai-composer' }, form, notice);
  dockObserver?.disconnect();
  dockObserver = null;
  if (typeof ResizeObserver !== 'undefined') {
    dockObserver = new ResizeObserver(syncComposerHeight);
    // The border box: html.kb-open changes only the footer's bottom padding (safe area -> 8 px), which the content box misses
    dockObserver.observe(dock, { box: 'border-box' });
  }
  return { form, input, label, count, btn, status, latest, latestText, dock, notice };
}

function toggleNotice(): void {
  noticeOpen = !noticeOpen;
  for (const b of document.querySelectorAll<HTMLElement>('.ai-notice-btn')) {
    b.classList.toggle('open', noticeOpen);
    b.setAttribute('aria-expanded', String(noticeOpen));
  }
  syncComposerHeight();
}

/** The administrator notice as one muted line with the info icon (ellipsis); a tap shows the full text in place (CH-3) */
function noticeButton(extra: string): HTMLButtonElement {
  return h('button', {
    type: 'button', class: `ai-notice-btn ${extra}` + (noticeOpen ? ' open' : ''), 'aria-expanded': String(noticeOpen), onclick: toggleNotice,
  }, ico('info'), h('span', null, t('ai.adminNotice')));
}

function syncCount(): void {
  if (!composer) return;
  const max = limits().messageChars;
  const n = memoryLength(composer.input.value);
  const show = n >= max - Math.min(2000, Math.floor(max / 8));
  composer.count.hidden = !show;
  composer.count.textContent = show ? t('ai.input.count', { n: n.toLocaleString(localeTag()), max: max.toLocaleString(localeTag()) }) : '';
  composer.count.classList.toggle('over', n > max);
}

/** An answer is being written in another chat (CH-7): the status slot says so and offers Stop it */
function busyElsewhere(): boolean {
  const st = aiStatus();
  return st.state === 'ready' && isBusyElsewhere(st.running, view.chatId, sending());
}

/** While "another chat is busy" shows: the AI status is read again every few seconds, so the line goes when that answer ends */
const BUSY_POLL_MS = 3000;
let busyTimer: number | null = null;

function watchBusy(on: boolean): void {
  if (!on) {
    if (busyTimer != null) window.clearTimeout(busyTimer);
    busyTimer = null;
    return;
  }
  if (busyTimer != null) return;
  busyTimer = window.setTimeout(() => {
    busyTimer = null;
    if (!onPage() || !busyElsewhere()) return;
    void refreshAi(true).then(() => {
      if (onPage()) syncComposer();
    });
  }, BUSY_POLL_MS);
}

/** Redraws the status slot when what it shows changed: an error, the cap callout, or "another chat is busy" + Stop it */
function syncStatus(c: Composer): void {
  const block = callBlock();
  const kind = composerStatus({ sendError, capped: block != null, busyElsewhere: busyElsewhere() });
  watchBusy(kind === 'busy' && onPage());
  const text = kind === 'error' ? (sendError ?? '') : kind === 'cap' && block ? capShort(block) : kind === 'busy' ? t('ai.busyElsewhere') : '';
  const sig = `${kind}|${text}|${uiLang()}`;
  if (sig === statusSig) return;
  statusSig = sig;
  // The slot's height changed: the input may now take more or fewer lines (the conversation keeps its room)
  if (composer && document.activeElement === composer.input) queueMicrotask(() => composer && autoGrow(composer.input));
  c.status.hidden = kind == null;
  if (kind === 'error') {
    c.status.replaceChildren(h('div', { class: 'err ai-send-err', id: 'ai-send-err', role: 'alert' }, h('p', null, text)));
  } else if (kind === 'cap') {
    // Warn callout with the clock; no alert role (it only stands there while the cap blocks calls)
    const longText = block ? capText(block) : text;
    const note = h('div', { class: 'callout warn ai-cap', id: 'ai-cap-note' }, ico('clock'),
      h('span', { class: 'ai-cap-t', 'aria-hidden': 'true' }, text), h('span', { class: 'sr-only' }, longText));
    holdTip(note, longText);
    c.status.replaceChildren(note);
  } else if (kind === 'busy') {
    c.status.replaceChildren(h('div', { class: 'ai-busy', id: 'ai-busy' },
      h('span', null, text),
      h('button', { class: 'btn ai-mini', id: 'ai-stop-other', type: 'button', onclick: () => void stopElsewhere() }, t('ai.stopIt'))));
  } else {
    c.status.replaceChildren();
  }
}

/** Brings the composer in line with the state: the language, Send or Stop, the counter, the status slot */
function syncComposer(): void {
  const c = composer;
  if (!c) return;
  c.label.textContent = t('ai.input.label');
  c.input.placeholder = t('ai.input.placeholder');
  const busy = sending();
  if (busy && c.btn.type !== 'button') stopSince = Date.now();
  c.btn.type = busy ? 'button' : 'submit';
  c.btn.classList.toggle('stop', busy);
  const label = t(busy ? 'ai.stop' : 'ai.send');
  const icon = busy ? 'stop' : 'send';
  if (c.btn.dataset.icon !== icon) {
    c.btn.dataset.icon = icon;
    c.btn.replaceChildren(ico(icon, 'ibtn-ic'));
  }
  c.btn.setAttribute('aria-label', label);
  // Send needs a model call: while the cap blocks calls it is aria-disabled with the cap text as its tip; Stop never is
  const q = callBlock();
  if (!busy && q) {
    c.btn.setAttribute('aria-disabled', 'true');
    c.btn.setAttribute('aria-describedby', 'ai-cap-note');
    holdTip(c.btn, capText(q));
  } else {
    c.btn.removeAttribute('aria-disabled');
    c.btn.removeAttribute('aria-describedby');
    holdTip(c.btn, label);
  }
  syncStatus(c);
  c.latestText.textContent = t('ai.latest');
  const noticeText = c.notice.lastElementChild;
  if (noticeText) noticeText.textContent = t('ai.adminNotice');
  syncCount();
  syncLatest();
}

// ---------------------------------------------------------------- Memory

async function addMem(): Promise<void> {
  const text = memDraft.trim();
  if (!text || memBusy) return;
  const max = limits().memoryChars;
  if (memoryLength(text) > max) {
    memFormError = aiCodeText('ai.memoryTooLong', max);
    render();
    return;
  }
  memBusy = true;
  memFormError = null;
  render();
  try {
    const m = await addMemory(text, 'manual');
    memories = [m, ...(memories ?? [])];
    memDraft = '';
    toast(t('ai.mem.saved'));
    void refreshAi(true);
  } catch (e) {
    memFormError = errorText(e);
  }
  memBusy = false;
  if (onPage()) render();
}

async function saveMemEdit(): Promise<void> {
  const ed = memEdit;
  if (!ed || memBusy) return;
  const text = ed.text.trim();
  if (!text) return;
  memBusy = true;
  memFormError = null;
  render();
  try {
    const m = await updateMemory(ed.id, text);
    memories = (memories ?? []).map((x) => (x.id === m.id ? m : x));
    memEdit = null;
  } catch (e) {
    memFormError = errorText(e);
  }
  memBusy = false;
  if (onPage()) render();
  document.getElementById(`ai-medit-${ed.id}`)?.focus();
}

async function removeMem(m: AiMemory): Promise<void> {
  if (!(await ask({ title: t('ai.mem.delete'), body: t('ai.mem.delete.ask'), ok: t('common.delete'), danger: true }))) return;
  try {
    await deleteMemory(m.id);
    memories = (memories ?? []).filter((x) => x.id !== m.id);
    toast(t('ai.mem.deleted'));
    void refreshAi(true);
  } catch (e) {
    toast(errorText(e), 6000);
    memWanted = true;
  }
  if (onPage()) render();
}

async function removeAllMem(): Promise<void> {
  const n = memories?.length ?? 0;
  if (!n) return;
  if (!(await ask({ title: t('ai.mem.deleteAll'), body: tn('ai.mem.deleteAll.ask', n), ok: t('common.delete'), danger: true }))) return;
  try {
    await deleteAllMemories();
    memories = [];
    toast(t('ai.mem.deleted'));
    void refreshAi(true);
  } catch (e) {
    toast(errorText(e), 6000);
    memWanted = true;
  }
  if (onPage()) render();
}

const shortText = (s: string): string => ([...s].length > 40 ? [...s].slice(0, 40).join('') + '…' : s);

function cancelMemEdit(id: number): void {
  memEdit = null;
  memFormError = null;
  render();
  document.getElementById(`ai-medit-${id}`)?.focus();
}

/** An icon button of a memory row (pencil, red trash, save, cancel): the name is the label, disabled while a save runs */
function memIcon(icon: MiniIcon, label: string, id: string, onClick: () => void, tone: 'default' | 'danger' | 'ai' = 'default'): HTMLButtonElement {
  const b = iconButton({ icon, label, tone, onClick });
  b.id = id;
  b.disabled = memBusy;
  return b;
}

function memoryRow(m: AiMemory): HTMLElement {
  const editing = memEdit?.id === m.id;
  const max = limits().memoryChars;
  const src = m.source === 'chat' ? 'ai.mem.source.chat' : 'ai.mem.source.manual';
  if (editing && memEdit) {
    const area = h('textarea', {
      id: `ai-medit-in-${m.id}`, class: 'ai-mem-in', rows: '2', maxlength: String(max), 'aria-label': t('ai.mem.editLabel'),
      oninput: () => {
        if (memEdit) memEdit = { ...memEdit, text: area.value };
      },
      // Esc cancels the edit (like the Cancel button) and gives the focus back to the row's Edit button
      onkeydown: (e: KeyboardEvent) => {
        if (e.key !== 'Escape' || memBusy) return;
        e.preventDefault();
        cancelMemEdit(m.id);
      },
    });
    area.value = memEdit.text;
    return h('li', { class: 'ai-mem-row editing' },
      area,
      h('div', { class: 'ai-mem-acts' }, memIcon('check', t('ai.mem.save'), `ai-msave-${m.id}`, () => void saveMemEdit(), 'ai'),
        memIcon('x', t('ai.mem.cancel'), `ai-mcancel-${m.id}`, () => cancelMemEdit(m.id))));
  }
  return h('li', { class: 'ai-mem-row' },
    h('span', { class: 'ai-mem-src', role: 'img', 'aria-label': t(src), title: t(src) }, miniIcon(m.source === 'chat' ? 'chat' : 'plus', 'ai-mi')),
    h('p', { class: 'ai-mem-t' }, m.content),
    h('div', { class: 'ai-mem-acts' },
      full()
        ? memIcon('pencil', t('ai.mem.editAria', { text: shortText(m.content) }), `ai-medit-${m.id}`, () => {
          memEdit = { id: m.id, text: m.content };
          render();
          document.getElementById(`ai-medit-in-${m.id}`)?.focus();
        })
        : null,
      memIcon('trash', t('ai.mem.deleteAria', { text: shortText(m.content) }), `ai-mdel-${m.id}`, () => void removeMem(m), 'danger')));
}

function memoryView(): HTMLElement {
  if (memWanted && !memLoading) void loadMemories();
  const lim = limits();
  const list = memories;
  let body: Child;
  if (!list) {
    body = memError
      ? h('div', { class: 'err', role: 'alert' }, h('p', null, t('ai.mem.errLoad', { reason: errorText(memError) })),
        h('div', { class: 'acts' }, h('button', { class: 'btn', type: 'button', onclick: () => void loadMemories() }, t('common.retry'))))
      : h('p', { class: 'dim', role: 'status' }, t('ai.loading'));
  } else {
    body = [
      list.length ? h('ul', { class: 'ai-mems' }, list.map(memoryRow)) : h('p', { class: 'ai-list-note' }, t('ai.mem.empty')),
      list.length
        ? h('button', { class: 'btn danger ai-delall', id: 'ai-mdelall', type: 'button', disabled: memBusy, onclick: () => void removeAllMem() }, t('ai.mem.deleteAll'))
        : null,
    ];
  }
  const area = h('textarea', {
    id: 'ai-mem-input', class: 'ai-mem-in', rows: '2', maxlength: String(lim.memoryChars), placeholder: t('ai.mem.placeholder'),
    disabled: memBusy, 'aria-describedby': 'ai-mem-count',
    oninput: () => {
      memDraft = area.value;
      count.textContent = t('ai.mem.count', { n: memoryLength(area.value), max: lim.memoryChars });
    },
  });
  area.value = memDraft;
  const count = h('span', { class: 'ai-count', id: 'ai-mem-count' }, t('ai.mem.count', { n: memoryLength(memDraft), max: lim.memoryChars }));
  const addBtn = iconButton({ icon: 'plus', label: t('ai.mem.add'), tone: 'ai', showLabel: true });
  addBtn.id = 'ai-mem-add';
  addBtn.type = 'submit';
  addBtn.disabled = memBusy || (list != null && list.length >= lim.memories);
  const form = full()
    ? h('form', {
      class: 'ai-mem-add', novalidate: true,
      onsubmit: (e: SubmitEvent) => {
        e.preventDefault();
        void addMem();
      },
    },
    h('label', { class: 'fld-l', for: 'ai-mem-input' }, t('ai.mem.addLabel')),
    h('div', { class: 'ai-mem-addrow' }, area, addBtn),
    count)
    : null;
  return h('section', { class: 'panel ai-mem', 'aria-labelledby': 'ai-mem-h' },
    h('div', { class: 'ai-mem-head' },
      h('h2', { id: 'ai-mem-h' }, t('ai.tab.memory')),
      list ? h('span', { class: 'ai-mem-n' }, t('ai.mem.count', { n: list.length, max: lim.memories })) : null),
    h('p', { class: 'ai-mem-intro' }, t('ai.mem.intro')),
    form,
    memFormError ? h('div', { class: 'err', role: 'alert' }, h('p', null, memFormError)) : null,
    body,
    // The last item of the memory scroller
    githubLine());
}

// ---------------------------------------------------------------- the page

/** The Chats / Memory switch: icon + short label; icon only up to 380 px (CSS), the name stays as aria-label and tip (CH-2).
 *  `idp` tells the three places apart: the title row, the list header and the memory head (the last two show in short
 *  landscape only, where the title row is hidden, CH-11) */
function tabs(idp = 'ai-tab', cls = ''): HTMLElement {
  const b = (which: 'chats' | 'memory', label: string, icon: MiniIcon): HTMLElement => holdTip(h('button', {
    type: 'button', id: `${idp}-${which}`, 'aria-pressed': String(tab === which), 'aria-label': label, title: label,
    onclick: () => {
      if (tab === which) return;
      tab = which;
      memEdit = null;
      render();
      // A tab switch keeps the focus on the pressed tab (the visible one of the sets; CH-9). Back to an open conversation on a
      // phone no tab is shown (FB-8): the focus goes to the conversation title, as when a chat is opened
      const next = ['ai-tab', 'ai-ltab', 'ai-mtab'].map((p) => document.getElementById(`${p}-${which}`)).find((el) => el?.offsetParent != null);
      (next ?? document.getElementById('ai-conv-t'))?.focus({ preventScroll: true });
    },
  }, ico(icon), h('span', { class: 'ai-tab-l' }, label)), label);
  return h('div', { class: 'seg ai-tabs' + (cls ? ` ${cls}` : ''), role: 'group', 'aria-label': t('ai.tabs') },
    b('chats', t('ai.tab.chats'), 'chat'), b('memory', t('ai.tab.memory'), 'memory'));
}

/** CH-8: a paused user with chats sees one muted line in the composer's place (the notice line stays under it) */
function readOnlyLine(): HTMLElement {
  const st = aiStatus();
  const reason = st.state === 'ready' ? readOnlyReason(st) : 'unavailable';
  const paused = reason === 'suspended';
  const text = paused
    ? t('ai.readOnly.paused')
    : (reason === 'noFunction' ? t('ai.err.notAllowed') : badgeReasonText(reason, isAdmin())) + (uiLang() === 'en' ? ' ' : '') + t('ai.readOnlyTail');
  return h('p', { class: 'ai-readonly-line', id: 'ai-readonly' }, ico(paused ? 'pauseCircle' : 'info'), h('span', null, text));
}

function chatsView(): HTMLElement {
  if (!composer) composer = makeComposer();
  syncComposer();
  listSig = listSignature();
  composer.dock.replaceChildren(...(full() ? [composer.form] : [readOnlyLine()]), composer.notice);
  const conv = h('section', { class: 'ai-conv', 'aria-labelledby': 'ai-conv-t' },
    h('div', { class: 'ai-conv-head', id: 'ai-conv-head' }, convHead()),
    h('div', { class: 'ai-scroll', id: 'ai-scroll', tabindex: '0', role: 'region', 'aria-label': t('ai.msgs.label'), onscroll: onScroll },
      h('div', { class: 'ai-msgs-wrap', id: 'ai-msgs-wrap' }, msgsBody())),
    composer.dock);
  return h('div', { class: panesClass(), id: 'ai-panes' },
    h('aside', { class: 'ai-list', 'aria-label': t('ai.tab.chats') },
      listHead(),
      // The footer line is the last item of the list's scroller (listBody), never inside a conversation
      h('div', { class: 'ai-list-body', id: 'ai-list-body' }, listBody()),
      // The notice also shows under the list on phones (the composer's one is in the conversation pane)
      noticeButton('ai-notice-flow')),
    conv);
}

/**
 * #/ai while the AI service cannot be reached (its status could not be read): says so, instead of "Not allowed", which would
 * blame the account. Retry reads the status again; the page follows when the service answers.
 */
export function renderAiUnavailable(): HTMLElement {
  lockDocument(false);
  const retryBtn = h('button', {
    class: 'btn', id: 'ai-unavail-retry', type: 'button',
    onclick: async () => {
      retryBtn.disabled = true;
      await refreshAi(true);
      // Still unreachable: no state change redraws the page, so it is drawn here (the button comes back)
      render();
    },
  }, t('common.retry'));
  return h('div', { class: 'page page-ai' },
    h('header', { class: 'page-head ai-head' }, h('h1', null, t('ai.title'))),
    h('div', { class: 'callout warn ai-unavail', role: 'alert' }, h('p', null, t(isAdmin() ? 'ai.err.unavailableAdmin' : 'ai.err.unavailable')), h('div', { class: 'acts' }, retryBtn)));
}

/**
 * #/ai for a user who cannot use AI and has no chats or memories to read: says why and what to do (instead of a bare
 * "Not allowed"). The cause is read from the AI status: the AI+ function paused, the AI+ feature off for everyone, or the
 * account without it. Nothing here names the ai-service container (administrators see that only in AI settings).
 */
export function renderAiNoAccess(): HTMLElement {
  lockDocument(false);
  const st = aiStatus();
  const key = st.state === 'ready' ? noAccessKey(st) : 'ai.noAccess.noFn';
  return h('div', { class: 'page page-ai' },
    h('header', { class: 'page-head ai-head' }, h('h1', null, t('ai.title'))),
    h('div', { class: 'callout note ai-noaccess', id: 'ai-noaccess', role: 'status' }, h('p', null, t(key))));
}

export function renderAiChat(m: ChatMode): HTMLElement {
  installOnce();
  mode = m;
  // Up to 900 px the document is locked: only the lists scroll (CH-1, CH-13)
  lockDocument(phoneScreen());
  if (!entered) enter();
  const root = h('div', { class: 'page page-ai' + (m === 'readOnly' ? ' ro' : '') + (tab === 'chats' && panesClass() === 'ai-panes on-conv' ? ' conv-on' : '') },
    h('header', { class: 'page-head ai-head' },
      // Free mode on (administrators, chosen in Settings): a green tag next to the title; nothing in normal mode
      h('div', { class: 'ai-title' }, h('h1', null, t('ai.title')),
        isAdmin() && aiFreeMode() ? h('span', { class: 'ai-mode-tag', role: 'status' }, t('ai.freeTag')) : null),
      tabs()),
    // Wide screens: the line under the title; up to 900 px it sits under the composer / the lists instead (CH-3)
    h('p', { class: 'ai-notice ai-notice-top' }, miniIcon('info', 'ai-mi'), h('span', null, t('ai.adminNotice'))),
    tab === 'chats' ? chatsView() : h('div', { class: 'ai-memwrap' }, tabs('ai-mtab', 'ai-ltabs ai-mtabs'), memoryView(), noticeButton('ai-notice-flow')),
    h('div', { class: 'sr-only', id: 'ai-sr', role: 'status', 'aria-live': 'polite' }));
  // CH-13: every other text field (suggestion, memory edits) also undoes iOS's page pan while it has the focus
  const otherField = (e: Event): boolean => {
    const el = e.target as HTMLElement | null;
    return !!el && (el.tagName === 'INPUT' || el.tagName === 'TEXTAREA') && el.id !== 'ai-input';
  };
  root.addEventListener('focusin', (e) => {
    if (otherField(e) && document.documentElement.classList.contains('no-doc-scroll')) startPanWatch();
  });
  root.addEventListener('focusout', (e) => {
    if (otherField(e)) stopPanWatch();
  });
  // After the page is in the document: the composer's height, and the conversation at its end
  queueMicrotask(() => {
    if (!root.isConnected) return;
    if (tab !== 'chats') {
      listObserver?.disconnect();
      listObserver = null;
      liftToastAbove(null);
      return;
    }
    // Toasts sit above the composer's footer (SH-4)
    liftToastAbove(composer && full() ? composer.dock : null);
    if (composer) autoGrow(composer.input);
    syncComposerHeight();
    watchList();
    if (stick) toBottom();
  });
  return root;
}
