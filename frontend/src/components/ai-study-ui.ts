// Shared pieces of the AI popover and the AI summary overlay (v0.9): the dependencies they get from the page layer, the
// wording of notes and states, and a few DOM helpers. The wording is the only place that calls t() for study answers; the
// decisions behind it are pure (domain/ai-study.ts, domain/ai-plus.ts).
import { localeTag, t, type MsgKey } from '../i18n/index.ts';
import { busyView, type ReplyLang, type UiLangCode } from '../domain/ai-plus.ts';
import type { AskError, EntryView, Note } from '../domain/ai-study.ts';
import type { StudyTransport } from '../domain/ai-spec.ts';

/** What the page layer provides (wired once in main.ts, see INTEGRATION.md) */
export interface StudyUiDeps {
  transport: StudyTransport;
  /** The Settings language (`uiLang()` of i18n) */
  uiLang(): UiLangCode;
  /** The text of a request error: by code, else by status (the frontend's `errorText` over an ApiError) */
  errorText(e: AskError): string;
  /** The Settings language picker's name of a reply language ("English", "中文", "日本語") */
  langName(l: ReplyLang): string;
  /** `refreshAi(force)` of state/actions.ts followed by a redraw when the status changed */
  refreshAi(force: boolean): void;
  /** Navigates to AI Chat with that chat open (`#/ai/<id>`) */
  openChat(chatId: number): void;
}

/** A key built at run time (the pure domain returns plain strings) */
export const msgKey = (k: string): MsgKey => k as MsgKey;

/** The text of a note (the one line under the title) */
export function noteText(note: Note, deps: StudyUiDeps, sessionChatId: number | null): string {
  switch (note.kind) {
    case 'busy': {
      const b = busyView(note.running, sessionChatId);
      return b.kind === 'study' ? t(b.noteKey, { label: t(b.labelKey), n: b.n }) : t(b.noteKey);
    }
    case 'error':
      return deps.errorText({ status: note.status, code: note.code, ...(note.limit != null ? { limit: note.limit } : {}) });
    case 'withheld':
      return t('aiplus.withheld');
    case 'withheldPrev':
      return t('aiplus.failed.withheldPrev');
    case 'failedPrev':
      return t('aiplus.failed.prev', { reason: note.reason === 'stopped' ? t('ai.status.stopped') : deps.errorText({ status: 0, code: note.code }) });
    case 'otherLang':
      return t('aiplus.otherLang', { lang: deps.langName(note.lang) });
  }
}

/** Notes that are problems (announced at once, role=alert); the language and busy notes are quiet */
export const noteIsAlert = (note: Note | null): boolean =>
  note != null && (note.kind === 'error' || note.kind === 'withheld' || note.kind === 'withheldPrev' || note.kind === 'failedPrev');

/** The short state line under the answer: "Writing…", "Saved 20:44", "Stopped" */
export function stateText(v: EntryView): string {
  switch (v.phase) {
    case 'loading': return t('aiplus.state.loading');
    case 'requesting': return t('ai.writing');
    case 'streaming': return t(v.thinking ? 'ai.thinking' : 'ai.writing');
    case 'stopped': return t('ai.status.stopped');
    case 'done': return v.savedAt != null ? t('aiplus.state.saved', { time: clock(v.savedAt) }) : '';
    default: return '';
  }
}

/** HH:MM of a unix time in the Settings language's locale */
export function clock(unix: number): string {
  try {
    return new Date(unix * 1000).toLocaleTimeString(localeTag(), { hour: '2-digit', minute: '2-digit' });
  } catch {
    return '';
  }
}

/** The value a window remembers between paints for `announce`: the phase, and whether a reasoning model is still thinking */
export const phaseKey = (v: EntryView): string => (v.thinking ? `${v.phase}:thinking` : v.phase);

/** What an announcement for a phase change says (for the polite live region); `prev` is the last `phaseKey` */
export function announce(prev: string | null, v: EntryView, deps: StudyUiDeps): string | null {
  if (prev === phaseKey(v)) return null;
  const p = prev?.split(':')[0] ?? null;
  if (v.phase === 'streaming' && v.thinking) return t('ai.thinking');
  if (v.phase === 'streaming' || v.phase === 'requesting') return t('ai.writing');
  if (v.phase === 'done' && (p === 'streaming' || p === 'requesting')) return t('ai.status.done');
  if (v.phase === 'stopped' && p === 'streaming') return t('ai.status.stopped');
  if ((v.phase === 'error' || v.phase === 'withheld') && v.note) return noteText(v.note, deps, null);
  return null;
}

/**
 * Replaces the children of `box` and keeps the keyboard focus: when it was on one of them, it goes to the button now in that
 * slot (Stop <-> Regenerate), else to the nearest remaining enabled one. A rebuilt group would otherwise drop it to <body>.
 */
export function replaceKeepingFocus(box: HTMLElement, kids: HTMLElement[]): void {
  const active = document.activeElement;
  const slot = active && box.contains(active) ? [...box.children].findIndex((c) => c === active || c.contains(active)) : -1;
  box.replaceChildren(...kids);
  if (slot < 0) return;
  const ok = (el: HTMLElement | undefined): el is HTMLElement => el != null && !(el instanceof HTMLButtonElement && el.disabled);
  const pick = [kids[slot], ...kids.slice(0, slot).reverse(), ...kids.slice(slot + 1)].find(ok);
  pick?.focus();
}

/** env(safe-area-inset-top / -bottom) in px (a probe element reads the computed padding) */
export function safeInsets(): { top: number; bottom: number } {
  const probe = document.createElement('div');
  probe.style.cssText = 'position:fixed;visibility:hidden;pointer-events:none;padding-top:env(safe-area-inset-top,0px);padding-bottom:env(safe-area-inset-bottom,0px)';
  document.body.append(probe);
  const cs = getComputedStyle(probe);
  const out = { top: parseFloat(cs.paddingTop) || 0, bottom: parseFloat(cs.paddingBottom) || 0 };
  probe.remove();
  return out;
}

/** Puts the focus back on the control that opened a window (found again by its data-focus key after redraws) */
export function focusOpener(anchorKey: string): void {
  const el = document.querySelector<HTMLElement>(`[data-focus="${anchorKey}"]`);
  el?.focus();
}

/** The word for a record's kind in the overlay's meta line (Practice / Exam) */
export const kindWord = (kind: 'practice' | 'exam'): string => t(kind === 'practice' ? 'aiplus.chat.suffix.practice' : 'aiplus.chat.suffix.exam');
