// The AI+ study controls of every page: the practice bar's AI 提示 / AI 解释 buttons, the docks the panel lives in (also the
// greyed bar of a paused / unavailable AI+), the AI 总结 button of a record, the 无法使用 tag and the note above the records
// table. ONE component builds them all, so every page greys, tags, explains and opens them the same way (the panel / overlay
// themselves are components/ai-panel.ts and ai-summary.ts). A control a user may not see is never built (null): without the
// AI+ feature the DOM stays the v0.8 one.
import {
  aiControls, aiPlusView, buildStudyAsk, controlState, dockKey, focusKey, resultExtra, unavailableParts,
  type AiPlusView, type ControlSpec, type ControlState, type QuestionLang, type RunningInfo,
} from '../domain/ai-plus.ts';
import { studySavedQuery, studySpec } from '../domain/ai-spec.ts';
import { sessionPart, studyKey } from '../domain/ai-study.ts';
import { studyTransport } from '../api/ai.ts';
import { t, uiLang, type MsgKey } from '../i18n/index.ts';
import { aiStatus } from '../state/ai.ts';
import { S } from '../state/store.ts';
import { h } from '../utils/dom.ts';
import { AI_BADGE_TEXT } from './ai-badge.ts';
import { openAiPanel, type PanelOpenArgs } from './ai-panel.ts';
import { openAiSummary } from './ai-summary.ts';
import { iconButton, withReason } from './icon-button.ts';
import { miniIcon } from './mini-icons.ts';
import { holdTip } from './tooltip.ts';
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

/** What an opener hands to the panel (also the answer of `resolveRunning`) */
function panelArgs(
  target: StudyTarget, spec: ControlSpec, anchorKey: string, o: { bar: boolean; byKeyboard: boolean },
): PanelOpenArgs {
  const part = sessionPart({ key: target.key, recordId: target.recordId }) ?? `q${target.questionId}`;
  const recordId = target.page === 'practice' ? null : target.recordId;
  const args: PanelOpenArgs = {
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
    dock: dockKey(target.page, target.no),
  };
  const resolve = target.resolve;
  if (resolve) {
    // [查看] on a busy note: the running answer's own control on this page (its kind and place decide which one)
    args.resolveRunning = (r) => {
      const next = resolve(r);
      const own = next ? aiControls(next.page).find((c) => c.kind === r.kind && c.source === r.source) : undefined;
      return next && own ? panelArgs(next, own, focusKey(next.page, own, next.no), { bar: true, byKeyboard: false }) : null;
    };
  }
  return args;
}

/** A bar button or a dock that can rebuild its panel arguments (a redraw or a question change re-anchors through it) */
type ArgsEl = HTMLElement & { __aipArgs?: () => PanelOpenArgs };
/** The `div.aip-dock` of one place (components/ai-panel.ts fills it with the panel or its collapsed bar) */
export type AipDockEl = ArgsEl;

/** The text a greyed control says (its tip and the toast of a tap): `label` names the control */
function greyText(state: ControlState, label: string): string {
  const p = unavailableParts(state);
  if (!p) return '';
  return p.badgeReasonKey ? t('aiplus.tip.na', { label, reason: t(msg(p.badgeReasonKey)) }) : t(msg(p.tipKey));
}

// ---------------------------------------------------------------- practice bar buttons (spec 2.12 c)

/** The "AI" corner mark of a bar button (a constant like the badge's letters, not a catalog text; aria-hidden) */
const AI_MARK = 'AI';

/**
 * The practice bar's AI 提示 / AI 解释 button: `button.ibtn.aip-bar-btn[data-slot]` = icon + `span.aip-ai-mark` + `span.lbl`.
 * The same DOM at every width; ai-plus.css shows the label (wide) or the mark (up to 880 px / a phone turned sideways).
 * Usable: opens the panel with this kind (`__aipArgs` lets the panel follow a question change). Greyed (P-20): the same
 * button with aria-disabled and the reason (`withReason`, never `disabled`). `bar` = the button sits in the action bar.
 */
