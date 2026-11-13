// A table with a search box, sortable columns and 10 rows per page (v1.1 L-54, A-OWNER-54 / 54b): the AI usage "by user"
// table and the AI+ users list. The state (search text, sort, page) lives in a module map under the table's id, so a redraw
// of the whole page keeps it; typing, sorting and paging redraw only the rows and the pager, so the search box and the
// focus stay where they are. The logic is domain/paged-table.ts.
//   · wide screens: the column headers are buttons (click to sort, again to reverse, an arrow shows the order);
//   · `cardsAt: 'mini'` (A-OWNER-59): never cards; on phones the same table with smaller type, the header row kept for sorting
//     (the sort select stays hidden) and its own sideways scroll with the first column fixed (styles/paged-table.css);
//   · screens where the CSS turns the rows into cards (`cardsAt`): a "Sort by" select and an order button take over
//     (the header row is hidden and not focusable there);
//   · the pager keeps its place: the arrows stay in the DOM (aria-disabled, never removed) and "Page x / y" is always shown.
import { localeTag, t } from '../i18n/index.ts';
import { clickSort, pageList, pageTable, withQuery, type PagedResult, type SortDir, type TableState } from '../domain/paged-table.ts';
import { h, type Child } from '../utils/dom.ts';
import { matches, PHONE_MQ } from '../utils/media.ts';
import '../styles/paged-table.css';

export interface PtColumn {
  key: string;
  label: string;
  /** Header cell classes */
  cls?: string;
  /** A text column (sorts A to Z first); the others sort largest first */
  text?: boolean;
  /** Not sortable (default: sortable) */
  plain?: boolean;
  /** A header only for screen readers (no visible text) */
  srOnly?: boolean;
}

export interface PagedTableOptions<T> {
  /** State key and element-id prefix, e.g. "usage-users" */
  id: string;
  rows: readonly T[];
  columns: PtColumn[];
  /** The text the search looks into */
  name: (r: T) => string;
  /** Ascending comparators by column key; the "user" key is also the tie-breaker */
  cmp: Record<string, (a: T, b: T) => number>;
  defaultSort: { key: string; dir: SortDir };
  /** The row element(s) of one item */
  renderRow: (r: T) => Child;
  tableClass: string;
  /** Explicit table roles (a table that becomes cards through display: grid) */
  roles?: boolean;
  /** The width (px) up to which the CSS shows cards (the sort select is shown there), or 'mini' for the phone mini table */
  cardsAt: 560 | 900 | 'mini';
}

const states = new Map<string, TableState>();

const num = (n: number): string => n.toLocaleString(localeTag());

function nodesOf(c: Child, out: Node[] = []): Node[] {
  if (Array.isArray(c)) c.forEach((k) => nodesOf(k, out));
  else if (c instanceof Node) out.push(c);
  return out;
}

