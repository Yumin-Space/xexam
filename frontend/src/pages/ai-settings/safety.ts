// AI settings page, Safety tab (v1.9): seven sub-tabs, one limits group each: Limits (per-user caps, administrators exempt, the
// site-wide daily budget, the one answer length), Chat, Hints and analysis (shared reuse, days, clear), Summary, AI generation,
// AI import and Server (calls and jobs at once, the provider rate limit, then the chat retention). The sub-tab is in the URL
// (`&sub=`); without one the last sub-tab of this browser session is shown. One Save sends every changed limit of the whole form
// (all seven sub-tabs share it); Reset fills the defaults of the shown sub-tab only. The field helpers here are shared with the
// AI banks tab (gen.ts).
import { clearAiData, fetchLimits, saveLimits, saveRetention as saveRetentionApi } from '../../api/ai-admin.ts';
import { ApiError } from '../../api/client.ts';
import { infoTip } from '../../components/tooltip.ts';
import {
  CAP_KEYS, DEFAULT_LIMITS, genCallLive, hashOfSafety, limitIssue, limitsFormOf, limitsUpdate, LIMIT_KEYS, paceEffective, rangeOf,
  retentionOf, SAFETY_SUBS, SUB_KEYS, subOfHash, subOfKey, type LimitKey, type LimitRanges, type LimitValues, type SafetySub,
} from '../../domain/ai-settings.ts';
import { aiCodeText, errorText } from '../../i18n/errors.ts';
import { limitHintArgs } from '../../domain/usage-bars.ts';
import { t, type MsgKey } from '../../i18n/index.ts';
import { holdTip } from '../../components/tooltip.ts';
import { refreshAi } from '../../state/actions.ts';
import type { AiLimitsAnswer } from '../../types/api.ts';
import { ask } from '../../utils/dialog.ts';
import { h, type Child } from '../../utils/dom.ts';
import { toast } from '../../utils/toast.ts';
import { aiDown } from './status.ts';
import { A, adopt, num, readSafetySub, redraw, writeSafetySub } from './state.ts';
import { loadUsage } from './usage.ts';
import { loadUsers } from './users.ts';

/** The limits counted in tokens (their range hint is shown compact) */
const TOKEN_LIMITS: ReadonlySet<LimitKey> = new Set<LimitKey>(['replyMaxTokens', 'memoryTokens', 'genCallTokens', 'dailyTokenBudget', 'paceTpm']);

export const LIMIT_LABEL: Record<LimitKey, MsgKey> = {
  callsPerMinute: 'ai.set.limit.callsPerMinute', callsPerHour: 'ai.set.limit.callsPerHour', callsPerDay: 'ai.set.limit.callsPerDay',
  callsAdminsExempt: 'ai.limits.callsAdminsExempt',
  replyMaxTokens: 'ai.set.limit.replyMaxTokens', memoryTokens: 'ai.set.limit.memoryTokens',
  perUserRunning: 'ai.set.limit.perUserRunning', globalRunning: 'ai.set.limit.globalRunning', chatsPerUser: 'ai.set.limit.chatsPerUser',
  messagesPerChat: 'ai.set.limit.messagesPerChat', memoriesPerUser: 'ai.set.limit.memoriesPerUser',
  genPerDay: 'ai.set.limit.genPerDay', genAdminsExempt: 'ai.set.limit.genAdminsExempt', genMinSource: 'ai.set.limit.genMinSource',
  genMinWrong: 'ai.set.limit.genMinWrong', genBankSingle: 'ai.set.limit.genSingle', genBankMulti: 'ai.set.limit.genMulti',
  genWrongSingle: 'ai.set.limit.genSingle', genWrongMulti: 'ai.set.limit.genMulti', genCallTokens: 'ai.set.limit.genCallTokens',
  genRunning: 'ai.set.limit.genRunning', importMaxMb: 'ai.set.limit.importMaxMb', importMaxQuestions: 'ai.set.limit.importMaxQuestions',
  genMaxCalls: 'ai.set.limit.genMaxCalls', importMaxCalls: 'ai.set.limit.importMaxCalls', recognizeMaxCalls: 'ai.set.limit.recognizeMaxCalls',
  genUnlimited: 'ai.set.limit.unlimited', importUnlimited: 'ai.set.limit.unlimited',
  studyShare: 'ai.limits.studyShare', studyShareDays: 'ai.limits.studyShareDays', genMaxChunks: 'ai.limits.genMaxChunks',
  summaryMaxItems: 'ai.limits.summaryMaxItems', dailyTokenBudget: 'ai.limits.dailyTokenBudget',
  paceOn: 'ai.limits.paceOn', paceRpm: 'ai.set.limit.paceRpm', paceTpm: 'ai.set.limit.paceTpm',
  assessPerDay: 'ai.set.limit.assessPerDay', assessAdminsExempt: 'ai.set.limit.assessAdminsExempt',
};

