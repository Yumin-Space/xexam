// AI settings page, AI banks tab: the content rules of AI question generation (the smallest source bank, the smallest number of wrong
// answers, the composition of single / multiple-choice questions per mode) with its own Save, and the recent AI bank jobs.
// The quotas, call ceilings, chunks and import limits are on the Safety tab.
import { fetchAdminGen } from '../../api/ai-gen.ts';
import { holdTip, infoTip } from '../../components/tooltip.ts';
import { miniIcon } from '../../components/mini-icons.ts';
import { compBad, compTotal, BANK_KEYS, jobTime, type LimitKey } from '../../domain/ai-settings.ts';
import { tokenLabel } from '../../domain/usage-bars.ts';
import { errorText, jobReasonText } from '../../i18n/errors.ts';
import { t, type MsgKey } from '../../i18n/index.ts';
import type { GenAdminJob } from '../../types/api.ts';
import { h, type Child } from '../../utils/dom.ts';
import { limitField, limitInput, resetKeys, saveLimitsForm } from './safety.ts';
import { aiDown } from './status.ts';
import { A, num, redraw } from './state.ts';

export async function loadGenJobs(): Promise<void> {
  try {
    A.genJobs = await fetchAdminGen(50);
    A.genJobsError = null;
  } catch (e) {
    if (aiDown(e)) return;
    A.genJobsError = e;
  }
  redraw();
}

/** `单选 [14] 多选 [6] · 共 20 道` of one mode, the total live (red with the rule's text outside 1 .. 100) */
function compRow(mode: 'bank' | 'wrong', f: Record<LimitKey, string>): HTMLElement {
  const sk: LimitKey = mode === 'bank' ? 'genBankSingle' : 'genWrongSingle';
  const mk: LimitKey = mode === 'bank' ? 'genBankMulti' : 'genWrongMulti';
  const sIn = limitInput(sk, A.limitsFieldErr?.key === sk, `ai-comp-${mode}-h`);
  const mIn = limitInput(mk, A.limitsFieldErr?.key === mk, `ai-comp-${mode}-h`);
  sIn.setAttribute('aria-label', t('ai.set.limit.genSingle'));
  mIn.setAttribute('aria-label', t('ai.set.limit.genMulti'));
  const total = h('span', { class: 'ais-comp-total', id: `ai-comp-total-${mode}`, 'aria-live': 'polite' });
  const sync = (): void => {
    const v = A.limitsForm ?? f;
    const bad = compBad(v[sk], v[mk]);
    const n = compTotal(v[sk], v[mk]);
    const el = document.getElementById(`ai-comp-total-${mode}`) ?? total;
    el.classList.toggle('bad', bad);
    el.textContent = Number.isNaN(n) ? t('ai.set.gen.totalBad') : bad ? `${t('ai.set.gen.total', { n })} · ${t('ai.set.gen.totalBad')}` : t('ai.set.gen.total', { n });
  };
  for (const input of [sIn, mIn]) input.addEventListener('input', sync);
  sync();
  return h('div', { class: 'ais-comp', role: 'group', 'aria-labelledby': `ai-comp-${mode}-h` },
    h('div', { class: 'ais-comp-head' },
      h('span', { class: 'ais-comp-l', id: `ai-comp-${mode}-h` }, t(mode === 'bank' ? 'ai.set.gen.compBank' : 'ai.set.gen.compWrong')),
      h('span', { class: 'fld-hint ais-comp-hint' }, t(mode === 'bank' ? 'ai.set.gen.compBankHint' : 'ai.set.gen.compWrongHint'))),
    h('div', { class: 'ais-comp-fields' },
      // A-OWNER-87: short visible labels keep the row on one line; the full names stay the inputs' accessible names
      h('label', { class: 'ais-comp-f', for: sIn.id }, h('span', { 'aria-hidden': 'true' }, t('ai.set.limit.genSingleShort')), sIn),
      h('label', { class: 'ais-comp-f', for: mIn.id }, h('span', { 'aria-hidden': 'true' }, t('ai.set.limit.genMultiShort')), mIn),
      total));
}