export function pagedTable<T>(o: PagedTableOptions<T>): HTMLElement {
  const stateOf = (): TableState => {
    let s = states.get(o.id);
    if (!s) {
      s = { query: '', key: o.defaultSort.key, dir: o.defaultSort.dir, page: 1 };
      states.set(o.id, s);
    }
    return s;
  };
  const sortable = o.columns.filter((c) => !c.plain);
  const isText = (key: string): boolean => !!o.columns.find((c) => c.key === key)?.text;
  const role = (r: string): Record<string, string> | null => (o.roles ? { role: r } : null);
  const sortLabel = (key: string): string => o.columns.find((c) => c.key === key)?.label ?? '';
  const dirText = (d: SortDir): string => t(d === 'asc' ? 'ai.set.tbl.asc' : 'ai.set.tbl.desc');

  const set = (next: TableState): void => {
    states.set(o.id, next);
    paint();
  };

  // Toolbar: the search box is always there; the sort select and the order button show in the card layouts only
  const search = h('input', {
    type: 'search', id: `${o.id}-q`, class: 'ptb-q', value: stateOf().query, maxlength: '100', autocomplete: 'off', spellcheck: 'false',
    enterkeyhint: 'search', placeholder: t('ai.set.tbl.search'), 'aria-label': t('ai.set.tbl.search'),
    oninput: () => set(withQuery(stateOf(), search.value)),
  });
  const sel = h('select', {
    id: `${o.id}-sort`, class: 'ptb-sel', 'aria-label': t('ai.set.tbl.sortBy'),
    onchange: () => set(clickSort({ ...stateOf(), key: '' }, sel.value, isText(sel.value))),
  }, sortable.map((c) => h('option', { value: c.key }, c.label)));
  const dirBtn = h('button', {
    type: 'button', id: `${o.id}-dir`, class: 'ptb-dir',
    onclick: () => {
      const s = stateOf();
      set({ ...s, dir: s.dir === 'asc' ? 'desc' : 'asc', page: 1 });
    },
  });
  const tools = h('div', { class: 'ptb-tools' }, search, h('div', { class: 'ptb-sortsel' }, sel, dirBtn));

  // Table: the header buttons are built once and only re-marked
  const arrows = new Map<string, HTMLElement>();
  const ths = new Map<string, HTMLElement>();
  const headCells = o.columns.map((c) => {
    if (c.plain) {
      return h('th', { scope: 'col', ...role('columnheader'), class: c.cls ?? null }, c.srOnly ? h('span', { class: 'sr-only' }, c.label) : c.label);
    }
    const arrow = h('span', { class: 'ptb-arrow', 'aria-hidden': 'true' });
    arrows.set(c.key, arrow);
    const btn = h('button', { type: 'button', class: 'ptb-sort', onclick: () => set(clickSort(stateOf(), c.key, !!c.text)) },
      h('span', { class: 'ptb-lbl' }, c.label), arrow);
    const th = h('th', { scope: 'col', ...role('columnheader'), class: ((c.cls ?? '') + ' ptb-th').trim(), 'aria-sort': 'none' }, btn);
    ths.set(c.key, th);
    return th;
  });
  const tbody = h('tbody', role('rowgroup'));
  const table = h('table', { class: `${o.tableClass} ptb-table`, ...role('table') },
    h('thead', role('rowgroup'), h('tr', role('row'), headCells)), tbody);
  const wrap = h('div', { class: 'ais-tablewrap' }, table);
  const none = h('p', { class: 'ais-note ptb-none', role: 'status', hidden: '' }, t('ai.set.tbl.none'));

  const status = h('p', { class: 'sr-only', role: 'status', 'aria-live': 'polite' });
  const pagerNav = h('nav', { class: 'ptb-pager', 'aria-label': t('ai.set.tbl.pager') });
  let focusAfter: string | null = null;

  function paint(): void {
    const s = stateOf();
    const r: PagedResult<T> = pageTable(o.rows, s, { name: o.name, cmp: o.cmp });
    if (r.page !== s.page) states.set(o.id, { ...s, page: r.page });
    const cur = stateOf();
    for (const [key, th] of ths) {
      const on = key === cur.key;
      th.setAttribute('aria-sort', on ? (cur.dir === 'asc' ? 'ascending' : 'descending') : 'none');
      th.classList.toggle('on', on);
      const a = arrows.get(key);
      if (a) a.textContent = on ? (cur.dir === 'asc' ? '▲' : '▼') : '↕';
    }
    sel.value = cur.key;
    dirBtn.textContent = cur.dir === 'asc' ? '▲' : '▼';
    dirBtn.setAttribute('aria-label', t('ai.set.tbl.dirBtn', { sort: sortLabel(cur.key), dir: dirText(cur.dir) }));
    tbody.replaceChildren(...r.items.flatMap((it) => nodesOf(o.renderRow(it))));
    wrap.hidden = r.total === 0;
    none.hidden = r.total !== 0;
    status.textContent = r.total ? t('ai.set.tbl.range', { from: num(r.from), to: num(r.to), total: num(r.total) }) : t('ai.set.tbl.none');

    const go = (p: number, key: string): void => {
      if (p < 1 || p > r.pages || p === r.page) return;
      focusAfter = key;
      set({ ...stateOf(), page: p });
    };
    const step = (dir: -1 | 1): HTMLElement => {
      const target = r.page + dir;
      const off = target < 1 || target > r.pages;
      return h('button', {
        type: 'button', id: `${o.id}-${dir < 0 ? 'prev' : 'next'}`, class: 'ptb-pg ptb-step', 'aria-disabled': off ? 'true' : null,
        'aria-label': t(dir < 0 ? 'ai.set.tbl.prev' : 'ai.set.tbl.next'),
        onclick: () => go(target, dir < 0 ? 'prev' : 'next'),
      }, dir < 0 ? '‹' : '›');
    };
    const pages = pageList(r.page, r.pages, matches(PHONE_MQ) ? 0 : 1).map((p) => (p === 'gap'
      ? h('span', { class: 'ptb-gap', 'aria-hidden': 'true' }, '…')
      : h('button', {
        type: 'button', id: `${o.id}-p${p}`, class: 'ptb-pg' + (p === r.page ? ' on' : ''), 'aria-current': p === r.page ? 'page' : null,
        'aria-label': t('ai.set.tbl.goPage', { n: num(p) }), onclick: () => go(p, `p${p}`),
      }, num(p))));
    pagerNav.replaceChildren(
      h('div', { class: 'ptb-btns' }, step(-1), ...pages, step(1)),
      h('span', { class: 'ptb-of' }, t('ai.set.tbl.pageOf', { p: num(r.page), n: num(r.pages) })));
    if (focusAfter) {
      // The arrow keeps the focus unless it just became unusable; then the current page number takes it
      let id = `${o.id}-${focusAfter}`;
      const el = document.getElementById(id);
      if (!el || el.getAttribute('aria-disabled') === 'true') id = `${o.id}-p${r.page}`;
      focusAfter = null;
      document.getElementById(id)?.focus();
    }
  }

  const root = h('div', { class: `ptb ptb-${o.cardsAt}` }, tools, wrap, none, pagerNav, status);
  paint();
  return root;
}
