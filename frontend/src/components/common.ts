import { t } from '../i18n/index.ts';
import type { Mode, Question, RecordBank } from '../types/api.ts';
import { bankById, S } from '../state/store.ts';
import { h } from '../utils/dom.ts';
import { aiOfBank } from '../domain/ai-gen.ts';
import { aiSetBadge } from './ai-badge.ts';

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

/** The logo (v0.8: the app icon, /icon.svg) and the name; used by the top bar, the loading page and the sign-in page */
export function brand(): HTMLElement {
  return h('div', { class: 'brand' },
    h('img', { class: 'logo', src: '/icon.svg', alt: '', width: '28', height: '28', 'aria-hidden': 'true' }),
    h('span', { class: 'brand-name' }, t('app.name')));
}

/** A page heading block: title (h1) and optional actions on the right */
export function pageHead(title: string, ...actions: Array<HTMLElement | null>): HTMLElement {
  return h('header', { class: 'page-head' }, h('h1', null, title), actions.some(Boolean) ? h('div', { class: 'page-acts' }, actions) : null);
}

/** The AI+题集 marker for a question's bank (A-OWNER-27): a label only, shown in every mode including an exam; null for an ordinary bank */
export function aiSetMark(bankId: number): HTMLElement | null {
  const ai = aiOfBank(S.banks, bankId);
  return ai ? aiSetBadge(ai) : null;
}
