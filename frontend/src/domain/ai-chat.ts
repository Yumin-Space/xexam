// The conversation shown in AI Chat (v0.8), as pure functions the page and the tests share: the reducer over the events of
// a streamed answer (start, delta, done, stopped, error), the page's own actions (Stop pressed, a polled chat merged in),
// the Enter-key rule of the composer (never send while an input method is composing), and the small facts the page shows
// (which answer can be retried, the status line under an answer, the time of a chat). No DOM, no i18n.
// Round 2: the request bodies (the reply language of Settings with send / retry / suggest, free mode is the server's state),
// the free-mode switch, and the "thinking" event of reasoning models (shown as "Thinking…" until the first text).
import type { AiChatDetail, AiChatKind, AiMessage, UiLang } from '../types/api.ts';

/** A message as the page shows it: `thinking` (view only, never from the server) = a reasoning model thinks, no text yet */
export type ViewMessage = AiMessage & { thinking?: boolean };

export interface ChatView {
  /** null: a new chat, created by its first message (the start event gives its id) */
  chatId: number | null;
  title: string;
  /** v0.9: 'chat' (AI Chat) or the kind of a study chat; decides the title suffix */
  kind: AiChatKind;
  messages: ViewMessage[];
  /** The assistant message being written (by this page's stream, or seen running by a poll); null = none */
  streamingId: number | null;
  /** A send or retry that has not started yet: the text stays in the composer until the start event */
  pending: { text: string; retry: boolean } | null;
  /** The server is still writing an answer of this chat */
  running: boolean;
}

export const emptyChat = (): ChatView => ({ chatId: null, title: '', kind: 'chat', messages: [], streamingId: null, pending: null, running: false });

function lastWhere<T>(list: readonly T[], test: (v: T) => boolean): T | undefined {
  for (let i = list.length - 1; i >= 0; i--) {
    const v = list[i] as T;
    if (test(v)) return v;
  }
  return undefined;
}

/** A chat as read from the server (opening it, or a poll) */
export function chatOf(d: AiChatDetail): ChatView {
  const messages = d.messages.map((m) => ({ ...m }));
  const writing = d.chat.running ? lastWhere(messages, (m) => m.status === 'streaming') : undefined;
  return {
    chatId: d.chat.id, title: d.chat.title, kind: d.chat.kind ?? 'chat', messages, streamingId: writing?.id ?? null,
    pending: null, running: d.chat.running,
  };
}

/** The page sends a message (or retries an answer): remembered until the stream starts */
export function localSend(s: ChatView, text: string, retry = false): ChatView {
  return { ...s, pending: { text, retry } };
}

/** The request failed before the stream started (busy, refused, offline): nothing changed in the chat */
export function localFail(s: ChatView): ChatView {
  return { ...s, pending: null };
}

// ---------------------------------------------------------------- the events of a streamed answer

export interface StartData {
  chatId: number;
  chatTitle: string;
  userMessageId: number;
  assistantMessageId: number;
  model: string;
  newChat: boolean;
  retry: boolean;
  /** The mode of this turn (free mode of the administrator) */
  free: boolean;
  /** v0.9: the kind of the chat the answer belongs to (null when the server sent none or an unknown one) */
  chatKind: AiChatKind | null;
}

const obj = (v: unknown): Record<string, unknown> => (typeof v === 'object' && v !== null ? (v as Record<string, unknown>) : {});
const num = (v: unknown): number | null => (typeof v === 'number' && Number.isFinite(v) ? v : null);
const str = (v: unknown): string => (typeof v === 'string' ? v : '');

const KINDS: readonly string[] = ['chat', 'practice', 'exam', 'summary'];
const chatKindOf = (v: unknown): AiChatKind | null => (typeof v === 'string' && KINDS.includes(v) ? (v as AiChatKind) : null);

