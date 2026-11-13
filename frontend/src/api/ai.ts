// The AI service's routes (v0.8, /api/ai/*): AI Chat, AI Memory and the administrators' AI settings. Every request goes
// through Nginx (or the vite server), which checks the session with the main backend first; the AI service never sees the
// session cookie. Errors carry a stable `code` (ApiError.code) that the pages translate.
// Round 2: send / retry / suggest carry the reply language (the interface language of Settings, domain/ai-chat.ts builds the
// bodies), free mode is a state of the administrator (setFreeMode), and administrators suspend / resume a user's AI+
// function.
import { retryBody, suggestBody, type AiLang, type SendBody } from '../domain/ai-chat.ts';
import type { StudyAskBody, ReplyLang } from '../domain/ai-plus.ts';
import { parseSaved, type SavedPayload } from '../domain/ai-study.ts';
import type { StudySavedQuery, StudyTransport } from '../domain/ai-spec.ts';
import type {
  AiAdminUserDetail, AiAdminUserRow, AiChatDetail, AiChatSummary, AiClear, AiLimits, AiLimitsAnswer, AiMemoriesAnswer,
  AiMemory, AiModels, AiSettings, AiSettingsUpdate, AiStatusJson, AiUsage, AiVerify,
} from '../types/api.ts';
import { getJson, postJson, postStream, type StreamOptions } from './client.ts';

/** A poll of a chat whose answer is written elsewhere gives up after this long (it is tried again 2 s later) */
const POLL_TIMEOUT_MS = 10_000;

// ---------------------------------------------------------------- every signed-in user

export const fetchAiStatus = (): Promise<AiStatusJson> => getJson<AiStatusJson>('/api/ai/status', { timeoutMs: 15_000 });

/** The user's chats, the most recently used first */
export const fetchChats = async (): Promise<AiChatSummary[]> => (await getJson<{ chats: AiChatSummary[] }>('/api/ai/chats')).chats;

export const fetchChat = (id: number, poll = false): Promise<AiChatDetail> =>
  getJson<AiChatDetail>(`/api/ai/chat?id=${id}`, poll ? { timeoutMs: POLL_TIMEOUT_MS } : {});

/**
 * Sends a message (chatId null: a new chat; the body from sendBody()); the answer streams in as events (start, thinking?,
 * delta…, done / stopped / error)
 */
export const sendMessage = (body: SendBody, opts: StreamOptions): Promise<void> => postStream('/api/ai/send', body, opts);

/** Writes the failed or stopped last answer of a chat again (streamed like a send; its start event has retry: true) */
export const retryAnswer = (chatId: number, lang: AiLang, opts: StreamOptions): Promise<void> =>
  postStream('/api/ai/retry', retryBody(chatId, lang), opts);

/**
 * Stops the running answer of a chat. Administrators may allow several running answers per user (safety limits), so the
 * chat is named when it is known; without it (a new chat whose start event has not arrived) every running answer of the
 * user stops.
 */
export const stopAnswer = (chatId: number | null = null): Promise<{ ok: boolean; stopped: boolean }> =>
  postJson('/api/ai/stop', chatId != null ? { chatId } : {});

/** Administrators: the free mode (a state of the user: every message in any chat, old or new, is answered in it) */
export const setFreeMode = (free: boolean): Promise<{ free: boolean }> => postJson<{ free: boolean }>('/api/ai/free', { free });

export const deleteChat = (id: number): Promise<{ deleted: number }> => postJson('/api/ai/chats/delete', { id });
export const deleteAllChats = (): Promise<{ deleted: number }> => postJson('/api/ai/chats/delete', { all: true });

export const fetchMemories = (): Promise<AiMemoriesAnswer> => getJson<AiMemoriesAnswer>('/api/ai/memories');

export const addMemory = async (content: string, source: AiMemory['source'] = 'manual'): Promise<AiMemory> =>
  (await postJson<{ memory: AiMemory }>('/api/ai/memories/add', { content, source })).memory;

export const updateMemory = async (id: number, content: string): Promise<AiMemory> =>
  (await postJson<{ memory: AiMemory }>('/api/ai/memories/update', { id, content })).memory;

export const deleteMemory = (id: number): Promise<{ deleted: number }> => postJson('/api/ai/memories/delete', { id });
export const deleteAllMemories = (): Promise<{ deleted: number }> => postJson('/api/ai/memories/delete', { all: true });

/**
 * A note the model proposes for a question that asked to remember something, written in `lang`; nothing is saved until the
 * user saves it
 */
export const suggestMemory = async (messageId: number, lang: AiLang): Promise<string> => {
  const r = await postJson<{ suggestion?: unknown }>('/api/ai/memories/suggest', suggestBody(messageId, lang));
  return typeof r.suggestion === 'string' ? r.suggestion : '';
};

// ---------------------------------------------------------------- v0.9: study answers (hint, explanation, summary)

/**
 * The saved hint / explanation of one question and place (exactly one of `key` / `record` in the query). Every value is a
 * character a browser never percent-encodes, so the query is built by hand.
 */
