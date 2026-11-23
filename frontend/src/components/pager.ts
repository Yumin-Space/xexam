// The numbered pager shared by the AI+ tables, User management and Study records (v2.7). Always drawn ("Page 1 / 1" too);
// the arrows stay in the DOM and only turn aria-disabled; page numbers come from domain pageList (phones: 1 ... cur ... last).
//
// Usage (the owner keeps `page` and repaints after onGo; ids are `${id}-prev`, `${id}-next`, `${id}-p${n}`):
//   const host = h('div', { class: 'us-pagerhost' });
//   const paint = () => host.replaceChildren(pager({ id: 'us', page, pages, onGo: (p) => { page = p; paint(); }, scrollTarget: list }));
//   // scrollTarget (optional): after a page change the element scrolls to the top when it starts above the screen.
// Focus after a click stays on the pressed control (or on the current page button when that arrow became disabled); it is
// applied right after onGo returns, so onGo must repaint synchronously.
import { localeTag, t } from '../i18n/index.ts';
import { canGoTo, pageList, pagerFocusId, startsAboveScreen } from '../domain/paged-table.ts';
import { h } from '../utils/dom.ts';
import { matches, PHONE_MQ } from '../utils/media.ts';
import '../styles/paged-table.css';

export interface PagerOptions {
  /** Element-id prefix */
  id: string;
  /** Current page, 1-based */
  page: number;
  pages: number;
  onGo: (page: number) => void;
  /** Accessible name of the nav (default: the shared "Pager" text) */
  label?: string;
  /** Scrolled to the top after a page change when it starts above the screen */
  scrollTarget?: HTMLElement | null;
  /** Focus the pressed control without scrolling it into view (the records / users lists); default: the browser's own scroll */
  preventScroll?: boolean;
}

const num = (n: number): string => n.toLocaleString(localeTag());

export function pager(o: PagerOptions): HTMLElement {
  const go = (target: number, key: string, byKeyboard: boolean): void => {
    if (!canGoTo(target, o.page, o.pages)) return;
    o.onGo(target);
    // The owner has repainted by now: settle focus and scroll on the new pager
    const el = document.getElementById(`${o.id}-${key}`);
    const fid = pagerFocusId(o.id, key, !el || el.getAttribute('aria-disabled') === 'true', target);
    const fe = document.getElementById(fid);
    // A tap or click leaves no ring (data-ptr hides :focus-visible); the keyboard keeps it
    if (fe && !byKeyboard) {
      fe.setAttribute('data-ptr', '');
      fe.addEventListener('blur', () => fe.removeAttribute('data-ptr'), { once: true });
    }
    fe?.focus({ preventScroll: o.preventScroll === true });
    const st = o.scrollTarget;
    if (st && st.isConnected && startsAboveScreen(st.getBoundingClientRect().top)) st.scrollIntoView({ block: 'start' });
  };
  const step = (dir: -1 | 1): HTMLElement => {
    const target = o.page + dir;
    const off = target < 1 || target > o.pages;
    return h('button', {
      type: 'button', id: `${o.id}-${dir < 0 ? 'prev' : 'next'}`, class: 'ptb-pg ptb-step', 'aria-disabled': off ? 'true' : null,
      'aria-label': t(dir < 0 ? 'ai.set.tbl.prev' : 'ai.set.tbl.next'),
      onclick: (ev: MouseEvent) => go(target, dir < 0 ? 'prev' : 'next', ev.detail === 0),
    }, dir < 0 ? '‹' : '›');
  };
  const nums = pageList(o.page, o.pages, matches(PHONE_MQ) ? 0 : 1).map((p) => (p === 'gap'
    ? h('span', { class: 'ptb-gap', 'aria-hidden': 'true' }, '…')
    : h('button', {
      type: 'button', id: `${o.id}-p${p}`, class: 'ptb-pg' + (p === o.page ? ' on' : ''), 'aria-current': p === o.page ? 'page' : null,
      'aria-label': t('ai.set.tbl.goPage', { n: num(p) }), onclick: (ev: MouseEvent) => go(p, `p${p}`, ev.detail === 0),
    }, num(p))));
  return h('nav', { class: 'ptb-pager', 'aria-label': o.label ?? t('ai.set.tbl.pager') },
    h('div', { class: 'ptb-btns' }, step(-1), ...nums, step(1)),
    h('span', { class: 'ptb-of' }, t('ai.set.tbl.pageOf', { p: num(o.page), n: num(o.pages) })));
}
