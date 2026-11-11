// AI+ study features (v0.9): who sees the controls and in which state, which controls each page has, the request bodies,
// the small facts the windows show (the "based on" line, chat titles with their suffix, what is running). Pure: no DOM and
// no i18n calls (it runs under `node --test`); keys are returned as strings and the components translate them.
import type { Role } from '../types/api.ts';
import type { AiStatus } from '../state/ai.ts';
import { badgeState } from './badge-state.ts';

// ---------------------------------------------------------------- visibility (spec 7.2)

export type AiPlusReason = 'loading' | 'suspended' | 'unavailable';
export type AiPlusView =
  | { show: false }
  | { show: true; usable: true }
  | { show: true; usable: false; reason: AiPlusReason };

/**
 * What a signed-in person sees of every v0.9 control. Built on the round-2 badge state, with the same inputs as the header
 * badge: `service` = the session's `aiAllowed`, suspended / available from the AI status. A user with the service sees the
 * controls greyed while the first status is still being read; everybody without a badge sees nothing.
 */
export function aiPlusView(i: { role: Role; service: boolean; status: AiStatus }): AiPlusView {
  const st = i.status;
  if (st.state === 'loading') {
    return i.role === 'user' && i.service ? { show: true, usable: false, reason: 'loading' } : { show: false };
  }
  const ready = st.state === 'ready';
  const b = badgeState({
    role: i.role, service: i.service, suspended: ready && st.suspended === true, available: ready && st.enabled === true,
  });
  if (b.kind === 'none') return { show: false };
  if (b.kind === 'off') return { show: true, usable: false, reason: b.reason };
  return { show: true, usable: true };
}

// ---------------------------------------------------------------- control state (view + row-level reasons)

/** Reasons that grey one control although AI itself works (no 无法使用 tag) */
export type ExtraReason = 'noDetail' | 'noWrong' | 'notSaved';
export type ControlReason = AiPlusReason | ExtraReason;

/** The tooltip of a greyed control: a message key, plus for `aiplus.tip.na` the key of the badge reason text */
export type TipSpec =
  | { key: 'aiplus.tip.na'; reasonKey: 'ai.badge.suspended' | 'ai.badge.unavailable' }
  | { key: string; reasonKey?: undefined };

export type ControlState =
  | { show: false }
  | { show: true; usable: true }
  | { show: true; usable: false; reason: ControlReason; tag: boolean; tip: TipSpec };

const WHY_KEY: Record<'loading' | ExtraReason, string> = {
  loading: 'aiplus.why.loading',
  noDetail: 'aiplus.why.noDetail',
  noWrong: 'aiplus.why.noWrong',
  notSaved: 'aiplus.why.notSaved',
};

/**
 * One control's state: the view first (a service problem greys everything, with the 无法使用 tag for `suspended` /
 * `unavailable` only), then the control's own reason (greyed without a tag).
 */
export function controlState(view: AiPlusView, extra: ExtraReason | null = null): ControlState {
  if (!view.show) return { show: false };
  if (!view.usable) {
    const r = view.reason;
    const tip: TipSpec = r === 'loading'
      ? { key: WHY_KEY.loading }
      : { key: 'aiplus.tip.na', reasonKey: r === 'suspended' ? 'ai.badge.suspended' : 'ai.badge.unavailable' };
    return { show: true, usable: false, reason: r, tag: r !== 'loading', tip };
  }
  if (extra) return { show: true, usable: false, reason: extra, tag: false, tip: { key: WHY_KEY[extra] } };
  return { show: true, usable: true };
}

/** The row-level reason of a study record's summary button: `noDetail` first, then `noWrong` (answered wrong = answered - correct) */
export function recordSummaryExtra(r: { detail?: boolean; answered: number | null; correct: number }): ExtraReason | null {
  if (r.detail === false) return 'noDetail';
  if (r.answered == null || r.answered - r.correct <= 0) return 'noWrong';
  return null;
}

/** The result page's reason: an exam result whose record was not saved cannot be explained */
export function resultExtra(i: { mode: 'practice' | 'exam'; recordId: number | null }): ExtraReason | null {
  return i.mode === 'exam' && i.recordId == null ? 'notSaved' : null;
}

// ---------------------------------------------------------------- which controls a page has (spec 7.4, 7.5)

export type StudyKind = 'hint' | 'explain';
export type StudySource = 'practice' | 'result';
/**
 * `exam` = the exam while it runs (AI-free); `result` = a practice or exam result page; `detail` = the record detail page
 * (owner answer 4)
 */