function genJobsTable(): HTMLElement {
  if (!A.genJobs) {
    return A.genJobsError
      ? h('div', { class: 'err', role: 'alert' }, h('p', null, errorText(A.genJobsError)))
      : h('p', { class: 'dim', role: 'status' }, t('common.loading'));
  }
  if (!A.genJobs.length) return h('p', { class: 'ais-note' }, t('ai.set.gen.jobsEmpty'));
  const col = (k: MsgKey, cls = ''): HTMLElement => h('th', { scope: 'col', class: cls || null }, t(k));
  const state = (j: GenAdminJob): string =>
    t(`ai.set.gen.state.${j.state}` as MsgKey) + (j.state === 'running' && (j.phase === 'reading' || j.phase === 'confirm') ? ` · ${t(`ai.set.gen.phase.${j.phase}` as MsgKey)}` : '');
  const source = (j: GenAdminJob): string =>
    j.source === 'file' ? t('ai.set.gen.source.file', { type: (j.file?.type ?? '').toUpperCase() }) : t(`ai.set.gen.source.${j.source}` as MsgKey);
  // the line's holdTip is the place of the exact numbers, so no title here
  const big = (n: number): HTMLElement => { const l = tokenLabel(n, num); return h('span', null, h('span', { 'aria-hidden': 'true' }, l.text), h('span', { class: 'sr-only' }, l.title)); };
  const rows = A.genJobs.map((j) => {
    const why = j.state === 'failed' || (j.state === 'cancelled' && j.error) ? jobReasonText(j) : null;
    const whyEl = why ? holdTip(h('span', { class: 'ais-genwhy', tabindex: '0' }, h('span', { class: 'ais-d' }, ' — '), why), why) : null;
    const cr = j.usage?.tokensCached ?? 0;
    const cw = j.usage?.tokensCacheWrite ?? 0;
    const tip = t('ai.set.gen.stats.tip', { calls: num(j.calls), tin: num(j.tokensIn), tout: num(j.tokensOut) })
      + (cr > 0 || cw > 0 ? t('ai.set.usage.sumCached', { w: num(cw), r: num(cr) }) : '');
    // L-55: icons carry screen-reader names; the whole figure line has one tip (hover / focus / hold)
    const stats = holdTip(h('span', { class: 'ais-stats', tabindex: '0' },
      h('span', { class: 'ais-st' }, miniIcon('refresh', 'ais-ji'), h('span', { class: 'sr-only' }, t('ai.set.gen.stats.calls') + ' '), num(j.calls)),
      h('span', { class: 'ais-st' }, h('span', { class: 'ais-arr', 'aria-hidden': 'true' }, '↓'), h('span', { class: 'sr-only' }, t('ai.set.gen.stats.in') + ' '), big(j.tokensIn)),
      h('span', { class: 'ais-st' }, h('span', { class: 'ais-arr', 'aria-hidden': 'true' }, '↑'), h('span', { class: 'sr-only' }, t('ai.set.gen.stats.out') + ' '), big(j.tokensOut))), tip);
    const nameText = j.state === 'done' ? j.name : (j.file?.name ?? j.name);
    const skipText = j.skipped && j.skipped.length > 0 ? t('ai.set.gen.skipped', { n: j.skipped.length, list: j.skipped.join(',') }) : null;
    // v1.2 (owner): on wide screens the failure reason and the skipped note go on one full-width line under the job
    // (like the phone card) instead of squeezing the state and count columns; phones keep them inside the card
    const detail = why || skipText
      ? h('tr', { class: 'ais-gendetail ' + j.state },
        h('td', { colspan: '7' },
          why ? h('span', { class: 'ais-genwhy-w' }, why) : null,
          skipText ? h('span', { class: 'ais-skipn-w' }, skipText) : null))
      : null;
    const row = h('tr', { class: 'ais-genjob ' + j.state + (detail ? ' has-detail' : '') },
      h('td', { class: 'ais-when', 'data-l': t('ai.set.gen.col.at') }, jobTime(j.createdAt)),
      h('th', { scope: 'row', class: 'ais-uname' }, j.username),
      h('td', { class: 'ais-gensrc', 'data-l': t('ai.set.gen.col.source') }, source(j)),
      h('td', { class: 'ais-genname', 'data-l': t('ai.set.gen.col.name') },
        h('span', { class: 'ais-srcm' }, source(j)), h('span', { class: j.state === 'done' ? 'ais-nm' : 'ais-nm dim' }, nameText)),
      h('td', { class: 'ais-genst', 'data-l': t('ai.set.gen.col.state') }, h('span', { class: 'ais-jchip ' + j.state }, state(j)), whyEl),
      h('td', { class: 'n ais-genmade', 'data-l': t('ai.set.gen.col.made') }, `${num(j.made)} / ${num(j.goal)}`,
        skipText ? h('span', { class: 'ais-skipn' }, skipText) : null),
      h('td', { class: 'n ais-gencalls', 'data-l': t('ai.set.gen.col.calls') }, stats));
    return detail ? [row, detail] : [row];
  }).flat();
  return h('div', { class: 'ais-tablewrap' }, h('table', { class: 'ais-table ais-genjobs' },
    h('thead', null, h('tr', null, col('ai.set.gen.col.at'), col('ai.set.gen.col.user'), col('ai.set.gen.col.source'), col('ai.set.gen.col.name'),
      col('ai.set.gen.col.state'), col('ai.set.gen.col.made', 'n'), col('ai.set.gen.col.calls', 'n'))),
    h('tbody', null, rows)));
}