export const fetchStudySaved = async (q: StudySavedQuery): Promise<SavedPayload> => {
  const who = q.key !== undefined ? `key=${q.key}` : `record=${q.record}`;
  const json = await getJson<unknown>(`/api/ai/study/saved?mode=${q.mode}&${who}&source=${q.source}&question=${q.question}&kind=${q.kind}`);
  return parseSaved(json);
};

/** Asks for a hint / explanation; the answer streams in as events (start, delta…, done / stopped / error) */
export const askStudyStream = (body: StudyAskBody, opts: StreamOptions): Promise<void> => postStream('/api/ai/study/ask', body, opts);

/** The saved summary of a record (`at` = its time as 12 digits, YYYYMMDDHHMM) */
export const fetchSummarySaved = async (q: { record: number; at: string }): Promise<SavedPayload> =>
  parseSaved(await getJson<unknown>(`/api/ai/summary/saved?record=${q.record}&at=${q.at}`));

/** Asks for the summary of a record's wrong answers (streamed like a study answer) */
export const askSummaryStream = (body: { recordId: number; lang: ReplyLang }, opts: StreamOptions): Promise<void> =>
  postStream('/api/ai/summary/ask', body, opts);

/** What the study popover and the summary window use to talk to the AI service (domain/ai-spec.ts) */
export const studyTransport: StudyTransport = {
  savedStudy: fetchStudySaved,
  askStudy: (body, signal, on) => askStudyStream(body, { signal, onEvent: on }),
  savedSummary: fetchSummarySaved,
  askSummary: (body, signal, on) => askSummaryStream(body, { signal, onEvent: on }),
  // always with a chat id, never `{}`; the answer does not matter
  stopChat: async (chatId) => {
    await stopAnswer(chatId);
  },
};

// ---------------------------------------------------------------- administrators

export const fetchAiSettings = (): Promise<AiSettings> => getJson<AiSettings>('/api/ai/admin/settings');
export const saveAiSettings = (u: AiSettingsUpdate): Promise<AiSettings> => postJson<AiSettings>('/api/ai/admin/settings', u);
export const listModels = (): Promise<AiModels> => postJson<AiModels>('/api/ai/admin/models', {});
export const verifyAi = (): Promise<AiVerify> => postJson<AiVerify>('/api/ai/admin/verify', {});
export const enableAi = (enabled: boolean): Promise<AiSettings> => postJson<AiSettings>('/api/ai/admin/enable', { enabled });
export const fetchUsage = (days: number): Promise<AiUsage> => getJson<AiUsage>(`/api/ai/admin/usage?days=${days}`);
/** Every administrator and every user with the AI+ service (suspended ones included), sorted by name */
export const fetchAiUsers = async (): Promise<AiAdminUserRow[]> => (await getJson<{ users: AiAdminUserRow[] }>('/api/ai/admin/users')).users;
/** Round 2: suspends a user's AI+ function (their running answer stops at once; written to the System log) */
export const suspendAiUser = async (userId: number): Promise<AiAdminUserRow> =>
  (await postJson<{ user: AiAdminUserRow }>('/api/ai/admin/suspend', { userId })).user;
/** Round 2: resumes a user's AI+ function (written to the System log) */
export const resumeAiUser = async (userId: number): Promise<AiAdminUserRow> =>
  (await postJson<{ user: AiAdminUserRow }>('/api/ai/admin/resume', { userId })).user;
/** A user's chats and memories (read only; every view is written to the System log) */
export const fetchAiUser = (id: number): Promise<AiAdminUserDetail> => getJson<AiAdminUserDetail>(`/api/ai/admin/user?id=${id}`);
export const fetchAiUserChat = (userId: number, chatId: number): Promise<AiChatDetail> =>
  getJson<AiChatDetail>(`/api/ai/admin/chat?user=${userId}&id=${chatId}`);
export const clearAiData = (c: AiClear): Promise<{ deleted: number }> => postJson('/api/ai/admin/clear', c);
export const fetchLimits = (): Promise<AiLimitsAnswer> => getJson<AiLimitsAnswer>('/api/ai/admin/limits');
export const saveLimits = (l: Partial<AiLimits>): Promise<AiLimitsAnswer> => postJson<AiLimitsAnswer>('/api/ai/admin/limits', l);

/** v1.1 (A-OWNER-21): an administrator switches the AI Chat free mode of an administrator account; answers its row */
export const setAiAdminFree = async (userId: number, free: boolean): Promise<AiAdminUserRow> =>
  (await postJson<{ user: AiAdminUserRow }>('/api/ai/admin/free', { userId, free })).user;

/** v1.1 (API-A2): the six feature switches of a user (all six values every time); answers the user's row */
export const setAiUserFeatures = async (userId: number, features: Record<string, boolean>): Promise<AiAdminUserRow> =>
  (await postJson<{ user: AiAdminUserRow }>('/api/ai/admin/features', { userId, features })).user;
