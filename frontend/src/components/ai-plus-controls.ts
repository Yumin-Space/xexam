// The AI+ study controls of every page (v0.9): the bulb / book icons, the AI 提示 / AI 解释 / AI 解析 buttons, the AI 总结
// button of a record, the 无法使用 tag and the note above the records table. ONE component builds them all, so every page
// greys, tags, explains and opens them the same way (the popover / overlay themselves are components/ai-pop.ts and
// ai-summary.ts). A control a user may not see is never built (null): without the AI+ feature the DOM stays the v0.8 one.
import {
  aiControls, aiPlusView, buildStudyAsk, controlState, type AiPlusView, type ControlSpec, type ControlState, type QuestionLang, type RunningInfo,
  resultExtra,
} from '../domain/ai-plus.ts';
import { studySavedQuery, studySpec } from '../domain/ai-spec.ts';
import { sessionPart, studyKey } from '../domain/ai-study.ts';
import { studyTransport } from '../api/ai.ts';
import { t, uiLang, type MsgKey } from '../i18n/index.ts';
import { aiStatus } from '../state/ai.ts';
import { S } from '../state/store.ts';
import { h } from '../utils/dom.ts';
import { toast } from '../utils/toast.ts';
import { openAiPop, type PopOpenArgs } from './ai-pop.ts';
import { openAiSummary } from './ai-summary.ts';
import { miniIcon } from './mini-icons.ts';
import { tipEl, withTip } from './tooltip.ts';
import '../styles/ai-plus.css';

const msg = (k: string): MsgKey => k as MsgKey;

/** aiPlusView({role: S.user.role, service: S.user.aiAllowed, status: aiStatus()}); {show:false} without a user */
export function aiPlusNow(): AiPlusView {
  const u = S.user;
  if (!u) return { show: false };
  const st = aiStatus();
  // The session's flag is refreshed only on route changes; the status (refreshed after a refusal) knows sooner that the
  // administrator removed the feature during a practice
  const removed = st.state === 'ready' && st.enabled && !st.suspended && st.allowed === false;
  return aiPlusView({ role: u.role, service: u.aiAllowed && !removed, status: st });
}

/** Where a hint / explanation belongs (spec 7.4, 7.5, A3, A7) */
export interface StudyTarget {
  page: 'practice' | 'result' | 'detail';
  mode: 'practice' | 'exam';
  /** practice / result: the session key; detail: record.sessionKey ?? null */
  key: string | null;
  /** Unix seconds; same rule */
  startedAt: number | null;
  /** practice: null; result: R.recordId; detail: the record id */
  recordId: number | null;
  questionId: string;
  /** The 1-based number shown */
  no: number;
  /** Read when the ask starts: the language the card / page shows now */
  qlang: () => QuestionLang;
  /** Practice page only (optional elsewhere): the page's own target for a running answer of this session ([查看]) */
  resolve?: (r: RunningInfo) => StudyTarget | null;
}

/**
 * controlState(aiPlusNow(), extra): the practice page has no row-level reason; elsewhere an exam without a record id, or a
 * target with neither a key nor a record id, is 'notSaved'.
 */
export function studyState(target: StudyTarget): ControlState {
  const view = aiPlusNow();
  if (target.page === 'practice') return controlState(view, null);
  const extra = resultExtra({ mode: target.mode, recordId: target.recordId })
    ?? (target.key == null && target.recordId == null ? 'notSaved' : null);
  return controlState(view, extra);
}

/** The words a greyed control says (its tooltip and the toast of a tap) */
function greyText(state: ControlState, labelKey: string): string {
  if (!state.show || state.usable) return '';
  const tip = state.tip;
  if (tip.key === 'aiplus.tip.na' && tip.reasonKey) return t('aiplus.tip.na', { label: t(msg(labelKey)), reason: t(msg(tip.reasonKey)) });
  return t(msg(tip.key));
}

