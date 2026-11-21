// AI settings page, AI banks tab: the content rules of AI question generation (the smallest source bank, the smallest number of wrong
// answers, the composition of single / multiple-choice questions per mode) with its own Save, and the recent AI bank jobs.
// The quotas, call ceilings, chunks and import limits are on the Safety tab.
import { ADMIN_GEN_MAX, fetchAdminGen } from '../../api/ai-gen.ts';
import { pagedTable, type PtColumn } from '../../components/paged-table.ts';
import { holdTip, infoTip } from '../../components/tooltip.ts';
import { miniIcon } from '../../components/mini-icons.ts';
import { compBad, compTotal, BANK_KEYS, jobTime, type LimitKey } from '../../domain/ai-settings.ts';
import { tokenLabel } from '../../domain/usage-bars.ts';
import { errorText, jobReasonText } from '../../i18n/errors.ts';
import { localeTag, t, type MsgKey } from '../../i18n/index.ts';
import type { GenAdminJob } from '../../types/api.ts';
import { h, type Child } from '../../utils/dom.ts';
import { limitField, limitInput, resetKeys, saveLimitsForm } from './safety.ts';
import { aiDown } from './status.ts';
import { A, num, redraw } from './state.ts';
import { jobDetailRows, sourceText, stateRank, stateText } from '../../domain/gen-jobs.ts';

export async function loadGenJobs(): Promise<void> {
  try {
    A.genJobs = await fetchAdminGen(ADMIN_GEN_MAX);
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

/** Jobs whose detail block is open; kept here so paging, sorting and redraws do not close them */
const openJobs = new Set<number>();

function detailBlock(j: GenAdminJob): HTMLElement {
  return h('dl', { class: 'ais-gendl' }, jobDetailRows(j).map(([k, v]) => h('div', { class: 'ais-gendl-r' }, h('dt', null, t(k)), h('dd', null, v))));
}

function genJobsTable(): HTMLElement {
  if (!A.genJobs) {
    return A.genJobsError
      ? h('div', { class: 'err', role: 'alert' }, h('p', null, errorText(A.genJobsError)))
      : h('p', { class: 'dim', role: 'status' }, t('common.loading'));
  }
  if (!A.genJobs.length) return h('p', { class: 'empty' }, t('ai.set.gen.jobsEmpty'));
  // the line's holdTip is the place of the exact numbers, so no title here
  const big = (n: number): HTMLElement => { const l = tokenLabel(n, num); return h('span', null, h('span', { 'aria-hidden': 'true' }, l.text), h('span', { class: 'sr-only' }, l.title)); };
  const renderRow = (j: GenAdminJob): HTMLElement[] => {
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
    const note = why || skipText
      ? h('tr', { role: 'row', class: 'ais-gendetail ' + j.state },
        h('td', { role: 'cell', colspan: '8' },
          why ? h('span', { class: 'ais-genwhy-w' }, why) : null,
          skipText ? h('span', { class: 'ais-skipn-w' }, skipText) : null))
      : null;
    // v2.2: the details toggle opens a block with every field of the job (all widths)
    const open = openJobs.has(j.id);
    const full = h('tr', { role: 'row', class: 'ais-genfull', hidden: !open },
      h('td', { role: 'cell', colspan: '8' }, detailBlock(j)));
    const label = (on: boolean): string => `${t(on ? 'ai.set.gen.d.hide' : 'ai.set.gen.d.show')} · ${t('ai.set.gen.d.aria', { user: j.username, time: jobTime(j.createdAt) })}`;
    const toggle = h('button', {
      type: 'button', class: 'ais-genmore-b' + (open ? ' on' : ''), 'aria-expanded': open ? 'true' : 'false',
      'aria-label': label(open), title: t(open ? 'ai.set.gen.d.hide' : 'ai.set.gen.d.show'),
      onclick: () => {
        const on = !openJobs.has(j.id);
        if (on) openJobs.add(j.id); else openJobs.delete(j.id);
        full.hidden = !on;
        toggle.setAttribute('aria-expanded', on ? 'true' : 'false');
        toggle.classList.toggle('on', on);
        toggle.setAttribute('aria-label', label(on));
        toggle.title = t(on ? 'ai.set.gen.d.hide' : 'ai.set.gen.d.show');
      },
    }, h('span', { class: 'ais-genchev', 'aria-hidden': 'true' }));
    const row = h('tr', { role: 'row', class: 'ais-genjob ' + j.state + (note ? ' has-detail' : '') },
      h('td', { role: 'cell', class: 'ais-when', 'data-l': t('ai.set.gen.col.at') }, jobTime(j.createdAt)),
      h('th', { scope: 'row', role: 'rowheader', class: 'ais-uname' }, j.username),
      h('td', { role: 'cell', class: 'ais-gensrc', 'data-l': t('ai.set.gen.col.source') }, sourceText(j)),
      h('td', { role: 'cell', class: 'ais-genname', 'data-l': t('ai.set.gen.col.name') },
        h('span', { class: 'ais-srcm' }, sourceText(j)), h('span', { class: j.state === 'done' ? 'ais-nm' : 'ais-nm dim' }, nameText)),
      h('td', { role: 'cell', class: 'ais-genst', 'data-l': t('ai.set.gen.col.state') }, h('span', { class: 'ais-jchip ' + j.state }, stateText(j)), whyEl),
      h('td', { role: 'cell', class: 'n ais-genmade', 'data-l': t('ai.set.gen.col.made') }, `${num(j.made)} / ${num(j.goal)}`,
        skipText ? h('span', { class: 'ais-skipn' }, skipText) : null),
      h('td', { role: 'cell', class: 'n ais-gencalls', 'data-l': t('ai.set.gen.col.calls') }, stats),
      h('td', { role: 'cell', class: 'ais-genmore' }, toggle));
    return note ? [row, note, full] : [row, full];
  };
  const c = (key: string, label: MsgKey, cls = '', text = false): PtColumn => ({ key, label: t(label), cls, text });
  const plain = (key: string, label: MsgKey, cls = ''): PtColumn => ({ key, label: t(label), cls, plain: true });
  return pagedTable<GenAdminJob>({
    id: 'ai-genjobs', rows: A.genJobs, name: (j) => j.username, cardsAt: 900, roles: true, tableClass: 'ais-table ais-genjobs',
    defaultSort: { key: 'at', dir: 'desc' },
    columns: [
      c('at', 'ai.set.gen.col.at'), c('user', 'ai.set.gen.col.user', '', true), plain('source', 'ai.set.gen.col.source'), plain('name', 'ai.set.gen.col.name'),
      c('state', 'ai.set.gen.col.state'), plain('made', 'ai.set.gen.col.made', 'n'), plain('calls', 'ai.set.gen.col.calls', 'n'),
      { key: 'more', label: t('ai.set.gen.d.show'), cls: 'ais-genmore', plain: true, srOnly: true },
    ],
    cmp: {
      at: (a, b) => a.createdAt - b.createdAt || a.id - b.id,
      user: (a, b) => a.username.localeCompare(b.username, localeTag()),
      // The first click on a non-text column is descending: running first, then waiting, done, failed, cancelled;
      // inside one state the newest job comes first
      state: (a, b) => stateRank(b) - stateRank(a) || a.createdAt - b.createdAt || a.id - b.id,
    },
    renderRow,
  });
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
