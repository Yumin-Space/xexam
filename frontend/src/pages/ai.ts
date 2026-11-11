// AI Chat (#/ai, v0.8). main.ts opens it in mode 'full' when the user may chat now, 'readOnly' when AI+ or AI is off but
// the user has chats or memories (reading and every delete stay; nothing new can be written), else "Not allowed".
//   · An always-visible line under the title says that administrators can view and delete the user's AI chats and
//     memories and see their usage (no pop-up, never dismissible).
//   · Two views: Chats (the chat list, and the conversation with its composer) and Memory (notes the AI uses in every chat).
//     From 900 px wide the list and the conversation sit side by side; narrower, one at a time with a back button.
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
import '../styles/ai.css';
import {
  addMemory, deleteAllChats, deleteAllMemories, deleteChat, deleteMemory, fetchChat, fetchChats, fetchMemories, retryAnswer,
  sendMessage, stopAnswer, suggestMemory, updateMemory,
} from '../api/ai.ts';
import { ApiError, isAbortError, type StreamEvent, type StreamOptions } from '../api/client.ts';
import { badgeReasonText } from '../components/ai-badge.ts';
import { fillMarkdown, markdownEl } from '../components/ai-render.ts';
import { miniIcon } from '../components/mini-icons.ts';
import {
  applyEvent, chatOf, clearsComposer, emptyChat, fmtAiTime, footerOf, ignoreSendClick, isTerminal, localFail,
  isThinking, localSend, localStop, noAccessKey, pollResult, readOnlyReason, questionOf, retryTarget, sendBody, shouldSend, showsEmptyHint, type ChatView, type ViewMessage,
} from '../domain/ai-chat.ts';
import { memoryLength, wantsMemory } from '../domain/ai-memory.ts';
import { shownCode } from '../domain/ai-settings.ts';
import { aiCodeText, errorText } from '../i18n/errors.ts';
import { localeTag, t, tn, uiLang } from '../i18n/index.ts';
import { parseHash } from '../router.ts';
import { onRouteChange, onSignedIn, refreshAi, render } from '../state/actions.ts';
import { aiFreeMode, aiStatus } from '../state/ai.ts';
import { S, isAdmin, onClearUserData } from '../state/store.ts';
import type { AiChatSummary, AiMemory, AiMessage } from '../types/api.ts';
import { ask } from '../utils/dialog.ts';
import { h, keepFocus, type Child } from '../utils/dom.ts';
import { toast } from '../utils/toast.ts';

export type ChatMode = 'full' | 'readOnly';

const POLL_MS = 2000;
const PAINT_MS = 100;
/** Within this many pixels of the end the conversation follows new text */
const FOLLOW_PX = 48;

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

interface Composer {
  form: HTMLFormElement;
  input: HTMLTextAreaElement;
  label: HTMLLabelElement;
  count: HTMLElement;
  btn: HTMLButtonElement;
  err: HTMLElement;
  latest: HTMLButtonElement;
  latestText: HTMLElement;
}
/** Kept across renders of the page, so its text, caret, undo history and composition survive */
let composer: Composer | null = null;

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
function setChatInUrl(id: number | 'new' | null): void {
  const want = id != null ? `#/ai/${id}` : '#/ai';
  if (location.hash !== want) history.replaceState(history.state, '', want);
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
  if (st.state !== 'ready') return { messageChars: 16000, memoryChars: 300, memories: 50 };
  return { messageChars: st.messageChars ?? st.maxMessageChars, memoryChars: st.memoryChars ?? st.memoryMaxChars, memories: st.memoryLimit };
}

function resetAll(): void {
  detachStream();
  stopPolling();
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
    // The answer keeps being written on the server; coming back re-attaches to it
    detachStream();
    stopPolling();
    entered = false;
    pane = null;
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
  window.addEventListener('scroll', onScroll, { passive: true });
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
  view = { ...emptyChat(), chatId: id, title: summary?.title ?? '' };
  setChatInUrl(id);
  opening = true;
  pane = 'conv';
  stick = true;
  paint();
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
  view = emptyChat();
  setChatInUrl('new');
  opening = false;
  openError = null;
  suggestion = null;
  sendError = null;
  pane = 'conv';
  stick = true;
  paint();
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
    if (q && wantsMemory(q.content)) void suggest(q.id);
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
      sendError = errorText(e);
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
    syncComposer();
  }
}