export type AiPage = 'practice' | 'exam' | 'result' | 'detail';

export interface ControlSpec {
  kind: StudyKind;
  source: StudySource;
  /** `data-focus` keys of the header icon and of the button */
  focus: { icon: string; button: string };
  /** Label key of the button and the icon (the book icon on result pages is the same control as AI 解析) */
  labelKey: 'aiplus.hint' | 'aiplus.explain' | 'aiplus.analyze';
  shortKey: 'aiplus.hint.short' | 'aiplus.explain.short' | null;
  tipKey: 'aiplus.tip.hint' | 'aiplus.tip.explain' | null;
}

export function aiControls(page: AiPage): ControlSpec[] {
  if (page === 'practice') {
    return [
      { kind: 'hint', source: 'practice', focus: { icon: 'q-bulb', button: 'q-aihint' }, labelKey: 'aiplus.hint', shortKey: 'aiplus.hint.short', tipKey: 'aiplus.tip.hint' },
      { kind: 'explain', source: 'practice', focus: { icon: 'q-book', button: 'q-aiexplain' }, labelKey: 'aiplus.explain', shortKey: 'aiplus.explain.short', tipKey: 'aiplus.tip.explain' },
    ];
  }
  if (page === 'result' || page === 'detail') {
    return [{ kind: 'explain', source: 'result', focus: { icon: 'rv-book', button: 'rv-ai' }, labelKey: 'aiplus.analyze', shortKey: null, tipKey: null }];
  }
  return [];
}

/** The popover title key: the explanation of a result page is "AI 解析" */
export const popTitleKey = (kind: StudyKind, source: StudySource): 'aiplus.pop.title.hint' | 'aiplus.pop.title.explain' | 'aiplus.pop.title.analyze' =>
  kind === 'hint' ? 'aiplus.pop.title.hint' : source === 'result' ? 'aiplus.pop.title.analyze' : 'aiplus.pop.title.explain';

/** The label key of what a study answer is called (busy notes, chat labels) */
export const kindLabelKey = (kind: StudyKind, source: StudySource | null): 'aiplus.hint' | 'aiplus.explain' | 'aiplus.analyze' =>
  kind === 'hint' ? 'aiplus.hint' : source === 'result' ? 'aiplus.analyze' : 'aiplus.explain';

// ---------------------------------------------------------------- request bodies (spec 5.3)

export type ReplyLang = 'en' | 'zh-CN' | 'ja';
export type QuestionLang = 'en' | 'zh' | 'ja';
export type UiLangCode = 'en' | 'zh' | 'ja';

/** The reply language of a request: the Settings language (`zh` is sent as `zh-CN`) */
export const replyLang = (ui: UiLangCode): ReplyLang => (ui === 'zh' ? 'zh-CN' : ui);

export interface StudyAskBody {
  kind: StudyKind;
  mode: 'practice' | 'exam';
  /** Left out when unknown (only with `recordId`: the server then takes the session from the record) */
  key?: string;
  startedAt?: number;
  questionId: string;
  no: number;
  qlang: QuestionLang;
  lang: ReplyLang;
  source: StudySource;
  recordId?: number | null;
}

export interface StudyAskInput {
  kind: StudyKind;
  mode: 'practice' | 'exam';
  key: string | null;
  startedAt: number | null;
  questionId: string;
  no: number;
  qlang: QuestionLang;
  ui: UiLangCode;
  source: StudySource;
  recordId?: number | null;
}

/**
 * The body of POST /api/ai/study/ask. Enforces the server's combination rules (a hint = practice mode and place only; an
 * exam explanation is a result-page one and needs its record), so a wrong call fails here, in a test, not as a 400.
 * `key` / `startedAt` are optional when `recordId` is set (coordinator answers A1 / A3: the record detail page sends them
 * only when the record JSON has them, the browser never invents keys) and are left out of the body when null.
 */
export function buildStudyAsk(i: StudyAskInput): StudyAskBody {
  if (i.kind === 'hint' && (i.mode !== 'practice' || i.source !== 'practice')) throw new Error('a hint is only for the practice page');
  if (i.mode === 'exam' && (i.source !== 'result' || i.recordId == null)) throw new Error('an exam explanation needs the result page and its record');
  if (i.recordId == null && (i.key == null || i.startedAt == null)) throw new Error('a request without a record id needs the session key and start time');
  const body: StudyAskBody = {
    kind: i.kind, mode: i.mode, questionId: i.questionId,
    no: Math.min(100000, Math.max(1, Math.floor(i.no))), qlang: i.qlang, lang: replyLang(i.ui), source: i.source,
  };
  if (i.key != null) body.key = i.key;
  if (i.startedAt != null) body.startedAt = Math.max(1, Math.floor(i.startedAt));
  if (i.recordId != null) body.recordId = i.recordId;
  return body;
}

