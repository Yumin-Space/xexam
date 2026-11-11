// The seam between the study engine (ai-run.ts) and the API layer (v0.9): the transport interface the API wrappers of
// api/ai.ts implement, and the builders that turn one popover / overlay request into a `RunSpec`. No DOM, no fetch.
import type { RunSpec } from './ai-run.ts';
import type { ReplyLang, StudyAskBody, StudyKind, StudySource } from './ai-plus.ts';
import { atDigits, buildSummaryAsk, type UiLangCode } from './ai-plus.ts';
import type { SavedPayload, StreamEvent } from './ai-study.ts';

/**
 * The query of GET /api/ai/study/saved (every value is a character a browser never percent-encodes, spec 5.3). Exactly one
 * of `key` / `record` is present (coordinator answer A7: both or neither is a 400).
 */
export type StudySavedQuery = {
  mode: 'practice' | 'exam';
  source: StudySource;
  question: string;
  kind: StudyKind;
} & ({ key: string; record?: undefined } | { record: number; key?: undefined });

/**
 * The saved query of a page: the practice page and the result pages ask by session `key`; the record detail page ALWAYS asks
 * by `record` (A7: one code path, the server maps the record to its chat). null = nothing to look up (no key, no record).
 */
export function studySavedQuery(i: {
  page: 'practice' | 'result' | 'detail';
  mode: 'practice' | 'exam';
  source: StudySource;
  kind: StudyKind;
  questionId: string;
  key: string | null;
  recordId: number | null;
}): StudySavedQuery | null {
  const base = { mode: i.mode, source: i.source, question: i.questionId, kind: i.kind };
  if (i.page === 'detail') return i.recordId != null ? { ...base, record: i.recordId } : null;
  return i.key != null ? { ...base, key: i.key } : null;
}

/**
 * What the API layer provides. A failed request must reject with an object that has `status` (0 = no HTTP answer), `code`
 * and, for a 409 ai.busy, `running` (the frontend's ApiError with `running` kept from the body does; see asAskError). An
 * aborted request rejects with an AbortError (ignored by the engine).
 */
export interface StudyTransport {
  /** GET /api/ai/study/saved?mode=&key=&source=&question=&kind= -> parseSaved(json) */
  savedStudy(q: StudySavedQuery): Promise<SavedPayload>;
  /** POST /api/ai/study/ask as server-sent events through postStream; `on` gets {event, data} in order */
  askStudy(body: StudyAskBody, signal: AbortSignal, on: (ev: StreamEvent) => void): Promise<void>;
  /** GET /api/ai/summary/saved?record=N&at=YYYYMMDDHHMM -> parseSaved(json) */
  savedSummary(q: { record: number; at: string }): Promise<SavedPayload>;
  /** POST /api/ai/summary/ask {recordId, lang} as server-sent events */
  askSummary(body: { recordId: number; lang: ReplyLang }, signal: AbortSignal, on: (ev: StreamEvent) => void): Promise<void>;
  /** POST /api/ai/stop {chatId}: always with a chat id, never `{}` */
  stopChat(chatId: number): Promise<void>;
}

/**
 * The spec of one hint / explanation. `body` is read when the ask starts (the card's language may have changed since the
 * popover opened). `saved` null = nothing to look up (the engine then generates).
 */
export function studySpec(tr: StudyTransport, saved: StudySavedQuery | null, body: () => StudyAskBody): RunSpec {
  return {
    loadSaved: () => (saved ? tr.savedStudy(saved) : Promise.resolve(null)),
    ask: (signal, on) => tr.askStudy(body(), signal, on),
    stopChat: (id) => tr.stopChat(id),
  };
}

/** The spec of one record summary; an `at` that does not match `YYYY-MM-DD HH:MM` has no lookup (the engine generates) */
export function summarySpec(tr: StudyTransport, rec: { id: number; at: string }, ui: () => UiLangCode): RunSpec {
  const at = atDigits(rec.at);
  return {
    loadSaved: () => (at ? tr.savedSummary({ record: rec.id, at }) : Promise.resolve(null)),
    ask: (signal, on) => tr.askSummary(buildSummaryAsk(rec.id, ui()), signal, on),
    stopChat: (id) => tr.stopChat(id),
  };
}