/** The start event's data, or null when it is not one */
export function startOf(data: unknown): StartData | null {
  const d = obj(data);
  const chatId = num(d.chatId);
  const userMessageId = num(d.userMessageId);
  const assistantMessageId = num(d.assistantMessageId);
  if (chatId == null || assistantMessageId == null) return null;
  return {
    chatId, chatTitle: str(d.chatTitle), userMessageId: userMessageId ?? 0, assistantMessageId, model: str(d.model),
    newChat: d.newChat === true, retry: d.retry === true, free: d.free === true, chatKind: chatKindOf(d.chatKind),
  };
}

function finish(s: ChatView, patch: Partial<AiMessage>): ChatView {
  const id = s.streamingId;
  if (id == null) return { ...s, running: false };
  return {
    ...s,
    messages: s.messages.map((m) => (m.id === id && m.status === 'streaming' ? { ...m, ...patch, thinking: false } : m)),
    streamingId: null,
    running: false,
  };
}

/** One event of the stream applied to the chat. `now`: unix seconds for the new messages. Unknown events change nothing. */
export function applyEvent(s: ChatView, ev: { event: string; data: unknown }, now = Math.floor(Date.now() / 1000)): ChatView {
  switch (ev.event) {
    case 'start': {
      const d = startOf(ev.data);
      if (!d) return s;
      const assistant: AiMessage = {
        id: d.assistantMessageId, role: 'assistant', content: '', status: 'streaming', error: null, truncated: false,
        model: d.model || null, createdAt: now, free: d.free,
      };
      let messages: AiMessage[];
      if (d.retry) {
        // Retry resets the failed answer in place: no new message
        const found = s.messages.some((m) => m.id === d.assistantMessageId);
        messages = found
          ? s.messages.map((m) => (m.id === d.assistantMessageId
            ? { ...m, content: '', status: 'streaming', error: null, truncated: false, model: d.model || m.model, thinking: false, free: d.free }
            : m.id === d.userMessageId ? { ...m, free: d.free } : m))
          : [...s.messages, assistant];
      } else {
        const user: AiMessage = {
          id: d.userMessageId, role: 'user', content: s.pending?.text ?? '', status: 'done', error: null, truncated: false, model: null, createdAt: now, free: d.free,
        };
        messages = [...s.messages.filter((m) => m.id !== d.userMessageId && m.id !== d.assistantMessageId), user, assistant];
      }
      return {
        chatId: d.chatId, title: d.chatTitle || s.title, kind: d.chatKind ?? s.kind, messages, streamingId: d.assistantMessageId, pending: null,
        running: true,
      };
    }
    case 'thinking': {
      // A reasoning model thinks before it writes: "Thinking…" until the first text (a late one, after text, is ignored)
      const id = s.streamingId;
      if (id == null) return s;
      const m = s.messages.find((x) => x.id === id);
      if (!m || m.status !== 'streaming' || m.content !== '' || m.thinking) return s;
      return { ...s, messages: s.messages.map((x) => (x.id === id ? { ...x, thinking: true } : x)) };
    }
    case 'delta': {
      const text = str(obj(ev.data).text);
      const id = s.streamingId;
      if (id == null || !text) return s;
      return { ...s, messages: s.messages.map((m) => (m.id === id ? { ...m, content: m.content + text, thinking: false } : m)) };
    }
    case 'done':
      return finish(s, { status: 'done', truncated: obj(ev.data).truncated === true });
    case 'stopped':
      return finish(s, { status: 'stopped' });
    case 'error':
      return finish(s, { status: 'error', error: str(obj(ev.data).code) || 'ai.internal' });
    default:
      return s;
  }
}

/** Whether an event ends the answer (exactly one of them ends every stream) */
export const isTerminal = (event: string): boolean => event === 'done' || event === 'stopped' || event === 'error';

/**
 * Whether an event empties the composer: only the start of a sent message, whose text came from the composer. The start
 * of a retry never does (its question is already in the chat), so a draft typed while the answer failed or was stopped
 * stays. `before`: the chat before the event.
 */
export function clearsComposer(before: Pick<ChatView, 'pending'>, ev: { event: string; data: unknown }): boolean {
  if (ev.event !== 'start') return false;
  const d = startOf(ev.data);
  return d != null && !d.retry && before.pending?.retry !== true;
}

