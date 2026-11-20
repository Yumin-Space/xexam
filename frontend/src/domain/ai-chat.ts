// The conversation shown in AI Chat (v0.8), as pure functions the page and the tests share: the reducer over the events of
// a streamed answer (start, delta, done, stopped, error), the page's own actions (Stop pressed, a polled chat merged in),
// the Enter-key rule of the composer (never send while an input method is composing), and the small facts the page shows
// (which answer can be retried, the status line under an answer, the time of a chat). No DOM, no i18n.
// Round 2: the request bodies (the reply language of Settings with send / retry / suggest, free mode is the server's state),
// the free-mode switch, and the "thinking" event of reasoning models (shown as "Thinking…" until the first text).
// v1.3: the conversation is the PATH of a branch tree (every message may have sibling versions: `versions`), Regenerate and Edit
// add a version, the version switcher moves between them, and administrators pick a model per chat. The model's reasoning
// (frames `reasoning` / `reasoningDone`, fields `reasoning*` of a message) is shown as a collapsible block above the answer:
// `reasoningView` decides whether it is there, live (open while it streams) and open.
import { modelRefText, parseModelRef } from '../api/model-ref.ts';
import type { AiChatDetail, AiChatKind, AiMessage, AiModelList, ModelRef, UiLang } from '../types/api.ts';

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
  /**
   * A send, regenerate or edit that has not started yet: the text stays in the composer until the start event. `prev`: the
   * messages before an optimistic cut (regenerate / edit), restored when the request fails before it starts. `model`: the
   * administrator's pick sent with it (undefined = unchanged, null = back to the default), kept as the chat's pick at the start.
   */
  pending: { text: string; retry: boolean; prev?: ViewMessage[]; model?: ModelRef | null } | null;
  /** v1.3, administrators: the model picked for this chat (null = the default) */
  pick: ModelRef | null;
  /** The server is still writing an answer of this chat */
  running: boolean;
}

export const emptyChat = (): ChatView => ({ chatId: null, title: '', kind: 'chat', messages: [], streamingId: null, pending: null, pick: null, running: false });

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
    pending: null, pick: d.chat.pick ?? null, running: d.chat.running,
  };
}

/** The page sends a message (or retries an answer): remembered until the stream starts */
export function localSend(s: ChatView, text: string, retry = false, model?: ModelRef | null): ChatView {
  return { ...s, pending: model === undefined ? { text, retry } : { text, retry, model } };
}

/** The id of the temporary question an edit shows until the server's start event (never a real message id) */
export const TEMP_ID = -1;

function withPrev(s: ChatView, text: string, retry: boolean, model: ModelRef | null | undefined, messages: ViewMessage[]): ChatView {
  const p = localSend(s, text, retry, model).pending as NonNullable<ChatView['pending']>;
  return { ...s, messages, pending: { ...p, prev: s.messages } };
}

/** Regenerate: the answer and everything after it leave the view at once (the new version streams in its place) */
export function localRegenerate(s: ChatView, assistantId: number, model?: ModelRef | null): ChatView {
  const i = s.messages.findIndex((m) => m.id === assistantId);
  if (i < 0) return localSend(s, '', true, model);
  return withPrev(s, '', true, model, s.messages.slice(0, i));
}

/** Edit: the edited question and everything after it leave the view; the new text stands in until the start event */
export function localEdit(s: ChatView, messageId: number, text: string, model?: ModelRef | null, now = Math.floor(Date.now() / 1000)): ChatView {
  const i = s.messages.findIndex((m) => m.id === messageId);
  if (i < 0) return localSend(s, text, false, model);
  const old = s.messages[i] as ViewMessage;
  const temp: ViewMessage = {
    id: TEMP_ID, role: 'user', content: text, status: 'done', error: null, truncated: false, model: null, createdAt: now, free: old.free,
    parentId: old.parentId ?? null,
  };
  return withPrev(s, text, false, model, [...s.messages.slice(0, i), temp]);
}