/** The label of a sub-tab: the full name and the short one (phones) */
const SUB_LABEL: Record<SafetySub, { full: MsgKey; short: MsgKey }> = {
  caps: { full: 'ai.set.safeTab.caps', short: 'ai.set.safeTab.capsShort' },
  chat: { full: 'ai.set.safeTab.chat', short: 'ai.set.safeTab.chatShort' },
  study: { full: 'ai.set.safeTab.study', short: 'ai.set.safeTab.studyShort' },
  summary: { full: 'ai.set.safeTab.summary', short: 'ai.set.safeTab.summaryShort' },
  assess: { full: 'ai.set.safeTab.assess', short: 'ai.set.safeTab.assessShort' },
  gen: { full: 'ai.set.safeTab.gen', short: 'ai.set.safeTab.genShort' },
  import: { full: 'ai.set.safeTab.import', short: 'ai.set.safeTab.importShort' },
  server: { full: 'ai.set.safeTab.server', short: 'ai.set.safeTab.serverShort' },
};

// ---------------------------------------------------------------- the sub-tabs

/** The sub-tab shown now: the one in the address, else the last one of this browser session, else the first */
export const currentSub = (): SafetySub => subOfHash(window.location.hash) ?? readSafetySub() ?? 'caps';

/** Makes a sub-tab the current one (address and session memory) without drawing; a field error of another sub-tab is dropped */
function setSub(k: SafetySub): void {
  try {
    window.history.replaceState(window.history.state, '', hashOfSafety(k));
  } catch {
    window.location.hash = hashOfSafety(k);
  }
  writeSafetySub(k);
  if (A.limitsFieldErr && subOfKey(A.limitsFieldErr.key) !== k) A.limitsFieldErr = null;
}

function selectSub(k: SafetySub): void {
  if (k === currentSub() && subOfHash(window.location.hash) === k) return;
  setSub(k);
  redraw();
  document.getElementById(`ais-sub-${k}`)?.focus();
}

/** The row of sub-tabs: full labels on wide screens, short ones on a phone (the full name is the tip and the accessible name) */
function subTabsRow(sub: SafetySub): HTMLElement {
  return h('div', { class: 'seg ais-subtabs', role: 'group', 'aria-label': t('ai.set.safeTab.aria') },
    SAFETY_SUBS.map((k) => {
      const b = h('button', {
        type: 'button', id: `ais-sub-${k}`, 'aria-pressed': String(k === sub), 'aria-controls': 'ais-safety-panel', onclick: () => selectSub(k),
      }, h('span', { class: 'ais-tl-full' }, t(SUB_LABEL[k].full)), h('span', { class: 'ais-tl-short', 'aria-hidden': 'true' }, t(SUB_LABEL[k].short)));
      holdTip(b, t(SUB_LABEL[k].full));
      return b;
    }));
}

// ---------------------------------------------------------------- retention

async function saveRetention(): Promise<void> {
  const s = A.settings;
  if (!s || A.retentionSaving) return;
  const n = retentionOf(A.retentionText);
  if (n == null) {
    A.retentionError = aiCodeText('ai.badRetention');
    redraw();
    return;
  }
  if (n === s.retentionDays) {
    A.retentionError = null;
    redraw();
    toast(t('ai.set.nothing'));
    return;
  }
  A.retentionSaving = true;
  A.retentionError = null;
  redraw();
  try {
    adopt(await saveRetentionApi(n));
    A.retentionText = String(n);
    toast(t('ai.set.retentionSaved', { n }));
    void refreshAi(true);
  } catch (e) {
    if (!aiDown(e)) A.retentionError = errorText(e);
  }
  A.retentionSaving = false;
  redraw();
}

