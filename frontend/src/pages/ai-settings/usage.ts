// AI settings page, Usage tab: calls and tokens by user (with today's count and the cap), by model (rows from before v1.3 have no
// connection and show a dash in the hidden table), by feature and over time (no chart library: CSS bars),
// the shared hint / explanation store (stored answers, reuses) and the site-wide daily budget when one is set.
import { fetchUsage } from '../../api/ai-admin.ts';
import { pagedTable, type PtColumn } from '../../components/paged-table.ts';
import { usageBarCard, usageLineCard } from '../../components/usage-charts.ts';
import { barPercent, connLabel, convTotals, todayCell } from '../../domain/ai-settings.ts';
import { formatBig, modelRows, purposeRows } from '../../domain/usage-bars.ts';
import { errorText } from '../../i18n/errors.ts';
import { localeTag, t, tn, type MsgKey } from '../../i18n/index.ts';
import type { AiUsage } from '../../types/api.ts';
import { h, type Child } from '../../utils/dom.ts';
import { aiDown } from './status.ts';
import { A, num, redraw, when } from './state.ts';

const PERIODS = [7, 30, 90, 365] as const;
/** The six features of the usage chart (UC-1), in the order of the bars */
const PURPOSES = ['chat', 'hint', 'explain', 'summary', 'generate', 'import'] as const;

export async function loadUsage(): Promise<void> {
  const days = A.usageDays;
  try {
    const u = await fetchUsage(days);
    if (days !== A.usageDays) return;
    A.usage = u;
    A.usageError = null;
  } catch (e) {
    if (aiDown(e)) return;
    A.usageError = e;
  }
  redraw();
}


type Counts = { calls: number; tokensIn: number; tokensOut: number; errors: number; tokensCached: number; tokensCacheWrite?: number };

/** A-OWNER-84: "Cache write/read: 6.9K/76.3K" (the same pair as the live token text; missing figures count as 0) */
function cacheText(c: { tokensCached?: number; tokensCacheWrite?: number }): string {
  return t('ai.set.cacheLine', { w: formatBig(c.tokensCacheWrite ?? 0), r: formatBig(c.tokensCached ?? 0) });
}

/** The five count cells of a usage row (hidden in the card layout, where `sumCell` says the same in one line) */
function countsCells(c: Counts, mini = false): HTMLElement[] {
  // The mini table of phones (A-OWNER-59) shows compact figures (1.2K, 3.4M); the exact number stays in the tip and the wide table
  const cell = (n: number): HTMLElement => (mini
    ? h('td', { class: 'n ais-c', title: num(n) }, h('span', { class: 'ais-x' }, num(n)), h('span', { class: 'ais-k', 'aria-label': num(n) }, formatBig(n)))
    : h('td', { class: 'n ais-c' }, num(n)));
  return [cell(c.calls), cell(c.tokensIn), cell(c.tokensOut), cell(c.tokensCached), cell(c.errors)];
}

const countsHead = (): HTMLElement[] => [
  h('th', { scope: 'col', class: 'n ais-c' }, t('ai.set.calls')), h('th', { scope: 'col', class: 'n ais-c' }, t('ai.set.tokensIn')),
  h('th', { scope: 'col', class: 'n ais-c' }, t('ai.set.tokensOut')), h('th', { scope: 'col', class: 'n ais-c' }, t('ai.set.tokensCached')),
  h('th', { scope: 'col', class: 'n ais-c' }, t('ai.set.errors')),
];

/** AS-3, up to 560 px: "Calls 17 · Tokens 2,631 / 367" (cached and errors only when not zero; today's count when given) */
function sumCell(c: Counts, today?: string): HTMLElement {
  return h('td', { class: 'ais-usum' },
    t('ai.set.usage.sum', { calls: num(c.calls), tin: num(c.tokensIn), tout: num(c.tokensOut) }),
    c.tokensCached || c.tokensCacheWrite ? t('ai.set.usage.sumCached', { w: formatBig(c.tokensCacheWrite ?? 0), r: formatBig(c.tokensCached ?? 0) }) : '',
    c.errors ? t('ai.set.usage.sumErr', { n: num(c.errors) }) : '',
    today ? t('ai.set.usage.sumToday', { v: today }) : '');
}

type UsageUser = AiUsage['byUser'][number];