/** How long after Send turned into Stop a click on it is taken as part of the click that sent */
export const STOP_GUARD_MS = 400;

/**
 * Whether a click on the Send / Stop button (one element) is ignored: the second click of a double-click never does a second
 * thing, and a click shortly after Send turned into Stop does not stop the answer it just started. `detail`: the click
 * count of the event (0 for Enter / Space on the focused button); `stop`: the button is Stop now; `sinceStopMs`: the time
 * since it became Stop.
 */
export function ignoreSendClick(detail: number, stop: boolean, sinceStopMs: number): boolean {
  if (detail > 1) return true;
  return stop && sinceStopMs < STOP_GUARD_MS;
}

/** Stop was pressed: the answer being written shows "Stopped" at once (the server stops it too) */
export function localStop(s: ChatView): ChatView {
  if (s.streamingId == null) return { ...s, pending: null, running: false };
  return { ...finish(s, { status: 'stopped' }), pending: null };
}

/**
 * A chat read again while its answer is written elsewhere (the stream was lost, the page came back, another tab): the
 * server's messages replace the page's, except that an answer stopped here stays stopped while the server still finishes it.
 * A poll of another chat (the user switched meanwhile) changes nothing.
 */
export function pollResult(s: ChatView, d: AiChatDetail): ChatView {
  if (s.chatId != null && d.chat.id !== s.chatId) return s;
  const stoppedHere = new Set(s.messages.filter((m) => m.status === 'stopped').map((m) => m.id));
  const messages = d.messages.map((m) => (m.status === 'streaming' && stoppedHere.has(m.id) ? { ...m, status: 'stopped' as const } : { ...m }));
  const writing = d.chat.running ? lastWhere(messages, (m) => m.status === 'streaming') : undefined;
  return {
    chatId: d.chat.id, title: d.chat.title, kind: d.chat.kind ?? 'chat', messages, streamingId: writing?.id ?? null,
    pending: s.pending, running: d.chat.running,
  };
}

/** The answer that Retry would write again: the last message, an assistant answer that failed or was stopped, after a question */
export function retryTarget(s: ChatView): number | null {
  if (s.pending || s.streamingId != null || s.running) return null;
  const n = s.messages.length;
  const last = s.messages[n - 1];
  const before = s.messages[n - 2];
  if (!last || !before || last.role !== 'assistant' || before.role !== 'user') return null;
  // v0.9: only an AI Chat answer is retried; the answers of hints, explanations and summaries are asked again from their windows
  if ((last.kind ?? 'chat') !== 'chat' || (before.kind ?? 'chat') !== 'chat') return null;
  return last.status === 'error' || last.status === 'stopped' ? last.id : null;
}

/** The question an answer replies to (the user message right before it), or null */
export function questionOf(s: Pick<ChatView, 'messages'>, assistantId: number): AiMessage | null {
  const i = s.messages.findIndex((m) => m.id === assistantId);
  const q = i > 0 ? s.messages[i - 1] : undefined;
  return q && q.role === 'user' ? q : null;
}

/** The line under an answer: thinking (a reasoning model, no text yet), writing, stopped, interrupted (with text) / no answer
 *  (without), cut at the length limit */
export type Footer =
  | { kind: 'thinking' }
  | { kind: 'writing' }
  | { kind: 'stopped' }
  | { kind: 'error'; code: string }
  | { kind: 'noAnswer'; code: string }
  | { kind: 'truncated' }
  | null;

export function footerOf(m: Pick<ViewMessage, 'role' | 'status' | 'content' | 'error' | 'truncated' | 'thinking'>): Footer {
  if (m.role !== 'assistant') return null;
  switch (m.status) {
    case 'streaming':
      return m.thinking && m.content === '' ? { kind: 'thinking' } : { kind: 'writing' };
    case 'stopped':
      return { kind: 'stopped' };
    case 'error':
      return m.content.trim() ? { kind: 'error', code: m.error || 'ai.internal' } : { kind: 'noAnswer', code: m.error || 'ai.internal' };
    default:
      return m.truncated ? { kind: 'truncated' } : null;
  }
}