function retentionSection(): HTMLElement {
  const tip = infoTip('ai-retention-tip', t('ai.set.retentionTipBtn'), t('ai.set.retentionTip'));
  const input = h('input', {
    type: 'number', id: 'ai-retention', class: 'ais-num', min: '1', max: '3650', step: '1', inputmode: 'numeric', value: A.retentionText,
    disabled: A.retentionSaving, 'aria-describedby': 'ai-retention-tip',
    oninput: () => {
      A.retentionText = input.value;
    },
  });
  return h('section', { class: 'panel ais-sec', 'aria-labelledby': 'ais-ret-h' },
    h('h2', { id: 'ais-ret-h' }, t('ai.set.sec.retention')),
    h('form', {
      class: 'ais-inline ais-retention', novalidate: true,
      onsubmit: (e: SubmitEvent) => {
        e.preventDefault();
        void saveRetention();
      },
    },
    h('label', { class: 'fld-l', for: 'ai-retention' }, t('ai.set.retentionDays')), tip.el, input,
    h('button', { class: 'btn primary', id: 'ai-retention-save', type: 'submit', disabled: A.retentionSaving }, t('ai.set.save'))),
    A.retentionError ? h('p', { class: 'ais-ferr', role: 'alert' }, A.retentionError) : null);
}

// ---------------------------------------------------------------- loading and saving the limits

export async function loadLimits(): Promise<void> {
  try {
    A.limits = await fetchLimits();
    A.limitsForm = limitsFormOf(A.limits.limits);
    A.limitsError = null;
  } catch (e) {
    if (aiDown(e)) return;
    A.limitsError = e;
  }
  redraw();
}

/** The text under a limit field: the allowed range, or (for the caps) that a longer period cannot allow fewer requests */
function limitFieldText(key: LimitKey, kind: 'range' | 'order' | 'comp', ranges: LimitRanges): string {
  if (kind === 'order') return t('ai.err.limitOrder');
  if (kind === 'comp') return t('ai.set.gen.totalBad');
  const [min, max] = rangeOf(ranges, key);
  return t('ai.set.limitBad', { min: num(min), max: num(max) });
}

export async function saveLimitsForm(tab: 'safety' | 'gen' = 'safety'): Promise<void> {
  A.limitsTab = tab;
  const l = A.limits;
  const f = A.limitsForm;
  if (!l || !f || A.limitsSaving) return;
  const issue = limitIssue(f, l.ranges);
  if (issue) {
    // The field may sit in another sub-tab (only the shown one is in the page): show that one first, then focus the field
    const sub = tab === 'safety' ? subOfKey(issue.key) : null;
    if (sub && sub !== currentSub()) setSub(sub);
    A.limitsFieldErr = { key: issue.key, text: limitFieldText(issue.key, issue.kind, l.ranges) };
    redraw();
    const focusBad = (): void => {
      const bad = document.getElementById(`ai-lim-${issue.key}`);
      if (bad) {
        // No default focus scroll: the pinned top bar would cover the field
        bad.focus({ preventScroll: true });
        bad.scrollIntoView({ block: 'center' });
      }
    };
    window.requestAnimationFrame(focusBad);
    return;
  }
  const u = limitsUpdate(l.limits, f);
  if (!Object.keys(u).length) {
    A.limitsFieldErr = null;
    A.limitsSaveError = null;
    redraw();
    toast(t('ai.set.nothing'));
    return;
  }
  A.limitsSaving = true;
  A.limitsFieldErr = null;
  A.limitsSaveError = null;
  redraw();
  try {
    A.limits = await saveLimits(u);
    A.limitsForm = limitsFormOf(A.limits.limits);
    toast(t(A.limitsTab === 'gen' ? 'ai.set.gen.saved' : 'ai.set.limitsSaved'));
    // The administrator's own block follows the new caps at once: force a status read, not the cached one
    void refreshAi(true);
    // The counts of the users (and their "today / cap" figures) are measured against the new caps
    void loadUsers();
    void loadUsage();
  } catch (e) {
    // The service names the field of a refused value; an order problem without a field belongs under the hourly one
    const named = e instanceof ApiError ? (LIMIT_KEYS as readonly string[]).find((k) => k === e.field) as LimitKey | undefined : undefined;
    const order = e instanceof ApiError && e.code === 'ai.limitOrder';
    const comp = e instanceof ApiError && e.code === 'ai.genCount';
    const key = named ?? (order ? 'callsPerHour' : undefined);
    if (key) {
      const sub = A.limitsTab === 'safety' ? subOfKey(key) : null;
      if (sub && sub !== currentSub()) setSub(sub);
      A.limitsFieldErr = { key, text: limitFieldText(key, comp ? 'comp' : order ? 'order' : 'range', l.ranges) };
    }
    else if (!aiDown(e)) A.limitsSaveError = errorText(e);
  }
  A.limitsSaving = false;
  redraw();
}

