import { t } from '../i18n/index.ts';
import type { Mode, Question, RecordBank } from '../types/api.ts';
import { bankById } from '../state/store.ts';
import { h } from '../utils/dom.ts';

const MODE_KEYS = { all: 'mode.all', weak: 'mode.weak', retry: 'mode.retry' } as const;

export function modeName(m: Mode | string): string {
  return m in MODE_KEYS ? t(MODE_KEYS[m as Mode]) : '';
}

export const kindLabel = (q: Pick<Question, 'kind'>): string =>
  t(q.kind === 'hotspot' ? 'kind.hotspot' : q.kind === 'multi' ? 'kind.multi' : 'kind.single');

/** Display name of a bank id (current name; "unknown" when it no longer exists) */
export function bankName(id: number): string {
  return bankById(id)?.name ?? t('common.unknownBank');
}

/** Names of the banks of an exam record, joined; deleted banks are marked */
export function recordBankNames(banks: readonly RecordBank[]): string {
  if (!banks.length) return t('common.none');
  return banks.map((b) => (b.deleted ? t('common.deletedBank', { name: b.name }) : b.name)).join(t('common.listSep'));
}

export function brand(): HTMLElement {
  return h('div', { class: 'brand' },
    h('span', { class: 'logo', 'aria-hidden': 'true' }, h('i', { class: 'on' }), h('i'), h('i', { class: 'on' }), h('i')),
    h('span', { class: 'brand-name' }, t('app.name')));
}

/** A page heading block: title (h1) and optional actions on the right */
export function pageHead(title: string, ...actions: Array<HTMLElement | null>): HTMLElement {
  return h('header', { class: 'page-head' }, h('h1', null, title), actions.some(Boolean) ? h('div', { class: 'page-acts' }, actions) : null);
}