/** The body of POST /api/ai/summary/ask */
export const buildSummaryAsk = (recordId: number, ui: UiLangCode): { recordId: number; lang: ReplyLang } => ({ recordId, lang: replyLang(ui) });

// ---------------------------------------------------------------- records and chats

/** `2026-11-11 21:58` -> `202611112158` (digits only: the server's query values are not percent-decoded); null when it does not match */
export function atDigits(at: string): string | null {
  const m = /^(\d{4})-(\d{2})-(\d{2}) (\d{2}):(\d{2})$/.exec(at);
  return m ? m[1]! + m[2]! + m[3]! + m[4]! + m[5]! : null;
}

export interface Sample {
  used: number;
  usable: number;
  wrong: number;
}

export type BasedKey = 'aiplus.sum.basedAll' | 'aiplus.sum.basedSome' | 'aiplus.sum.basedGone' | 'aiplus.sum.basedSomeGone';

/** Which "based on" line the overlay shows: random = fewer sent than usable; gone = fewer usable than wrong */
export function basedKey(s: Sample): BasedKey {
  const random = s.usable > s.used;
  const gone = s.usable < s.wrong;
  if (random && gone) return 'aiplus.sum.basedSomeGone';
  if (random) return 'aiplus.sum.basedSome';
  if (gone) return 'aiplus.sum.basedGone';
  return 'aiplus.sum.basedAll';
}

export type ChatKind = 'chat' | 'practice' | 'exam' | 'summary';
export type ChatSuffixKey = 'aiplus.chat.suffix.practice' | 'aiplus.chat.suffix.exam' | 'aiplus.chat.suffix.summary';

/** A chat's title as shown: the stored title plus, for study chats, a suffix key the page translates (`<time>-练习`) */
export function chatDisplayTitle(c: { title: string; kind?: ChatKind | null }): { text: string; suffixKey: ChatSuffixKey | null } {
  switch (c.kind) {
    case 'practice': return { text: c.title, suffixKey: 'aiplus.chat.suffix.practice' };
    case 'exam': return { text: c.title, suffixKey: 'aiplus.chat.suffix.exam' };
    case 'summary': return { text: c.title, suffixKey: 'aiplus.chat.suffix.summary' };
    default: return { text: c.title, suffixKey: null };
  }
}

// ---------------------------------------------------------------- what is running (409 ai.busy, spec 5.5 / 7.8)

export interface RunningInfo {
  chatId: number;
  messageId: number;
  chatKind: ChatKind;
  kind: 'chat' | 'hint' | 'explain' | 'summary';
  questionId: string | null;
  questionNo: number | null;
  source: StudySource | null;
  recordId: number | null;
}

export type BusyView =
  | { kind: 'study'; labelKey: 'aiplus.hint' | 'aiplus.explain' | 'aiplus.analyze'; n: number; noteKey: 'aiplus.busy.study'; canShow: boolean }
  | { kind: 'summary'; noteKey: 'aiplus.busy.summary'; canShow: false }
  | { kind: 'chat'; noteKey: 'aiplus.busy.chat'; canShow: false }
  | { kind: 'other'; noteKey: 'aiplus.busy.other'; canShow: false };

/**
 * The busy note's wording and whether [查看] is offered. `sessionChatId` = the chat of the session the page shows (known from
 * an earlier saved / start answer of this session), so a running answer of ANOTHER chat is never offered as "show".
 */
export function busyView(running: RunningInfo | null, sessionChatId: number | null): BusyView {
  if (!running) return { kind: 'other', noteKey: 'aiplus.busy.other', canShow: false };
  if (running.kind === 'summary') return { kind: 'summary', noteKey: 'aiplus.busy.summary', canShow: false };
  if (running.kind === 'hint' || running.kind === 'explain') {
    const canShow = running.questionId != null && sessionChatId != null && running.chatId === sessionChatId
      && (running.chatKind === 'practice' || running.chatKind === 'exam');
    return {
      kind: 'study', labelKey: kindLabelKey(running.kind, running.source), n: running.questionNo ?? 0, noteKey: 'aiplus.busy.study', canShow,
    };
  }
  return { kind: 'chat', noteKey: 'aiplus.busy.chat', canShow: false };
}