/** Fills the defaults of `keys` into the form (not saved until Save); the other tab's fields keep what they hold */
export function resetKeys(keys: readonly LimitKey[]): void {
  if (!A.limits || !A.limitsForm) return;
  const d = limitsFormOf(A.limits.defaults);
  const next = { ...A.limitsForm };
  for (const k of keys) next[k] = d[k];
  A.limitsForm = next;
  A.limitsFieldErr = null;
  redraw();
}

// ---------------------------------------------------------------- field helpers (shared with the AI banks tab)

/** One number field of the limits form (whole numbers; 16 px text on touch comes from the stylesheet) */
export function limitInput(k: LimitKey, bad: boolean, describedBy: string): HTMLInputElement {
  const [min, max] = rangeOf(A.limits?.ranges, k);
  const input = h('input', {
    type: 'number', id: `ai-lim-${k}`, class: 'ais-num', min: String(min), max: String(max), step: '1', inputmode: 'numeric',
    value: A.limitsForm?.[k] ?? '', disabled: A.limitsSaving, 'aria-describedby': describedBy, 'aria-invalid': bad ? 'true' : null,
    oninput: () => {
      if (A.limitsForm) A.limitsForm = { ...A.limitsForm, [k]: input.value };
    },
  });
  return input;
}

/** The default of a limit: the service's own when it sent one, else the spec's */
export const defOf = (l: AiLimitsAnswer, k: LimitKey): number => (l.defaults as LimitValues)[k] ?? DEFAULT_LIMITS[k];

/** A-OWNER-50: either "unlimited" switch also lifts the answer length per call (a shared box), so it greys with them */
function syncCallLength(): void {
  const el = document.getElementById('ai-lim-genCallTokens') as HTMLInputElement | null;
  const f = A.limitsForm;
  if (el && f) el.disabled = A.limitsSaving || f.genUnlimited === '1' || f.importUnlimited === '1';
}

/** A ticked-or-not box of a 0 / 1 limit (kept in the form as '1' / '0') */
export function limitCheck(k: LimitKey, labelKey: MsgKey, note?: MsgKey): HTMLElement {
  const on = A.limitsForm?.[k] === '1';
  const box = h('input', {
    type: 'checkbox', id: `ai-lim-${k}`, checked: on, disabled: A.limitsSaving,
    onchange: () => {
      if (A.limitsForm) A.limitsForm = { ...A.limitsForm, [k]: box.checked ? '1' : '0' };
    },
  });
  return h('div', { class: 'ais-checkrow' },
    h('label', { class: 'ais-check', for: box.id }, box, h('span', null, t(labelKey))),
    note ? h('p', { class: 'fld-hint' }, t(note)) : null);
}