export function studyButton(target: StudyTarget, spec: ControlSpec, state: ControlState, o: { bar: boolean }): HTMLElement | null {
  if (!state.show) return null;
  const label = t(spec.labelKey);
  const key = focusKey(target.page, spec, target.no);
  const usable = state.usable;
  const btn: ArgsEl = iconButton({
    icon: spec.kind === 'hint' ? 'bulb' : 'book',
    label,
    tone: 'ai',
    focusKey: key,
    onClick: usable ? (e) => openAiPanel(panelArgs(target, spec, key, { bar: o.bar, byKeyboard: e.detail === 0 })) : undefined,
  });
  btn.classList.add('aip-bar-btn');
  btn.dataset.slot = spec.kind;
  btn.append(h('span', { class: 'aip-ai-mark', 'aria-hidden': 'true' }, AI_MARK), h('span', { class: 'lbl' }, label));
  if (!usable) return withReason(btn, greyText(state, label));
  btn.setAttribute('data-aip-opener', '');
  btn.setAttribute('data-aip-pair', `${target.questionId}|${spec.kind}|${spec.source}`);
  btn.__aipArgs = () => panelArgs(target, spec, key, { bar: o.bar, byKeyboard: false });
  // The tip explains what the button does; its accessible name stays the label
  if (spec.tipKey) holdTip(btn, t(spec.tipKey));
  return btn;
}

// ---------------------------------------------------------------- docks (spec 2.12 b)

/** The greyed bar of P-20: [dark AI+ badge] title [无法使用] reason (ellipsis); a tap shows the full reason; no chevron */
function greyedBar(title: string, state: ControlState): HTMLElement {
  const p = unavailableParts(state);
  const reason = p ? t(msg(p.reasonKey)) : '';
  const btn = h('button', {
    type: 'button', class: 'aip-na-bar', 'aria-label': t(msg('aiplus.panel.unavailable'), { label: title }),
  },
    h('span', { class: 'ai-badge off', 'aria-hidden': 'true' },
      h('span', { class: 'ai-badge-t' }, AI_BADGE_TEXT.slice(0, -1), h('span', { class: 'ai-badge-plus' }, AI_BADGE_TEXT.slice(-1)))),
    h('span', { class: 'aip-na-title' }, title),
    naTag(state),
    h('span', { class: 'aip-na-reason' }, reason));
  return withReason(btn, greyText(state, title));
}

/**
 * The AI+ dock of one place (spec 2.12 b). Practice: key `practice`; result / detail card n: `rv-<n>`.
 *  - !state.show: null (nothing in the DOM);
 *  - greyed: `div.aip-dock.aip-dock-na[data-aip-dock-na]` holding the greyed bar; it never takes the live panel;
 *  - usable: an empty `div.aip-dock[data-aip-dock]`; a card's dock carries `__aipArgs` (its AI 解析 arguments), the practice
 *    dock none (the bar buttons carry theirs).
 */
export function studyDock(target: StudyTarget, spec: ControlSpec, state: ControlState): AipDockEl | null {
  if (!state.show) return null;
  const key = dockKey(target.page, target.no);
  if (!state.usable) {
    const title = target.page === 'practice' ? t(msg('aiplus.panel.idleTitle')) : t(spec.labelKey);
    return h('div', { class: 'aip-dock aip-dock-na', 'data-aip-dock-na': key }, greyedBar(title, state));
  }
  const dock: AipDockEl = h('div', { class: 'aip-dock', 'data-aip-dock': key });
  if (target.page !== 'practice') {
    dock.__aipArgs = () => panelArgs(target, spec, focusKey(target.page, spec, target.no), { bar: false, byKeyboard: false });
  }
  return dock;
}

/** The 无法使用 tag of one group: span.aip-na[aria-hidden=true]; null unless greyed with tag:true */
export function naTag(state: ControlState): HTMLElement | null {
  if (!state.show || state.usable || !state.tag) return null;
  return h('span', { class: 'aip-na', 'aria-hidden': 'true' }, t('aiplus.na'));
}

/**
 * The AI 总结 button of a record (records cell: quiet + no own tag; Home: own tag after it); null when !state.show.
 * Sparkle icon + text; usable ones are outlined, greyed ones flat (SH-17) with the reason as tip and tap toast.
 */
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
    control = withReason(btn, greyText(state, t('aiplus.summary')));
    // The pages (Home) lay the wrapper out by this class
    control.classList.add('aip-tipwrap');
  }
  const tag = o.tag ? naTag(state) : null;
  return tag ? h('span', { class: 'aip-grp-inline' }, control, tag) : control;
}

/** p.aip-tablenote above the records table: the 无法使用 tag and the reason; null unless greyed with tag:true */
export function aiTableNote(state: ControlState): HTMLElement | null {
  const tag = naTag(state);
  if (!tag) return null;
  return h('p', { class: 'aip-tablenote' }, tag, ' ', greyText(state, t('aiplus.summary')));
}