/** The fifth tab, "AI banks": the content rules, one Save, the recent jobs */
export function genSection(): HTMLElement[] {
  const l = A.limits;
  const f = A.limitsForm;
  if (!l || !f) {
    return [h('section', { class: 'panel ais-sec' }, A.limitsError
      ? h('div', { class: 'err', role: 'alert' }, h('p', null, t('ai.set.limitsErr', { reason: errorText(A.limitsError) })))
      : h('p', { class: 'dim', role: 'status' }, t('common.loading')))];
  }
  const card = (id: string, title: MsgKey, ...kids: Child[]): HTMLElement =>
    h('section', { class: 'panel ais-sec ais-gencard', 'aria-labelledby': id }, h('h2', { id }, t(title)), ...kids);
  const sub = (id: string, title: MsgKey, ...kids: Child[]): HTMLElement =>
    h('div', { class: 'ais-subcard', role: 'group', 'aria-labelledby': id }, h('h3', { class: 'ais-subh', id }, t(title)), ...kids);
  const tip = infoTip('ai-gen-tip', t('ai.set.tab.gen'), `${t('ai.set.gen.howTip')}

${t('ai.set.imp.howTip')}`, 'ais-pre');
  const form = h('form', {
    class: 'ais-genform', novalidate: true, 'aria-busy': A.limitsSaving ? 'true' : null,
    onsubmit: (e: SubmitEvent) => {
      e.preventDefault();
      void saveLimitsForm('gen');
    },
  },
  card('ais-gen-h', 'ai.set.gen.card',
    sub('ais-sub-source', 'ai.set.sub.source',
      h('div', { class: 'ais-genrow' },
        limitField('genMinSource', l, f, 'ai.set.limit.genMinSourceHint'),
        limitField('genMinWrong', l, f, null))),
    sub('ais-sub-comp', 'ai.set.gen.comp', compRow('bank', f), compRow('wrong', f))),
  A.limitsSaveError ? h('div', { class: 'err', role: 'alert' }, h('p', null, A.limitsSaveError)) : null,
  h('div', { class: 'acts ais-genacts' },
    h('button', { class: 'btn primary', id: 'ai-gen-save', type: 'submit', disabled: A.limitsSaving }, t(A.limitsSaving ? 'common.saving' : 'ai.set.save')),
    h('button', { class: 'btn', id: 'ai-gen-reset', type: 'button', disabled: A.limitsSaving, onclick: () => resetKeys(BANK_KEYS) }, t('ai.set.limitsReset'))));
  const jobs = h('section', { class: 'panel ais-sec', 'aria-labelledby': 'ais-gen-jobs-h' },
    h('h2', { id: 'ais-gen-jobs-h' }, t('ai.set.gen.jobs')), genJobsTable());
  return [h('div', { class: 'ais-gentitle' }, h('h2', { class: 'ais-gen-h' }, t('ai.set.tab.gen')), tip.el), form, jobs];
}