/** One numeric field: label, input, the range / hint line and its error; an "unlimited" switch greys the box (value kept) */
export function limitField(k: LimitKey, l: AiLimitsAnswer, f: Record<LimitKey, string>, hintKey: MsgKey | null, extra: Child[] = [], unlimitedKey?: LimitKey, showSwitch = true): HTMLElement {
  const [min, max] = rangeOf(l.ranges, k);
  const def = defOf(l, k);
  const bad = A.limitsFieldErr?.key === k;
  const input = limitInput(k, bad, `ai-lim-${k}-hint`);
  if (k === 'genCallTokens' && (f.genUnlimited === '1' || f.importUnlimited === '1')) input.disabled = true;
  let sw: HTMLElement | null = null;
  if (unlimitedKey) {
    const uk = unlimitedKey;
    const on = f[uk] === '1';
    input.disabled = A.limitsSaving || on;
    if (showSwitch) {
      const box = h('input', {
        type: 'checkbox', id: `ai-lim-${uk}`, checked: on, disabled: A.limitsSaving,
        onchange: () => {
          if (A.limitsForm) A.limitsForm = { ...A.limitsForm, [uk]: box.checked ? '1' : '0' };
          input.disabled = A.limitsSaving || box.checked;
          syncCallLength();
          // The import switch also covers the recognition box
          if (uk === 'importUnlimited') {
            const rec = document.getElementById('ai-lim-recognizeMaxCalls') as HTMLInputElement | null;
            if (rec) rec.disabled = A.limitsSaving || box.checked;
          }
        },
      });
      sw = h('label', { class: 'ais-check', for: box.id }, box, h('span', null, t('ai.set.limit.unlimited')));
    }
  }
  return h('div', { class: 'fld ais-genfld' },
    h('label', { class: 'fld-l', for: input.id }, t(LIMIT_LABEL[k])),
    sw ? h('div', { class: 'ais-numrow' }, input, sw) : input,
    h('p', { class: 'fld-hint', id: `ai-lim-${k}-hint` }, hintKey ? t(hintKey) + ' ' : '', (() => {
      // v1.4: a token range is shown compact; the exact numbers are in the title and in hidden text
      const a = limitHintArgs(TOKEN_LIMITS.has(k), min, max, def, num);
      const exact = t('ai.set.limitHint', a.exact);
      return a.compact
        ? h('span', { title: exact }, h('span', { 'aria-hidden': 'true' }, t('ai.set.limitHint', a.compact)), h('span', { class: 'sr-only' }, exact))
        : exact;
    })()),
    unlimitedKey && showSwitch ? h('p', { class: 'fld-hint' }, t('ai.set.limit.unlimitedHint')) : null,
    extra,
    bad && A.limitsFieldErr?.text ? h('p', { class: 'ais-ferr', role: 'alert' }, A.limitsFieldErr.text) : null);
}

// ---------------------------------------------------------------- the groups

function group(id: string, title: MsgKey, ...kids: Child[]): HTMLElement {
  return h('div', { class: 'ais-subgroup', role: 'group', 'aria-labelledby': id }, h('h3', { class: 'ais-h3', id }, t(title)), ...kids);
}

/** The per-user caps first: three compact fields in one row, one hint line and the (i) tip with the full rule */
function capsRow(l: AiLimitsAnswer): HTMLElement {
  const tip = infoTip('ai-caps-tip', t('ai.set.caps.tipBtn'), t('ai.set.caps.tip'));
  const bad = CAP_KEYS.find((k) => A.limitsFieldErr?.key === k);
  return h('div', { class: 'ais-caps-wrap' },
    h('div', { class: 'ais-caps' }, CAP_KEYS.map((k) => {
      const [min, max] = rangeOf(l.ranges, k);
      const input = limitInput(k, A.limitsFieldErr?.key === k, `ai-caps-hint ai-caps-tip ai-lim-${k}-range`);
      return h('div', { class: 'fld ais-cap' },
        h('label', { class: 'fld-l', for: input.id }, t(LIMIT_LABEL[k])),
        input,
        h('span', { class: 'sr-only', id: `ai-lim-${k}-range` }, t('ai.set.limitHint', { min: num(min), max: num(max), def: num(defOf(l, k)) })));
    })),
    h('p', { class: 'fld-hint ais-caps-hint', id: 'ai-caps-hint' }, t('ai.set.caps.hint'), tip.el),
    bad ? h('p', { class: 'ais-ferr', role: 'alert' }, A.limitsFieldErr?.text ?? '') : null);
}

async function clearShared(): Promise<void> {
  if (A.shareBusy) return;
  if (!(await ask({ title: t('ai.limits.studyShareClear'), body: t('ai.limits.studyShareClear.ask'), ok: t('ai.set.user.clearOk'), danger: true }))) return;
  A.shareBusy = true;
  redraw();
  try {
    const r = await clearAiData({ what: 'shared' });
    toast(t('ai.limits.studyShareClear.done', { n: num(r.deleted) }));
    void loadUsage();
  } catch (e) {
    if (!aiDown(e)) toast(errorText(e), 6000);
  }
  A.shareBusy = false;
  redraw();
  document.getElementById('ai-share-clear')?.focus();
}