/** The request failed before the stream started (busy, refused, offline): nothing changed in the chat */
export function localFail(s: ChatView): ChatView {
  return { ...s, messages: s.pending?.prev ?? s.messages, pending: null };
}

// ---------------------------------------------------------------- the events of a streamed answer

export interface StartData {
  chatId: number;
  chatTitle: string;
  userMessageId: number;
  assistantMessageId: number;
  /** `''` when the server sent none (every non-administrator, v1.3 sprint 2) */
  model: string;
  newChat: boolean;
  retry: boolean;
  /** The mode of this turn (free mode of the administrator) */
  free: boolean;
  /** v0.9: the kind of the chat the answer belongs to (null when the server sent none or an unknown one) */
  chatKind: AiChatKind | null;
  /** v1.3: what the turn is (a frame without it is a send) */
  mode: 'send' | 'regenerate' | 'edit';
  /** v1.3: the parent of the answer (the question), and of the new question (edit; null = a first message) */
  parentId: number | null;
  userParentId: number | null;
  /** v1.3: the sibling versions of the answer / of the new question (2+ only) */
  versions: number[] | undefined;
  userVersions: number[] | undefined;
  /** v1.3, administrators: the connection that answers */
  connection: string | undefined;
  /** v1.3: the chat's picked model was unavailable and the default is used */
  modelFallback: boolean;
}

const obj = (v: unknown): Record<string, unknown> => (typeof v === 'object' && v !== null ? (v as Record<string, unknown>) : {});
const num = (v: unknown): number | null => (typeof v === 'number' && Number.isFinite(v) ? v : null);
const str = (v: unknown): string => (typeof v === 'string' ? v : '');

const KINDS: readonly string[] = ['chat', 'practice', 'exam', 'summary'];
const idList = (v: unknown): number[] | undefined => {
  if (!Array.isArray(v)) return undefined;
  const ids = v.filter((x): x is number => typeof x === 'number' && Number.isFinite(x));
  return ids.length >= 2 ? ids : undefined;
};
const modeOf = (v: unknown): StartData['mode'] => (v === 'regenerate' || v === 'edit' ? v : 'send');
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
    mode: modeOf(d.mode), parentId: num(d.parentId), userParentId: num(d.userParentId), versions: idList(d.versions),
    userVersions: idList(d.userVersions), connection: str(d.connection) || undefined, modelFallback: d.modelFallback === true,
  };
}