function send(): void {
  if (!full() || !composer || sending()) return;
  const text = composer.input.value.trim();
  if (!text) return;
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

async function suggest(messageId: number): Promise<void> {
  suggestion = { messageId, text: '', busy: true, error: null };
  paintMsgs();
  try {
    const text = await suggestMemory(messageId, uiLang());
    if (suggestion?.messageId !== messageId) return;
    suggestion = { messageId, text, busy: false, error: null };
  } catch (e) {
    if (suggestion?.messageId !== messageId) return;
    suggestion = { messageId, text: '', busy: false, error: errorText(e) };
    if (e instanceof ApiError && e.code != null && ACCESS_CODES.includes(e.code)) void refreshAi(true);
  }
  if (onPage()) paintMsgs();
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

async function removeChat(c: Pick<AiChatSummary, 'id' | 'title'>): Promise<void> {
  const title = c.title || t('ai.new');
  if (!(await ask({ title: t('ai.delete'), body: t('ai.delete.ask', { title }), ok: t('common.delete'), danger: true }))) return;
  try {
    await deleteChat(c.id);
    toast(t('ai.delete.done'));
    if (view.chatId === c.id) {
      detachStream();
      stopPolling();
      view = emptyChat();
      suggestion = null;
      pane = null;
    }
  } catch (e) {
    toast(errorText(e), 6000);
  }
  await loadChats();
  void refreshAi(true);
}

async function removeAllChats(): Promise<void> {
  const n = chats?.length ?? 0;
  if (!n) return;
  if (!(await ask({ title: t('ai.deleteAll'), body: tn('ai.deleteAll.ask', n), ok: t('common.delete'), danger: true }))) return;
  try {
    await deleteAllChats();
    detachStream();
    stopPolling();
    view = emptyChat();
    suggestion = null;
    pane = null;
    toast(t('ai.delete.done'));
  } catch (e) {
    toast(errorText(e), 6000);
  }
  await loadChats();
  void refreshAi(true);
}

// ---------------------------------------------------------------- scrolling

/** The element that scrolls the conversation: always its own fixed-height box (on phones too, owner's request); null only
 *  while the conversation pane is not drawn */
function scroller(): HTMLElement | null {
  return document.getElementById('ai-scroll');
}

/** Phones: how far the end of the conversation (its composer) is below the bottom of the window */
function convBelow(): number {
  const conv = document.querySelector('.ai-conv');
  return conv ? conv.getBoundingClientRect().bottom - window.innerHeight : 0;
}

function nearEnd(): boolean {
  const el = scroller();
  if (el) return el.scrollHeight - el.scrollTop - el.clientHeight <= FOLLOW_PX;
  return convBelow() <= FOLLOW_PX;
}

function toBottom(): void {
  const el = scroller();
  if (el) el.scrollTop = el.scrollHeight;
  // Phones: the end of the conversation at the bottom of the window (not the page's footer); never while the chat list
  // is the pane shown (the conversation is hidden then)
  else if (document.getElementById('ai-msgs') && (document.querySelector('.ai-conv') as HTMLElement | null)?.offsetParent) {
    window.scrollBy(0, convBelow() + 8);
  }
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

function paint(): void {
  const panes = document.getElementById('ai-panes');
  if (!panes) return;
  keepFocus(() => {
    panes.className = panesClass();
    document.getElementById('ai-list-body')?.replaceChildren(...nodes(listBody()));
    document.getElementById('ai-conv-head')?.replaceChildren(...nodes(convHead()));
    paintMsgs();
  });
  syncComposer();
}

/** The title of a chat in a list */
export function chatTitleEl(title: string): HTMLElement {
  return h('span', { class: 'ai-chat-t' }, title);
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

function listBody(): Child[] {
  const out: Child[] = [];
  if (full()) {
    out.push(h('button', { class: 'btn ai-new', id: 'ai-new', type: 'button', onclick: () => newChat() }, miniIcon('plus', 'ai-mi'), t('ai.new')));
  }
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
      const title = c.title || t('ai.new');
      const on = c.id === view.chatId;
      return h('li', { class: 'ai-chat' + (on ? ' on' : '') },
        h('button', {
          class: 'ai-chat-open', id: `ai-c-${c.id}`, type: 'button', 'aria-current': on ? 'true' : null,
          onclick: () => void openChat(c.id),
        },
        chatTitleEl(title),
        h('span', { class: 'ai-chat-at' }, c.running ? t(listThinking(c) ? 'ai.thinking' : 'ai.writing') : fmtAiTime(c.updatedAt, localeTag()))),
        h('button', {
          class: 'icon-btn ai-chat-del', id: `ai-cdel-${c.id}`, type: 'button', 'aria-label': t('ai.deleteOne.aria', { title }),
          title: t('ai.deleteOne.aria', { title }), onclick: () => void removeChat(c),
        }, miniIcon('trash', 'ai-mi')));
    })));
    out.push(h('button', { class: 'btn danger ai-delall', id: 'ai-delall', type: 'button', onclick: () => void removeAllChats() }, t('ai.deleteAll')));
  }
  const st = aiStatus();
  if (st.state === 'ready') out.push(h('p', { class: 'ai-retention' }, t('ai.retention', { n: st.retentionDays })));
  return out;
}