/** What an opener hands to the popover (also the answer of `resolveRunning`) */
function popArgs(
  target: StudyTarget, spec: ControlSpec, anchorKey: string, o: { bar: boolean; byKeyboard: boolean },
): PopOpenArgs {
  const part = sessionPart({ key: target.key, recordId: target.recordId }) ?? `q${target.questionId}`;
  const recordId = target.page === 'practice' ? null : target.recordId;
  const args: PopOpenArgs = {
    kind: spec.kind,
    qid: target.questionId,
    source: spec.source,
    no: target.no,
    anchorKey,
    cacheKey: studyKey({ mode: target.mode, sessionKey: part, source: spec.source, kind: spec.kind, questionId: target.questionId }),
    spec: studySpec(
      studyTransport,
      studySavedQuery({
        page: target.page, mode: target.mode, source: spec.source, kind: spec.kind, questionId: target.questionId,
        key: target.key, recordId: target.recordId,
      }),
      () => buildStudyAsk({
        kind: spec.kind, mode: target.mode, key: target.key, startedAt: target.startedAt, questionId: target.questionId,
        no: target.no, qlang: target.qlang(), ui: uiLang(), source: spec.source, recordId,
      }),
    ),
    sessionPrefix: `${target.mode}|${part}|`,
    fromBar: o.bar,
    byKeyboard: o.byKeyboard,
  };
  const resolve = target.resolve;
  if (resolve) {
    // [查看] on a busy note: the running answer's own control on this page (its kind and place decide which one)
    args.resolveRunning = (r) => {
      const next = resolve(r);
      const own = next ? aiControls(next.page).find((c) => c.kind === r.kind && c.source === r.source) : undefined;
      return next && own ? popArgs(next, own, focusKeyOf(next, own, 'button'), { bar: true, byKeyboard: false }) : null;
    };
  }
  return args;
}

/** The data-focus key of an opener (D12): practice `q-bulb`…, result / detail `rv-book-<n>`… */
function focusKeyOf(target: StudyTarget, spec: ControlSpec, which: 'icon' | 'button'): string {
  const base = spec.focus[which];
  return target.page === 'practice' ? base : `${base}-${target.no}`;
}

let tipSeq = 0;

/** A greyed control: aria-disabled (never `disabled`), its reason as tooltip and aria-describedby, a tap shows it as a toast */
function greyOut(control: HTMLElement, focusKey: string, text: string): HTMLElement {
  tipSeq += 1;
  const id = `aip-tip-${focusKey}-${tipSeq}`;
  control.classList.add('aip-off');
  control.setAttribute('aria-disabled', 'true');
  control.setAttribute('aria-describedby', id);
  control.addEventListener('click', () => toast(text));
  return withTip(control, tipEl(id, text), 'aip-tipwrap');
}

/** What every study opener shares; returns the control, wrapped in its tooltip when it has one */
function opener(
  control: HTMLButtonElement, target: StudyTarget, spec: ControlSpec, state: ControlState, focusKey: string, bar: boolean,
): HTMLElement {
  control.type = 'button';
  control.setAttribute('data-focus', focusKey);
  control.setAttribute('data-aip-opener', '');
  control.setAttribute('data-aip-pair', `${target.questionId}|${spec.kind}|${spec.source}`);
  control.setAttribute('aria-haspopup', 'dialog');
  if (!state.show) return control;
  if (!state.usable) return greyOut(control, focusKey, greyText(state, spec.labelKey));
  control.addEventListener('click', (e) => {
    openAiPop(popArgs(target, spec, focusKey, { bar, byKeyboard: e.detail === 0 }));
  });
  if (!spec.tipKey) return control;
  tipSeq += 1;
  const id = `aip-tip-${focusKey}-${tipSeq}`;
  control.setAttribute('aria-describedby', id);
  return withTip(control, tipEl(id, t(spec.tipKey)), 'aip-tipwrap');
}