/** The messages up to and including `id` (all of them when `id` is not in the list) */
function cutAfter<T extends { id: number }>(list: readonly T[], id: number | null): T[] {
  const i = id == null ? -1 : list.findIndex((m) => m.id === id);
  return i < 0 ? [...list] : list.slice(0, i + 1);
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
        model: d.model || null, createdAt: now, free: d.free, parentId: d.userMessageId || d.parentId,
        ...(d.versions ? { versions: d.versions } : {}), ...(d.connection ? { connection: d.connection } : {}),
      };
      let messages: AiMessage[];
      if (d.retry) {
        // An empty failed answer is reset in place: no new message
        const found = s.messages.some((m) => m.id === d.assistantMessageId);
        messages = found
          ? s.messages.map((m) => (m.id === d.assistantMessageId
            ? { ...m, content: '', status: 'streaming', error: null, truncated: false, model: d.model || m.model, thinking: false, free: d.free,
              reasoning: null, reasoningMs: null, reasoningTruncated: false }
            : m.id === d.userMessageId ? { ...m, free: d.free } : m))
          : [...s.messages, assistant];
      } else if (d.mode === 'regenerate') {
        // A new version of the answer: everything after its question leaves, the new answer follows the question
        messages = [...cutAfter(s.messages, d.parentId).filter((m) => m.id !== d.assistantMessageId), assistant];
      } else {
        const last = s.messages[s.messages.length - 1];
        const base = d.mode === 'edit' ? (d.userParentId == null ? [] : cutAfter(s.messages, d.userParentId)) : s.messages;
        const parent = d.mode === 'edit' ? d.userParentId : (last && last.id > 0 ? last.id : null);
        const user: AiMessage = {
          id: d.userMessageId, role: 'user', content: s.pending?.text ?? '', status: 'done', error: null, truncated: false, model: null, createdAt: now,
          free: d.free, parentId: parent, ...(d.userVersions ? { versions: d.userVersions } : {}),
        };
        messages = [...base.filter((m) => m.id !== d.userMessageId && m.id !== d.assistantMessageId && m.id !== TEMP_ID), user, assistant];
      }
      const picked = s.pending?.model;
      return {
        chatId: d.chatId, title: d.chatTitle || s.title, kind: d.chatKind ?? s.kind, messages, streamingId: d.assistantMessageId, pending: null,
        pick: d.modelFallback ? null : picked !== undefined ? picked : s.pick, running: true,
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
    case 'reasoning': {
      // The model's reasoning text, before the first answer text (a late one is ignored)
      const text = str(obj(ev.data).text);
      const id = s.streamingId;
      if (id == null || !text) return s;
      const m = s.messages.find((x) => x.id === id);
      if (!m || m.status !== 'streaming' || m.content !== '') return s;
      return { ...s, messages: s.messages.map((x) => (x.id === id ? { ...x, reasoning: (x.reasoning ?? '') + text } : x)) };
    }
    case 'reasoningDone': {
      // The reasoning ended (the answer starts or the stream ends): its duration is known
      const id = s.streamingId;
      if (id == null) return s;
      const d = obj(ev.data);
      const ms = typeof d.ms === 'number' && Number.isFinite(d.ms) && d.ms >= 0 ? d.ms : undefined;
      const truncated = d.truncated === true;
      return { ...s, messages: s.messages.map((x) => (x.id === id ? { ...x, ...(ms !== undefined ? { reasoningMs: ms } : {}), reasoningTruncated: truncated } : x)) };
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
    pending: s.pending, pick: d.chat.pick !== undefined ? d.chat.pick : s.pick, running: d.chat.running,
  };
}

/**
 * Whether an answer can get a new version: an AI Chat answer that is not being written, right under an AI Chat question, while
 * nothing is running or about to start (one answer per chat at a time).
 */
export function canRegenerate(s: ChatView, m: ViewMessage): boolean {
  if (s.pending || s.streamingId != null || s.running) return false;
  if (m.role !== 'assistant' || (m.kind ?? 'chat') !== 'chat' || m.status === 'streaming' || m.id <= 0) return false;
  const q = questionOf(s, m.id);
  return q != null && (q.kind ?? 'chat') === 'chat';
}

/** Whether a question can be edited and sent again as a new branch: an AI Chat question while nothing is running */
export function canEdit(s: ChatView, m: ViewMessage): boolean {
  if (s.pending || s.streamingId != null || s.running) return false;
  return m.role === 'user' && (m.kind ?? 'chat') === 'chat' && m.id > 0;
}

/** The text of the Regenerate button: a failed or stopped LAST answer keeps the old name "Retry", every other answer says Regenerate */
export function regenerateLabelKey(s: Pick<ChatView, 'messages'>, m: Pick<ViewMessage, 'id' | 'status'>): 'ai.retry' | 'ai.regenerate' {
  const last = s.messages[s.messages.length - 1];
  return last?.id === m.id && (m.status === 'error' || m.status === 'stopped') ? 'ai.retry' : 'ai.regenerate';
}