/** v1.1 (L-54): the "by user" table with a search box, sortable columns and 10 users per page */
function usageUsersTable(rows: UsageUser[], perDay: number | undefined): HTMLElement {
  const nameOf = (r: UsageUser): string => (r.userId == null ? t('ai.set.deletedUsers') : r.username ?? `#${r.userId}`);
  const numCmp = (f: (r: UsageUser) => number) => (a: UsageUser, b: UsageUser): number => f(a) - f(b);
  const col = (key: string, label: MsgKey, cls: string, text = false): PtColumn => ({ key, label: t(label), cls, text, short: t(`ai.set.short.${key}` as MsgKey) });
  return pagedTable<UsageUser>({
    id: 'usage-users', rows, name: nameOf, cardsAt: 'mini', tableClass: 'ais-table ais-mini',
    defaultSort: { key: 'calls', dir: 'desc' },
    columns: [
      col('user', 'ai.set.col.user', '', true), col('calls', 'ai.set.calls', 'n ais-c'), col('tin', 'ai.set.tokensIn', 'n ais-c'),
      col('tout', 'ai.set.tokensOut', 'n ais-c'), { ...col('cached', 'ai.set.tokensCached', 'n ais-c'), plain: true },
      col('errors', 'ai.set.errors', 'n ais-c'), col('today', 'ai.set.col.today', 'n ais-c'), col('last', 'ai.set.user.lastUsed', 'ais-whenh'),
    ],
    cmp: {
      user: (a, b) => nameOf(a).localeCompare(nameOf(b), localeTag()),
      calls: numCmp((r) => r.calls), tin: numCmp((r) => r.tokensIn), tout: numCmp((r) => r.tokensOut), errors: numCmp((r) => r.errors),
      today: numCmp((r) => r.today ?? -1), last: numCmp((r) => r.lastAt ?? 0),
    },
    renderRow: (r) => h('tr', null,
      h('th', { scope: 'row', title: cacheText(r) }, nameOf(r)), countsCells(r, true),
      h('td', { class: 'n ais-c ais-today' }, todayCell(r.today, perDay)),
      h('td', { class: 'ais-when' }, when(r.lastAt))),
  });
}

const DASH = (): string => t('common.none');

/** Usage by model: the bars and the hidden table with the connection column (a dash before v1.3) */
function modelParts(u: AiUsage): HTMLElement[] {
  return [
    usageBarCard({ pref: 'model', kind: 'model', exact: num, rows: modelRows(u.byModel, t('ai.set.uc.other')) }),
    h('div', { class: 'ais-tablewrap sr-only' }, h('table', { class: 'ais-table ais-cards' },
      h('thead', null, h('tr', null, h('th', { scope: 'col' }, t('ai.set.col.model')), h('th', { scope: 'col' }, t('ai.usage.col.connection')), countsHead())),
      // The model-listing calls are stored without a model (kind 'models'): labelled like "Deleted users" above
      h('tbody', null, u.byModel.map((r) => h('tr', null,
        r.model ? h('th', { scope: 'row', class: 'ais-mono' }, r.model) : h('th', { scope: 'row' }, t('ai.set.modelListCalls')),
        h('td', null, connLabel(r.connection, DASH())),
        countsCells(r), sumCell(r)))))),
  ];
}

/** The shared hint / explanation store and the site-wide daily budget, as two quiet lines */
function sharedLines(u: AiUsage): Array<HTMLElement | null> {
  const b = u.budget;
  return [
    u.shared ? h('p', { class: 'ais-note ais-shared' }, t('ai.usage.shared', { rows: num(u.shared.rows), hits: num(u.shared.hits) })) : null,
    b && b.limit > 0 ? h('p', { class: 'ais-note ais-budget' }, t('ai.usage.budget', { used: num(b.usedToday), limit: num(b.limit) })) : null,
  ];
}