// ---------------------------------------------------------------- the conversation pane

function convHead(): Child[] {
  const title = view.chatId == null ? t('ai.new') : view.title || t('ai.new');
  return [
    h('button', {
      class: 'btn quiet ai-back', id: 'ai-back', type: 'button',
      onclick: () => {
        pane = 'list';
        setChatInUrl(null);
        paint();
        document.getElementById(view.chatId != null ? `ai-c-${view.chatId}` : 'ai-new')?.focus();
      },
    }, miniIcon('back', 'ai-mi'), t('ai.back')),
    h('h2', { class: 'ai-conv-t', id: 'ai-conv-t' }, title),
    view.chatId != null
      ? h('button', {
        class: 'btn ai-mini danger ai-del', id: 'ai-del', type: 'button',
        onclick: () => void removeChat({ id: view.chatId as number, title: view.title }),
      }, t('ai.delete'))
      : null,
  ];
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

/** Copies a message's text: the async clipboard where allowed, else a hidden textarea and execCommand (older browsers,
 *  plain-http hosts) */
async function copyText(text: string): Promise<boolean> {
  try {
    if (navigator.clipboard && window.isSecureContext) {
      await navigator.clipboard.writeText(text);
      return true;
    }
  } catch {
    // fall through to the old way
  }
  const area = h('textarea', { class: 'sr-only', readonly: true, 'aria-hidden': 'true', tabindex: '-1' });
  area.value = text;
  document.body.append(area);
  area.select();
  let ok = false;
  try {
    ok = document.execCommand('copy');
  } catch {
    ok = false;
  }
  area.remove();
  return ok;
}

function copyButton(m: AiMessage): HTMLElement {
  return h('button', {
    class: 'btn quiet ai-mini ai-copy', id: `ai-copy-${m.id}`, type: 'button', 'aria-label': t('ai.copyLabel'),
    onclick: async () => toast(t((await copyText(m.content)) ? 'ai.copied' : 'ai.copyFailed'), 2500),
  }, miniIcon('copy', 'ai-mi'), t('ai.copy'));
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
  const buttons: Child[] = [
    busy || m.content.trim() === '' ? null : copyButton(m),
    user && full()
      ? h('button', {
        class: 'btn quiet ai-mini ai-remember', id: `ai-rem-${m.id}`, type: 'button', disabled: sending() || suggestion?.busy === true,
        onclick: () => void suggest(m.id),
      }, miniIcon('sparkle', 'ai-mi'), t('ai.mem.remember'))
      : null,
    !user && retryId === m.id && full()
      ? h('button', { class: 'btn ai-mini', id: 'ai-retry', type: 'button', onclick: retry }, t('ai.retry'))
      : null,
  ];
  return h('li', { class: 'ai-msg ' + (user ? 'user' : 'assistant'), id: `ai-m-${m.id}`, 'aria-busy': busy ? 'true' : null },
    whoLine(m),
    user ? h('div', { class: 'ai-bubble' }, h('div', { class: 'ai-text' }, m.content)) : markdownEl(m.content, `ai-md-${m.id}`),
    foot ? h('p', { class: 'ai-foot' + (bad ? ' bad' : ''), id: `ai-foot-${m.id}` }, foot) : null,
    actsRow(buttons),
    showSuggest ? suggestionBar() : null);
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

function autoGrow(input: HTMLTextAreaElement): void {
  input.style.height = 'auto';
  const cs = window.getComputedStyle(input);
  const line = parseFloat(cs.lineHeight) || 22;
  const pad = (parseFloat(cs.paddingTop) || 0) + (parseFloat(cs.paddingBottom) || 0) + (parseFloat(cs.borderTopWidth) || 0) + (parseFloat(cs.borderBottomWidth) || 0);
  const max = line * 8 + pad;
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
  const btn = h('button', {
    id: 'ai-send', class: 'btn primary ai-send', type: 'submit',
    onclick: (e: MouseEvent) => {
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
  const err = h('div', { class: 'err ai-send-err', id: 'ai-send-err', role: 'alert', hidden: true });
  const latestText = h('span', null);
  const latest = h('button', {
    class: 'btn ai-latest', id: 'ai-latest', type: 'button', hidden: true,
    onclick: () => {
      toBottom();
      composer?.input.focus({ preventScroll: true });
    },
  }, miniIcon('down', 'ai-mi'), latestText);
  const form = h('form', {
    class: 'ai-composer', id: 'ai-composer', novalidate: true,
    onsubmit: (e: SubmitEvent) => {
      e.preventDefault();
      if (sending()) return;
      send();
    },
  }, latest, err, label, input, h('div', { class: 'ai-composer-side' }, count, btn));
  return { form, input, label, count, btn, err, latest, latestText };
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

/** Brings the composer in line with the state: the language, Send or Stop, the counter, the error */
function syncComposer(): void {
  const c = composer;
  if (!c) return;
  c.label.textContent = t('ai.input.label');
  c.input.placeholder = t('ai.input.placeholder');
  const busy = sending();
  if (busy && c.btn.type !== 'button') stopSince = Date.now();
  c.btn.type = busy ? 'button' : 'submit';
  c.btn.classList.toggle('stop', busy);
  c.btn.classList.toggle('primary', !busy);
  c.btn.textContent = t(busy ? 'ai.stop' : 'ai.send');
  c.err.hidden = !sendError;
  c.err.replaceChildren(sendError ? h('p', null, sendError) : '');
  c.latestText.textContent = t('ai.latest');
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
      h('div', { class: 'ai-mem-acts' },
        h('button', { class: 'btn primary ai-mini', id: `ai-msave-${m.id}`, type: 'button', disabled: memBusy, onclick: () => void saveMemEdit() }, t('ai.mem.save')),
        h('button', {
          class: 'btn quiet ai-mini', type: 'button', disabled: memBusy,
          onclick: () => cancelMemEdit(m.id),
        }, t('ai.mem.cancel'))));
  }
  return h('li', { class: 'ai-mem-row' },
    h('span', { class: 'ai-mem-src', role: 'img', 'aria-label': t(src), title: t(src) }, miniIcon(m.source === 'chat' ? 'chat' : 'pencil', 'ai-mi')),
    h('p', { class: 'ai-mem-t' }, m.content),
    h('div', { class: 'ai-mem-acts' },
      full()
        ? h('button', {
          class: 'btn quiet ai-mini', id: `ai-medit-${m.id}`, type: 'button', disabled: memBusy, 'aria-label': t('ai.mem.editAria', { text: shortText(m.content) }),
          onclick: () => {
            memEdit = { id: m.id, text: m.content };
            render();
            document.getElementById(`ai-medit-in-${m.id}`)?.focus();
          },
        }, t('ai.mem.edit'))
        : null,
      h('button', {
        class: 'btn quiet ai-mini danger', id: `ai-mdel-${m.id}`, type: 'button', disabled: memBusy,
        'aria-label': t('ai.mem.deleteAria', { text: shortText(m.content) }), onclick: () => void removeMem(m),
      }, t('ai.mem.delete'))));
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
  const form = full()
    ? h('form', {
      class: 'ai-mem-add', novalidate: true,
      onsubmit: (e: SubmitEvent) => {
        e.preventDefault();
        void addMem();
      },
    },
    h('label', { class: 'fld-l', for: 'ai-mem-input' }, t('ai.mem.addLabel')),
    area,
    h('div', { class: 'ai-mem-addrow' }, count,
      h('button', { class: 'btn primary ai-mini', id: 'ai-mem-add', type: 'submit', disabled: memBusy || (list != null && list.length >= lim.memories) },
        miniIcon('plus', 'ai-mi'), t('ai.mem.add'))))
    : null;
  return h('section', { class: 'panel ai-mem', 'aria-labelledby': 'ai-mem-h' },
    h('div', { class: 'ai-mem-head' },
      h('h2', { id: 'ai-mem-h' }, t('ai.tab.memory')),
      list ? h('span', { class: 'ai-mem-n' }, t('ai.mem.count', { n: list.length, max: lim.memories })) : null),
    h('p', { class: 'ai-mem-intro' }, t('ai.mem.intro')),
    form,
    memFormError ? h('div', { class: 'err', role: 'alert' }, h('p', null, memFormError)) : null,
    body);
}

// ---------------------------------------------------------------- the page

function tabs(): HTMLElement {
  const b = (which: 'chats' | 'memory', label: string): HTMLElement => h('button', {
    type: 'button', id: `ai-tab-${which}`, 'aria-pressed': String(tab === which),
    onclick: () => {
      if (tab === which) return;
      tab = which;
      memEdit = null;
      render();
    },
  }, label);
  return h('div', { class: 'seg ai-tabs', role: 'group', 'aria-label': t('ai.tabs') }, b('chats', t('ai.tab.chats')), b('memory', t('ai.tab.memory')));
}

function chatsView(): HTMLElement {
  if (!composer) composer = makeComposer();
  syncComposer();
  const conv = h('section', { class: 'ai-conv', 'aria-labelledby': 'ai-conv-t' },
    h('div', { class: 'ai-conv-head', id: 'ai-conv-head' }, convHead()),
    h('div', { class: 'ai-scroll', id: 'ai-scroll', tabindex: '0', role: 'region', 'aria-label': t('ai.msgs.label'), onscroll: onScroll },
      h('div', { class: 'ai-msgs-wrap', id: 'ai-msgs-wrap' }, msgsBody())),
    full() ? composer.form : null);
  return h('div', { class: panesClass(), id: 'ai-panes' },
    h('aside', { class: 'ai-list', 'aria-label': t('ai.tab.chats') }, h('div', { class: 'ai-list-body', id: 'ai-list-body' }, listBody())),
    conv);
}

/**
 * #/ai while the AI service cannot be reached (its status could not be read): says so, instead of "Not allowed", which would
 * blame the account. Retry reads the status again; the page follows when the service answers.
 */
export function renderAiUnavailable(): HTMLElement {
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
  const st = aiStatus();
  const key = st.state === 'ready' ? noAccessKey(st) : 'ai.noAccess.noFn';
  return h('div', { class: 'page page-ai' },
    h('header', { class: 'page-head ai-head' }, h('h1', null, t('ai.title'))),
    h('div', { class: 'callout note ai-noaccess', id: 'ai-noaccess', role: 'status' }, h('p', null, t(key))));
}

export function renderAiChat(m: ChatMode): HTMLElement {
  installOnce();
  mode = m;
  if (!entered) enter();
  const st = aiStatus();
  // Why the page is read only; the reason reads exactly like the header badge's tooltip (the same texts), then what still works
  const reason = st.state === 'ready' ? readOnlyReason(st) : 'unavailable';
  const why = reason === 'noFunction' ? t('ai.err.notAllowed') : badgeReasonText(reason, isAdmin());
  const readOnlyNote = m === 'readOnly' ? h('p', { class: 'callout note ai-readonly' }, why + (uiLang() === 'en' ? ' ' : '') + t('ai.readOnlyTail')) : null;
  const root = h('div', { class: 'page page-ai' + (m === 'readOnly' ? ' ro' : '') },
    h('header', { class: 'page-head ai-head' },
      // Free mode on (administrators, chosen in Settings): a green tag next to the title; nothing in normal mode
      h('div', { class: 'ai-title' }, h('h1', null, t('ai.title')),
        isAdmin() && aiFreeMode() ? h('span', { class: 'ai-mode-tag', role: 'status' }, t('ai.freeTag')) : null),
      tabs()),
    h('p', { class: 'ai-notice' }, miniIcon('info', 'ai-mi'), h('span', null, t('ai.adminNotice'))),
    readOnlyNote,
    tab === 'chats' ? chatsView() : memoryView(),
    h('div', { class: 'sr-only', id: 'ai-sr', role: 'status', 'aria-live': 'polite' }));
  // After the page is in the document: the composer's height, and the conversation at its end
  queueMicrotask(() => {
    if (!root.isConnected || tab !== 'chats') return;
    if (composer) autoGrow(composer.input);
    if (stick) toBottom();
  });
  return root;
}