/** The question an answer replies to: its parent when the server names one, else (an older answer) the message right before it */
export function questionOf(s: Pick<ChatView, 'messages'>, assistantId: number): AiMessage | null {
  const i = s.messages.findIndex((m) => m.id === assistantId);
  if (i < 0) return null;
  const a = s.messages[i] as AiMessage;
  if (a.role !== 'assistant') return null;
  if (a.parentId != null) {
    const p = s.messages.find((m) => m.id === a.parentId);
    return p && p.role === 'user' ? p : null;
  }
  if (a.parentId === null) return null;
  const q = i > 0 ? s.messages[i - 1] : undefined;
  return q && q.role === 'user' ? q : null;
}

// ---------------------------------------------------------------- versions (v1.3)

export interface VersionInfo {
  /** 1-based position and the number of versions */
  index: number;
  total: number;
  /** The neighbouring version ids (null at the ends) */
  prev: number | null;
  next: number | null;
}

/** The switcher `<  n/total  >` of a message with 2+ versions, or null */
export function versionInfo(m: Pick<AiMessage, 'id' | 'versions'>): VersionInfo | null {
  const v = m.versions;
  if (!v || v.length < 2) return null;
  const i = v.indexOf(m.id);
  if (i < 0) return null;
  return { index: i + 1, total: v.length, prev: i > 0 ? (v[i - 1] as number) : null, next: i < v.length - 1 ? (v[i + 1] as number) : null };
}

// ---------------------------------------------------------------- editing a question (v1.3)

/** The question being edited in the composer, and the draft the composer held before (restored on cancel) */
export interface EditState {
  messageId: number;
  draft: string;
}

/** Starts editing `m`: null when it cannot be edited now */
export function beginEdit(s: ChatView, m: ViewMessage, composerText: string): EditState | null {
  return canEdit(s, m) ? { messageId: m.id, draft: composerText } : null;
}

/** The edit still makes sense: its question is on the path (a poll or another tab may have changed the chat) */
export function editValid(s: ChatView, e: EditState): boolean {
  const m = s.messages.find((x) => x.id === e.messageId);
  return m != null && m.role === 'user' && (m.kind ?? 'chat') === 'chat' && m.id > 0;
}

// ---------------------------------------------------------------- the administrators' model picker (v1.3, v1.9)

/** A model of the picker's second list; `value` is `<connectionId>:<model>`; `isDefault` marks the AI Chat default model */
export interface ChatModelOpt { value: string; label: string; isDefault: boolean }
/** A connection of the picker's first list (listed = enabled, working and with at least one usable model) */
export interface ChatConnOpt { id: number; name: string; models: ChatModelOpt[] }
/** The AI Chat default as the status gives it */
export type ChatDefault = { connectionId: number; connectionName: string; model: string } | null;
/**
 * What the picker offers. `loaded` false = the model list is not here yet (the pick is kept, nothing is offered);
 * `off` = the chat's pick sits on a connection that is switched off (`name` null when the list does not name it).
 */
export interface ChatMenu { loaded: boolean; conns: ChatConnOpt[]; off: { name: string | null } | null }

/**
 * The picker's lists. A listed connection is enabled (`enabled !== false`), working (`status` 'ok') and has at least one model
 * that did not fail its test; the models come in the listing's order (pool models and catalogue models alike) and the one
 * equal to `def` is marked. A stored pick that its listed connection no longer offers (a failed or unlisted model) comes first
 * in that connection's models, so the chat's current model is always visible. A pick on a connection that is present but off
 * is reported in `off`; a pick on a broken or removed connection is neither listed nor `off` (the server falls back to the
 * default, and the picker shows the default). While the list is not loaded (`null`) the pick is kept.
 */