/** v1.9: the provider rate limit of AI generation and import (one setting for every connection) */
function paceGroup(l: AiLimitsAnswer, f: Record<LimitKey, string>): HTMLElement {
  const live = h('p', { class: 'fld-hint ais-live', id: 'ai-pace-live', 'aria-live': 'polite' });
  const offNote = h('p', { class: 'fld-hint', id: 'ai-pace-off' }, t('ai.limits.pace.offNote'));
  const check = limitCheck('paceOn', 'ai.limits.paceOn');
  const fields = h('div', { class: 'ais-limitgrid' }, limitField('paceRpm', l, f, null), limitField('paceTpm', l, f, null));
  const box = check.querySelector<HTMLInputElement>('input');
  const rpm = fields.querySelector<HTMLInputElement>('#ai-lim-paceRpm');
  const tpm = fields.querySelector<HTMLInputElement>('#ai-lim-paceTpm');
  const typed = (k: 'paceRpm' | 'paceTpm'): number | null => {
    const v = A.limitsForm?.[k]?.trim() ?? '';
    const [min, max] = rangeOf(l.ranges, k);
    return /^\d{1,9}$/.test(v) && Number(v) >= min && Number(v) <= max ? Number(v) : null;
  };
  // The grey state and the live line follow the box and the typed numbers without a redraw (the caret stays)
  const sync = (): void => {
    const on = A.limitsForm?.paceOn === '1';
    if (rpm) rpm.disabled = A.limitsSaving || !on;
    if (tpm) tpm.disabled = A.limitsSaving || !on;
    offNote.hidden = on;
    const r = typed('paceRpm');
    const k = typed('paceTpm');
    if (!on || r == null || k == null) live.textContent = '';
    else {
      const e = paceEffective(r, k);
      live.textContent = t('ai.limits.pace.live', { rpm: num(e.rpm), tpm: num(e.tpm), sec: String(e.sec) });
    }
  };
  box?.addEventListener('change', sync);
  fields.addEventListener('input', sync);
  sync();
  return group('ais-g-pace', 'ai.limits.group.pace', h('p', { class: 'fld-hint' }, t('ai.limits.pace.note')), check, fields, live, offNote);
}

/** The groups of one sub-tab (the same groups the single page had before v1.9) */
function subGroups(sub: SafetySub, l: AiLimitsAnswer, f: Record<LimitKey, string>): Child[] {
  switch (sub) {
    case 'chat':
      return [group('ais-g-chat', 'ai.limits.group.chat',
        h('div', { class: 'ais-limitgrid' }, (['chatsPerUser', 'messagesPerChat', 'memoriesPerUser', 'memoryTokens'] as const).map((k) => limitField(k, l, f, null))),
        h('p', { class: 'fld-hint' }, t('ai.limits.memoryHint')))];
    case 'study':
      return [group('ais-g-study', 'ai.limits.group.study',
        limitCheck('studyShare', 'ai.limits.studyShare'),
        limitField('studyShareDays', l, f, null),
        h('div', { class: 'acts' },
          h('button', { class: 'btn danger', id: 'ai-share-clear', type: 'button', disabled: A.shareBusy || A.limitsSaving, onclick: () => void clearShared() }, t('ai.limits.studyShareClear'))))];
    case 'summary':
      return [group('ais-g-summary', 'ai.limits.group.summary', limitField('summaryMaxItems', l, f, null))];
    case 'assess':
      return [group('ais-g-assess', 'ai.limits.group.assess',
        h('div', { class: 'ais-genrow' },
          limitField('assessPerDay', l, f, 'ai.set.limit.assessPerDayHint'),
          limitCheck('assessAdminsExempt', 'ai.set.limit.assessAdminsExempt')))];
    case 'gen': {
      const live = h('p', { class: 'fld-hint ais-live', id: 'ai-lim-genCallTokens-live', 'aria-live': 'polite' });
      const liveText = (text: string): string => {
        const v = genCallLive(text, l.ranges);
        return v ? t('ai.set.limit.genCallLive', { q: v.q }) : '';
      };
      live.textContent = liveText(f.genCallTokens);
      // the live line follows the typed length without a redraw (only while this sub-tab is shown)
      const lengthField = limitField('genCallTokens', l, f, 'ai.set.limit.genCallTokensHint', [live]);
      lengthField.addEventListener('input', (e) => {
        if ((e.target as HTMLElement).id === 'ai-lim-genCallTokens') live.textContent = liveText((e.target as HTMLInputElement).value);
      });
      return [group('ais-g-gen', 'ai.limits.group.gen',
        h('div', { class: 'ais-genrow' },
          limitField('genPerDay', l, f, 'ai.set.limit.genPerDayHint'),
          limitCheck('genAdminsExempt', 'ai.set.limit.genAdminsExempt')),
        lengthField,
        limitField('genMaxCalls', l, f, 'ai.set.limit.genMaxCallsHint', [], 'genUnlimited'),
        limitField('genMaxChunks', l, f, null),
        h('p', { class: 'ais-note' }, t('ai.set.imp.shared')))];
    }
    case 'import':
      return [group('ais-g-import', 'ai.limits.group.import',
        h('div', { class: 'ais-genrow' },
          limitField('importMaxMb', l, f, 'ai.set.limit.importMaxMbHint'),
          limitField('importMaxQuestions', l, f, 'ai.set.limit.importMaxQuestionsHint')),
        limitField('recognizeMaxCalls', l, f, 'ai.set.limit.recognizeMaxCallsHint', [], 'importUnlimited', false),
        limitField('importMaxCalls', l, f, 'ai.set.limit.importMaxCallsHint', [], 'importUnlimited'),
        h('p', { class: 'ais-note' }, t('ai.set.imp.shared')))];
    case 'server':
      return [
        group('ais-g-server', 'ai.limits.group.running',
          h('div', { class: 'ais-limitgrid' }, (['perUserRunning', 'globalRunning', 'genRunning'] as const).map((k) => limitField(k, l, f, null)))),
        paceGroup(l, f),
      ];
    default: {
      const lengthTip = infoTip('ai-length-tip', t('ai.set.limits.lengthTipBtn'), t('ai.set.limits.lengthTip'));
      return [
        group('ais-g-calls', 'ai.limits.group.calls',
          capsRow(l),
          limitCheck('callsAdminsExempt', 'ai.limits.callsAdminsExempt', 'ai.limits.callsAdminsExempt.note'),
          limitField('dailyTokenBudget', l, f, null, [h('p', { class: 'fld-hint' }, t('ai.limits.dailyTokenBudget.note'))])),
        h('div', { class: 'ais-subgroup ais-length-group', role: 'group', 'aria-labelledby': 'ais-length-h' },
          h('h3', { class: 'ais-h3', id: 'ais-length-h' }, t('ai.set.limits.length'), lengthTip.el),
          limitField('replyMaxTokens', l, f, null, [h('p', { class: 'fld-hint' }, t('ai.limits.replyNote'))])),
      ];
    }
  }
}