// ---------------------------------------------------------------- the request bodies and free mode (round 2)

/** The reply language: the interface language of Settings (the AI answers in it unless the message asks otherwise) */
export type AiLang = UiLang;

export interface SendBody {
  chatId: number | null;
  text: string;
  lang: AiLang;
}

/**
 * The body of `POST /api/ai/send`: the chat (null starts one), the text and the reply language. Free mode is a state of the
 * administrator on the server (`POST /api/ai/free`), so the body never carries it.
 */
export function sendBody(o: { chatId: number | null; text: string; lang: AiLang }): SendBody {
  return { chatId: o.chatId, text: o.text, lang: o.lang };
}

/** The body of `POST /api/ai/retry` */
export const retryBody = (chatId: number, lang: AiLang): { chatId: number; lang: AiLang } => ({ chatId, lang });

/** The body of `POST /api/ai/memories/suggest` (the note is written in the language of Settings) */
export const suggestBody = (messageId: number, lang: AiLang): { messageId: number; lang: AiLang } => ({ messageId, lang });

// ---------------------------------------------------------------- the composer's Enter key

export interface KeyLike {
  key: string;
  shiftKey: boolean;
  ctrlKey: boolean;
  metaKey: boolean;
  altKey: boolean;
  isComposing: boolean;
  keyCode: number;
}

export interface ComposerKeys {
  /** Set on compositionstart, cleared one tick after compositionend (Safari fires compositionend before the committing Enter) */
  composing: boolean;
  /** `matchMedia('(pointer: fine)')`: with a mouse / trackpad, Enter sends; on a touch screen Enter is a new line */
  finePointer: boolean;
}

/**
 * Whether a keydown in the composer sends the message. Never while an input method is composing (the Enter that commits a
 * Chinese or Japanese word must not send). Ctrl / Cmd + Enter always sends; Enter alone sends with a fine pointer; Shift +
 * Enter (and Enter on a touch screen) is a new line.
 */
export function shouldSend(e: KeyLike, s: ComposerKeys): boolean {
  if (e.key !== 'Enter') return false;
  if (e.isComposing || e.keyCode === 229 || s.composing) return false;
  if (e.ctrlKey || e.metaKey) return true;
  if (e.shiftKey || e.altKey) return false;
  return s.finePointer;
}

// ---------------------------------------------------------------- times

/**
 * The time of a chat or an event (unix seconds) for a list: the time only when it is today, else the date and the time,
 * in the interface language (the same formatter is used for "last successful test" and "last used").
 */