export function chatMenu(list: AiModelList | null, pick: ModelRef | null, def: ChatDefault): ChatMenu {
  if (!list) return { loaded: false, conns: [], off: null };
  const defText = def ? modelRefText({ connectionId: def.connectionId, model: def.model }) : null;
  const conns: ChatConnOpt[] = [];
  let off: ChatMenu['off'] = null;
  for (const c of list.connections) {
    if (c.enabled === false) {
      if (pick?.connectionId === c.id) off = { name: c.name };
      continue;
    }
    if (c.status !== 'ok') continue;
    const models: ChatModelOpt[] = c.models.filter((m) => m.status !== 'failed').map((m) => {
      const value = modelRefText({ connectionId: c.id, model: m.id });
      return { value, label: m.id, isDefault: value === defText };
    });
    if (!models.length) continue;
    if (pick?.connectionId === c.id && !models.some((m) => m.label === pick.model)) {
      const value = modelRefText(pick);
      models.unshift({ value, label: pick.model, isDefault: value === defText });
    }
    conns.push({ id: c.id, name: c.name, models });
  }
  return { loaded: true, conns, off };
}

/**
 * The value the picker shows for `selected` (the unsent choice, at first the stored pick): itself while the menu offers it
 * (or keeps it: list not loaded, or the pick is on an off connection), else '' (the default). A stored pick on a broken or
 * removed connection shows as the default; the request still carries "unchanged" (`modelArg`), so the server falls back.
 */
export function chatShown(selected: string, menu: ChatMenu): string {
  if (selected === '' || !menu.loaded || menu.off) return selected;
  return menu.conns.some((c) => c.models.some((m) => m.value === selected)) ? selected : '';
}

/** What the two lists show: the connection (null = the placeholder) and the model value ('' = the placeholder) */
export interface PickerView { connId: number | null; modelValue: string }

/**
 * Pending connection (only the first list was changed): that connection and no model. Else a chosen model: its connection
 * and value. Else the AI Chat default while its connection is listed. Else nothing (both placeholders).
 */
export function pickerView(selected: string, pendingConn: number | null, menu: ChatMenu, def: ChatDefault): PickerView {
  if (pendingConn != null) return { connId: pendingConn, modelValue: '' };
  if (selected !== '') {
    const ref = parseModelRef(selected);
    return ref ? { connId: ref.connectionId, modelValue: selected } : { connId: null, modelValue: '' };
  }
  if (def && menu.conns.some((c) => c.id === def.connectionId)) {
    return { connId: def.connectionId, modelValue: modelRefText({ connectionId: def.connectionId, model: def.model }) };
  }
  return { connId: null, modelValue: '' };
}

/** What a chosen model value means for the chat: the default model means "follow the default" ('') */
export function chooseValue(value: string, def: ChatDefault): string {
  return def && value === modelRefText({ connectionId: def.connectionId, model: def.model }) ? '' : value;
}

