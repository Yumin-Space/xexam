// AI banks tab, the recent jobs table: the text of one job (source, state) and the list of fields of its detail block. Pure
// (no DOM), so it is tested on its own; the page is pages/ai-settings/gen.ts.
import { jobReasonText } from '../i18n/errors.ts';
import { localeTag, t, type MsgKey } from '../i18n/index.ts';
import type { GenAdminJob } from '../types/api.ts';
import { jobTime } from './ai-settings.ts';

/** Display order of the states (the state sort: running first) */
const STATE_ORDER = ['running', 'queued', 'done', 'failed', 'cancelled'];

export const stateRank = (j: { state: string }): number => { const i = STATE_ORDER.indexOf(j.state); return i < 0 ? STATE_ORDER.length : i; };

const num = (n: number): string => n.toLocaleString(localeTag());

export const sourceText = (j: GenAdminJob): string =>
  j.source === 'file' ? t('ai.set.gen.source.file', { type: (j.file?.type ?? '').toUpperCase() }) : t(`ai.set.gen.source.${j.source}` as MsgKey);

export const stateText = (j: GenAdminJob): string =>
  t(`ai.set.gen.state.${j.state}` as MsgKey) + (j.state === 'running' && (j.phase === 'reading' || j.phase === 'confirm') ? ` · ${t(`ai.set.gen.phase.${j.phase}` as MsgKey)}` : '')
  // v1.9: the job waits for the provider's quota
  + ((j.state === 'queued' || j.state === 'running') && j.waiting ? ` · ${t('ai.set.gen.phase.waiting')}` : '');

const fileSize = (n: number): string => (n >= 1048576 ? `${(n / 1048576).toFixed(1)} MB` : n >= 1024 ? `${Math.round(n / 1024)} KB` : `${n} B`);

/** One line of the detail block: a label key and the value (only fields the job really carries are listed) */
export function jobDetailRows(j: GenAdminJob): Array<[MsgKey, string]> {
  const rows: Array<[MsgKey, string]> = [];
  const add = (k: MsgKey, v: string | null): void => { if (v) rows.push([k, v]); };
  const isFile = j.source === 'file';
  add('ai.set.gen.d.user', j.username);
  add('ai.set.gen.d.kind', t(isFile ? 'ai.set.gen.d.kind.import' : 'ai.set.gen.d.kind.gen'));
  add('ai.set.gen.col.source', sourceText(j));
  if (j.from.length > 0) {
    const more = j.fromCount - j.from.length;
    add('ai.set.gen.d.from', j.from.map((f) => f.name).join(', ') + (more > 0 ? ` · ${t('ai.set.gen.d.more', { n: num(more) })}` : ''));
  }
  if (j.file) add('ai.set.gen.d.file', `${j.file.name} (${j.file.type.toUpperCase()}, ${fileSize(j.file.size)})`);
  if (j.mode) add('ai.set.gen.d.mode', t(`ai.set.gen.d.mode.${j.mode}` as MsgKey));
  const found = j.total ?? j.found ?? null;
  if (found != null) add('ai.set.gen.d.found', j.approx ? t('ai.set.gen.d.about', { n: num(found) }) : num(found));
  if (j.taken != null) add('ai.set.gen.d.taken', num(j.taken));
  add('ai.set.gen.d.made', `${num(j.made)} / ${num(j.goal)}`);
  if (j.skipped && j.skipped.length > 0) add('ai.set.gen.d.skipped', j.skipped.join(', '));
  add('ai.set.gen.d.calls', num(j.calls));
  add('ai.set.gen.d.tin', num(j.tokensIn));
  add('ai.set.gen.d.tout', num(j.tokensOut));
  const cr = j.usage?.tokensCached ?? 0;
  const cw = j.usage?.tokensCacheWrite ?? 0;
  if (cr > 0 || cw > 0) add('ai.set.gen.d.cached', `${num(cr)} / ${num(cw)}`);
  if (j.chunks != null && j.chunks > 0) add('ai.set.gen.d.chunks', num(j.chunks));
  if (j.prefixTokens != null && j.prefixTokens > 0) add('ai.set.gen.d.prefix', num(j.prefixTokens));
  add('ai.set.gen.d.created', jobTime(j.createdAt));
  if (j.startedAt != null) add('ai.set.gen.d.started', jobTime(j.startedAt));
  if (j.finishedAt != null) add('ai.set.gen.d.finished', jobTime(j.finishedAt));
  add('ai.set.gen.d.state', stateText(j));
  if (j.state === 'failed' || (j.state === 'cancelled' && j.error)) add('ai.set.gen.d.reason', jobReasonText(j));
  const bank = j.state === 'done' ? j.name : null;
  if (bank) add('ai.set.gen.d.bank', j.bankId != null ? `${bank} (#${j.bankId})` : bank);
  return rows;
}