export function usageSection(): HTMLElement {
  const select = h('select', {
    id: 'ai-days', class: 'ais-select',
    onchange: () => {
      A.usageDays = Number(select.value) || 7;
      A.usage = null;
      redraw();
      void loadUsage();
    },
  }, PERIODS.map((p) => h('option', { value: String(p), selected: p === A.usageDays ? 'selected' : null }, t('ai.set.days', { n: p }))));
  let body: Child;
  const u = A.usage;
  if (!u) {
    body = A.usageError
      ? h('div', { class: 'err', role: 'alert' }, h('p', null, t('ai.set.usageErr', { reason: errorText(A.usageError) })))
      : h('p', { class: 'dim', role: 'status' }, t('common.loading'));
  } else {
    const tiles = h('dl', { class: 'ais-totals' },
      ([['ai.set.calls', u.totals.calls], ['ai.set.tokensIn', u.totals.tokensIn], ['ai.set.tokensOut', u.totals.tokensOut], ['ai.set.tokensCached', u.totals.tokensCached], ['ai.set.errors', u.totals.errors]] as Array<[MsgKey, number]>)
        .map(([k, v]) => h('div', { class: 'ais-total' }, h('dt', null, t(k)), h('dd', { class: 'num' }, num(v)))));
    const cacheNote = h('p', { class: 'ais-note ais-cache' }, cacheText(u.totals));
    // The conversation figures that go with the usage (the owner's "用量跟对话"): the sums over the listed AI+ users
    const conv = A.users && A.users.length ? convTotals(A.users) : null;
    const convs = conv
      ? h('p', { class: 'ais-note ais-convs' }, t('ai.set.usage.convs', { chats: tn('ai.set.usage.nChats', conv.chats, { n: num(conv.chats) }), messages: tn('ai.set.usage.nMessages', conv.messages, { n: num(conv.messages) }), memories: tn('ai.set.usage.nMemories', conv.memories, { n: num(conv.memories) }) }))
      : null;
    const perDay = A.limits?.limits.callsPerDay;
    if (!u.totals.calls) {
      body = [tiles, cacheNote, convs, ...sharedLines(u), h('p', { class: 'ais-note' }, t('ai.set.noUsage'))];
    } else {
      const maxCalls = Math.max(0, ...u.byPeriod.map((p) => p.calls));
      body = [
        tiles,
        cacheNote,
        convs,
        ...sharedLines(u),
        h('h3', { class: 'ais-h3' }, t('ai.set.byUser')),
        usageUsersTable(u.byUser, perDay),
        // v1.1 (UC-1, A-OWNER-39 / 40): vertical bars by model, calls or stacked tokens; the model lists are the "other" bar;
        // the numbers stay in the visually hidden table
        ...modelParts(u),
        // v0.9: what the calls were for (AI Chat, hint, explanation, summary), in the server's order
        u.byPurpose
          ? usageBarCard({
            pref: 'purpose', kind: 'purpose', exact: num,
            rows: purposeRows(PURPOSES, u.byPurpose, u.totals, (k) => t(`ai.set.usage.purpose.${k}` as MsgKey), t('ai.set.uc.other')),
          })
          : null,
        u.byPurpose
          ? h('div', { class: 'ais-tablewrap sr-only' }, h('table', { class: 'ais-table ais-cards' },
            h('thead', null, h('tr', null, h('th', { scope: 'col' }, t('ai.set.usage.byPurpose')), countsHead())),
            h('tbody', null, u.byPurpose.map((r) => h('tr', null,
              h('th', { scope: 'row' }, t(`ai.set.usage.purpose.${r.purpose}` as MsgKey)), countsCells(r), sumCell(r))))))
          : null,
        // A-OWNER-63: the same 调用次数 | Token 数 switch as the bar charts
        usageLineCard({ unit: u.unit, from: u.from, to: u.to, rows: u.byPeriod, format: num }),
        h('div', { class: 'ais-tablewrap sr-only' }, h('table', { class: 'ais-table ais-cards ais-period' },
          h('thead', null, h('tr', null,
            h('th', { scope: 'col' }, t(u.unit === 'month' ? 'ai.set.col.month' : 'ai.set.col.day')),
            h('th', { scope: 'col', class: 'ais-barcol' }, h('span', { class: 'sr-only' }, t('ai.set.calls'))),
            countsHead())),
          h('tbody', null, u.byPeriod.map((r) => h('tr', { class: r.calls ? null : 'zero' },
            h('th', { scope: 'row', class: 'num' }, r.period),
            h('td', { class: 'ais-barcol', 'aria-hidden': 'true' }, h('span', { class: 'ais-bar' }, h('i', { style: `width:${barPercent(r.calls, maxCalls)}%` }))),
            countsCells(r), sumCell(r)))))),
      ];
    }
  }
  return h('section', { class: 'panel ais-sec', 'aria-labelledby': 'ais-usage-h' },
    h('div', { class: 'ais-sechead' },
      h('h2', { id: 'ais-usage-h' }, t('ai.set.sec.usage')),
      h('div', { class: 'ais-period-pick' }, h('label', { class: 'fld-l', for: 'ai-days' }, t('ai.set.period')), select)),
    body);
}