/** Header icon (bulb = hint, book = explain): button.aip-ico, null when !state.show */
export function studyIcon(target: StudyTarget, spec: ControlSpec, state: ControlState): HTMLElement | null {
  if (!state.show) return null;
  const label = t(spec.labelKey);
  const btn = h('button', { class: 'aip-ico', 'aria-label': label, title: spec.tipKey ? null : label },
    miniIcon(spec.kind === 'hint' ? 'bulb' : 'book', 'aip-mi'));
  return opener(btn, target, spec, state, focusKeyOf(target, spec, 'icon'), false);
}

/**
 * Text button (AI 提示 / AI 解释 / AI 解析): button.btn.aip-btn with the orange mini icon. `bar` = in the bottom action bar
 * (the popover opens upwards); the buttons that have a short label hold both spans and the CSS shows one of them.
 */
export function studyButton(target: StudyTarget, spec: ControlSpec, state: ControlState, o: { bar: boolean }): HTMLElement | null {
  if (!state.show) return null;
  const full = t(spec.labelKey);
  const kids = spec.shortKey
    ? [h('span', { class: 'aip-full' }, full), h('span', { class: 'aip-short', 'aria-hidden': 'true' }, t(spec.shortKey))]
    : [full];
  const btn = h('button', { class: 'btn aip-btn', 'aria-label': spec.shortKey ? full : null },
    miniIcon(spec.kind === 'hint' ? 'bulb' : 'book', 'aip-mi'), ...kids);
  return opener(btn, target, spec, state, focusKeyOf(target, spec, 'button'), o.bar);
}

/** The 无法使用 tag of one group: span.aip-na[aria-hidden=true]; null unless greyed with tag:true */
export function naTag(state: ControlState): HTMLElement | null {
  if (!state.show || state.usable || !state.tag) return null;
  return h('span', { class: 'aip-na', 'aria-hidden': 'true' }, t('aiplus.na'));
}

/** span.phead-r wrapping [span.aip-icons(icons…), naTag, langSwitch] (nulls skipped; no span.aip-icons when no icon) */
export function pheadRight(icons: Array<HTMLElement | null>, tag: HTMLElement | null, sw: HTMLElement): HTMLElement {
  const present = icons.filter((i): i is HTMLElement => i != null);
  return h('span', { class: 'phead-r' }, present.length > 0 ? h('span', { class: 'aip-icons' }, present) : null, tag, sw);
}

/** The AI 总结 button of a record (records cell: quiet + no own tag; Home: own tag after it); null when !state.show */
export function summaryButton(
  rec: { id: number; at: string; kind: 'practice' | 'exam' }, state: ControlState,
  o: { anchorKey: string; quiet: boolean; tag: boolean },
): HTMLElement | null {
  if (!state.show) return null;
  const btn = h('button', {
    class: 'btn aip-sum-btn' + (o.quiet ? ' quiet' : ''), type: 'button', 'data-focus': o.anchorKey, 'data-aip-opener': '',
    'aria-haspopup': 'dialog', 'aria-label': o.quiet ? t('aiplus.row.summaryLabel', { at: rec.at }) : null,
  }, miniIcon('sparkle', 'aip-mi'), t('aiplus.summary'));
  let control: HTMLElement = btn;
  if (state.usable) {
    btn.addEventListener('click', (e) => openAiSummary({ rec, anchorKey: o.anchorKey, byKeyboard: e.detail === 0 }));
  } else {
    control = greyOut(btn, o.anchorKey, greyText(state, 'aiplus.summary'));
  }
  const tag = o.tag ? naTag(state) : null;
  return tag ? h('span', { class: 'aip-grp-inline' }, control, tag) : control;
}

/** p.aip-tablenote above the records table: the 无法使用 tag and the reason; null unless greyed with tag:true */
export function aiTableNote(state: ControlState): HTMLElement | null {
  const tag = naTag(state);
  if (!tag) return null;
  return h('p', { class: 'aip-tablenote' }, tag, ' ', greyText(state, 'aiplus.summary'));
}