export function fmtAiTime(sec: number, locale: string, nowMs = Date.now()): string {
  const d = new Date(sec * 1000);
  const now = new Date(nowMs);
  const today = d.getFullYear() === now.getFullYear() && d.getMonth() === now.getMonth() && d.getDate() === now.getDate();
  const opts: Intl.DateTimeFormatOptions = today
    ? { hour: '2-digit', minute: '2-digit' }
    : { month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit' };
  try {
    return new Intl.DateTimeFormat(locale, opts).format(d);
  } catch {
    return d.toISOString().slice(0, 16).replace('T', ' ');
  }
}

// ---------------------------------------------------------------- small facts of the page (UX review)

/** The answer being written is a reasoning model still thinking (no text yet): the list row says "Thinking…" too */
export function isThinking(s: ChatView): boolean {
  if (s.streamingId == null) return false;
  const m = s.messages.find((x) => x.id === s.streamingId);
  return m?.thinking === true && m.content === '';
}

/** A new chat with nothing in it (no message, nothing being sent, no answer running): the page shows its short hint */
export const showsEmptyHint = (s: ChatView): boolean => s.messages.length === 0 && s.pending == null && !s.running;

/**
 * The text of #/ai for a user who cannot use AI and has no data to read: the AI+ function paused by an administrator, the
 * AI+ feature off for everyone (or not reachable), or the account without the function. Never names the ai-service container.
 */
export function noAccessKey(st: { enabled: boolean; suspended: boolean }): 'ai.noAccess.suspended' | 'ai.noAccess.disabled' | 'ai.noAccess.noFn' {
  if (st.suspended) return 'ai.noAccess.suspended';
  return st.enabled ? 'ai.noAccess.noFn' : 'ai.noAccess.disabled';
}

/** Why AI Chat is read only: the AI+ function paused, the account without it, or the AI+ feature not available (off / not connected) */
export function readOnlyReason(st: { enabled: boolean; allowed: boolean; suspended: boolean }): 'suspended' | 'noFunction' | 'unavailable' {
  if (st.suspended) return 'suspended';
  return st.enabled && !st.allowed ? 'noFunction' : 'unavailable';
}

// ---------------------------------------------------------------- the phone screen (v1.0: CH-1, CH-9, CH-13)

/** AI Chat is a full-height screen with a locked document up to this width (px); wider, the owner's fixed-height box */
export const LOCK_MAX_WIDTH = 900;

/** From this width (px) the chat list and the conversation sit side by side again (tablets, CH-10) */
export const TWO_PANES_MIN_WIDTH = 720;

/** Within this many pixels of the end the list counts as "at the end" and follows new text and a changed height */
export const END_PX = 24;

/** Whether a scroller is at its end (within `px`): the list then keeps showing the newest message when its height changes */
export function atEnd(box: { scrollHeight: number; scrollTop: number; clientHeight: number }, px = END_PX): boolean {
  return box.scrollHeight - box.scrollTop - box.clientHeight <= px;
}

/** Whether the AI Chat page locks the document (no page scroll; only the lists scroll) at this window width */
export const locksDocument = (width: number): boolean => width <= LOCK_MAX_WIDTH;

/** Whether the chat list and the conversation are both shown at this window width (otherwise one at a time) */
export const showsTwoPanes = (width: number): boolean => width >= TWO_PANES_MIN_WIDTH;

// ---------------------------------------------------------------- the composer (v1.0: CH-7, CH-8, C-6)

/** What the status slot above the composer's input shows (one thing at a time, never more than two lines) */
export type ComposerStatus = 'error' | 'cap' | 'busy' | null;

/**
 * The status slot: a send error (red) first, then the request-cap callout (warn), then "another chat is writing an answer"
 * (with its Stop it button). A cap refusal is shown as the callout, never also as a send error.
 */
export function composerStatus(o: { sendError: string | null; capped: boolean; busyElsewhere: boolean }): ComposerStatus {
  if (o.sendError) return 'error';
  if (o.capped) return 'cap';
  return o.busyElsewhere ? 'busy' : null;
}

/** An answer is being written in another chat of this user (a study answer counts too), not in the open one */
export function isBusyElsewhere(running: { chatId: number } | null | undefined, openChatId: number | null, sendingHere: boolean): boolean {
  return running != null && !sendingHere && running.chatId !== openChatId;
}

/** How many lines the composer's input grows to before it scrolls: 4 on touch screens or in a window under 700 px tall, else 8 */
export const composerMaxLines = (coarsePointer: boolean, windowHeight: number): number => (coarsePointer || windowHeight < 700 ? 4 : 8);

/** The full-height chat screen keeps at least this much of the conversation (px) visible while the input grows */
export const MIN_MSGS_PX = 100;

/**
 * How many lines the input may grow to given the room left: at most `composerMaxLines`, never more than about 22% of the
 * visual viewport, and never more than `textRoomPx` (the px its text may take so that MIN_MSGS_PX of the conversation stay
 * visible above the composer: the open keyboard, a short landscape screen); at least 1.
 */
export function composerLines(coarsePointer: boolean, windowHeight: number, viewportHeight: number, lineHeight: number,
  textRoomPx = Number.POSITIVE_INFINITY): number {
  const lh = Math.max(1, lineHeight);
  const cap = composerMaxLines(coarsePointer, windowHeight);
  const room = Math.floor((0.22 * viewportHeight) / lh);
  const fit = Number.isFinite(textRoomPx) ? Math.floor(textRoomPx / lh) : cap;
  return Math.max(1, Math.min(cap, room, fit));
}