function limitsSection(sub: SafetySub): HTMLElement {
  const l = A.limits;
  const f = A.limitsForm;
  let body: Child;
  if (!l || !f) {
    body = A.limitsError
      ? h('div', { class: 'err', role: 'alert' }, h('p', null, t('ai.set.limitsErr', { reason: errorText(A.limitsError) })))
      : h('p', { class: 'dim', role: 'status' }, t('common.loading'));
  } else {
    body = h('form', {
      class: 'ais-limits', novalidate: true, 'aria-busy': A.limitsSaving ? 'true' : null,
      onsubmit: (e: SubmitEvent) => {
        e.preventDefault();
        void saveLimitsForm('safety');
      },
    },
    ...subGroups(sub, l, f),
    A.limitsSaveError ? h('div', { class: 'err', role: 'alert' }, h('p', null, A.limitsSaveError)) : null,
    h('div', { class: 'acts' },
      h('button', { class: 'btn primary', id: 'ai-limits-save', type: 'submit', disabled: A.limitsSaving }, t(A.limitsSaving ? 'common.saving' : 'ai.set.save')),
      h('button', { class: 'btn', id: 'ai-limits-reset', type: 'button', disabled: A.limitsSaving, onclick: () => resetKeys(SUB_KEYS[sub]) }, t('ai.set.limit.resetSub'))));
  }
  return h('section', { class: 'panel ais-sec', 'aria-labelledby': 'ais-lim-h' },
    h('h2', { id: 'ais-lim-h' }, t('ai.set.sec.limits')),
    sub === 'caps' ? h('p', { class: 'ais-lead' }, t('ai.set.limitsLeadV10')) : null,
    body);
}

export function safetySections(): HTMLElement[] {
  const sub = currentSub();
  // The chat retention is a server matter: it sits under the Server sub-tab's limits
  return [subTabsRow(sub), h('div', { id: 'ais-safety-panel' }, limitsSection(sub), sub === 'server' ? retentionSection() : null)];
}