/** The model a request carries: undefined = nothing changed (the server uses the stored pick), null = back to the default */
export function modelArg(selected: string, stored: ModelRef | null, parse: (text: string) => ModelRef | null): ModelRef | null | undefined {
  const was = stored ? modelRefText(stored) : '';
  if (selected === was) return undefined;
  if (selected === '') return null;
  return parse(selected) ?? undefined;
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

/** v1.4 (owner): AI Chat names the model only where the model can be chosen, i.e. for an administrator with the free mode
 *  on (Settings → AI chat mode); in the normal mode, and for everyone else, an answer is just "AI". The administrators'
 *  chat viewer in AI settings always shows it. */
export const chatShowsModel = (admin: boolean, freeMode: boolean): boolean => admin && freeMode;

/** v1.3 sprint 2 (owner): what the line above a message names. A question: "You". An answer: "connection · model" when
 *  `show` is true and the server sent a model (the connection when known); otherwise the neutral "AI" mark. The server never
 *  sends a model or connection to a non-administrator; `show` also keeps stale data from showing one. */
export interface WhoParts { mark: 'you' | 'ai' | null; connection: string | null; model: string | null }
export function whoParts(m: Pick<AiMessage, 'role' | 'model' | 'connection'>, show: boolean): WhoParts {
  if (m.role === 'user') return { mark: 'you', connection: null, model: null };
  if (show && typeof m.model === 'string' && m.model !== '') return { mark: null, connection: m.connection || null, model: m.model };
  return { mark: 'ai', connection: null, model: null };
}

export function footerOf(m: Pick<ViewMessage, 'role' | 'status' | 'content' | 'error' | 'truncated' | 'thinking' | 'reasoning'>): Footer {
  if (m.role !== 'assistant') return null;
  switch (m.status) {
    case 'streaming':
      // While reasoning text streams, the block's own header says "Thinking…"
      if (m.content === '' && (m.reasoning ?? '').trim() !== '') return null;
      return m.thinking && m.content === '' ? { kind: 'thinking' } : { kind: 'writing' };
    case 'stopped':
      return { kind: 'stopped' };
    case 'error':
      return m.content.trim() ? { kind: 'error', code: m.error || 'ai.internal' } : { kind: 'noAnswer', code: m.error || 'ai.internal' };
    default:
      return m.truncated ? { kind: 'truncated' } : null;
  }
}

// ---------------------------------------------------------------- the reasoning block (v1.3)

export interface ReasoningView { text: string; live: boolean; open: boolean; seconds: number | null; truncated: boolean }

/** Seconds shown in "Thought for N s": rounded, at least 1 */
export const reasoningSeconds = (ms: number): number => Math.max(1, Math.round(ms / 1000));

/** The reasoning block of a message, or null (no block). `userOpen`: the reader's own toggle (undefined = none). Live =
 *  the answer is being written, has no text yet and its reasoning has not ended: open unless the reader closed it;
 *  otherwise closed unless the reader opened it. `text` is trimStart-ed while live, trimmed otherwise. */
export function reasoningView(
  m: Pick<ViewMessage, 'role' | 'status' | 'content' | 'reasoning' | 'reasoningMs' | 'reasoningTruncated'>, userOpen?: boolean,
): ReasoningView | null {
  if (m.role !== 'assistant') return null;
  const raw = m.reasoning ?? '';
  if (raw.trim() === '') return null;
  const live = m.status === 'streaming' && m.content === '' && m.reasoningMs == null;
  return {
    text: live ? raw.trimStart() : raw.trim(), live, open: live ? userOpen !== false : userOpen === true,
    seconds: m.reasoningMs == null ? null : reasoningSeconds(m.reasoningMs), truncated: m.reasoningTruncated === true,
  };
}

// ---------------------------------------------------------------- the request bodies and free mode (round 2)

/** The reply language: the interface language of Settings (the AI answers in it unless the message asks otherwise) */
export type AiLang = UiLang;

export interface SendBody {
  chatId: number | null;
  text: string;
  lang: AiLang;
  /** v1.3, administrators: the picked model (null = back to the default); absent = the stored pick */
  model?: ModelRef | null;
}

/**
 * The body of `POST /api/ai/send`: the chat (null starts one), the text and the reply language. Free mode is a state of the
 * administrator on the server (`POST /api/ai/free`), so the body never carries it.
 */
export function sendBody(o: { chatId: number | null; text: string; lang: AiLang; model?: ModelRef | null }): SendBody {
  return o.model === undefined ? { chatId: o.chatId, text: o.text, lang: o.lang } : { chatId: o.chatId, text: o.text, lang: o.lang, model: o.model };
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

/** Why AI Chat is read only: the AI+ function paused, the `chat` feature unticked (v1.1), the account without it, or the AI+ feature not available (off / not connected) */
export function readOnlyReason(
  st: { enabled: boolean; allowed: boolean; suspended: boolean; why?: 'off' | 'link' | null; features?: { chat?: boolean } },
): 'suspended' | 'featureOff' | 'noFunction' | 'off' | 'down' | 'unavailable' {
  if (st.suspended) return 'suspended';
  if (st.enabled && st.allowed && st.features?.chat === false) return 'featureOff';
  if (st.enabled && !st.allowed) return 'noFunction';
  // v1.1 (BT-2): the cause of "not available" when the status names it
  return st.why === 'off' ? 'off' : st.why === 'link' ? 'down' : 'unavailable';
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
