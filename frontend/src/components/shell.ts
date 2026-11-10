// App shell: a slim header (logo left, menu button right; the navigation opens as a small dropdown) + the centered page + footer.
// The quiz page does not use the shell (focused full-screen view).
import { shutdownApp } from '../api/import.ts';
import { t, type MsgKey } from '../i18n/index.ts';
import { sectionOf, type NavSection, type Route } from '../router.ts';
import { navigate, setNavOpen } from '../state/actions.ts';
import { S } from '../state/store.ts';
import { ask } from '../utils/dialog.ts';
import { h } from '../utils/dom.ts';
import { brand } from './common.ts';

const SVGNS = 'http://www.w3.org/2000/svg';

type Shape = { d: string } | { cx: number; cy: number; r: number };

/** A 24x24 line icon (stroke = currentColor) */
export function icon(shapes: Shape[], cls = 'ico'): SVGSVGElement {
  const s = document.createElementNS(SVGNS, 'svg');
  s.setAttribute('viewBox', '0 0 24 24');
  s.setAttribute('fill', 'none');
  s.setAttribute('stroke', 'currentColor');
  s.setAttribute('stroke-width', '1.8');
  s.setAttribute('stroke-linecap', 'round');
  s.setAttribute('stroke-linejoin', 'round');
  s.setAttribute('aria-hidden', 'true');
  s.setAttribute('class', cls);
  for (const sh of shapes) {
    if ('d' in sh) {
      const p = document.createElementNS(SVGNS, 'path');
      p.setAttribute('d', sh.d);
      s.append(p);
    } else {
      const c = document.createElementNS(SVGNS, 'circle');
      c.setAttribute('cx', String(sh.cx));
      c.setAttribute('cy', String(sh.cy));
      c.setAttribute('r', String(sh.r));
      s.append(c);
    }
  }
  return s;
}

export const ICONS = {
  home: [{ d: 'M3.5 11 12 4l8.5 7' }, { d: 'M6 9.5V20h12V9.5' }, { d: 'M10 20v-5.5h4V20' }],
  banks: [{ d: 'M4 6.5C4 5.1 7.6 4 12 4s8 1.1 8 2.5S16.4 9 12 9 4 7.9 4 6.5Z' }, { d: 'M4 6.5v5C4 12.9 7.6 14 12 14s8-1.1 8-2.5v-5' }, { d: 'M4 11.5v5C4 17.9 7.6 19 12 19s8-1.1 8-2.5v-5' }],
  records: [{ d: 'M3.5 20h17' }, { d: 'M6.5 20v-7' }, { d: 'M12 20V5' }, { d: 'M17.5 20v-10' }],
  settings: [{ d: 'M4 7h9' }, { d: 'M17 7h3' }, { cx: 15, cy: 7, r: 2 }, { d: 'M4 17h3' }, { d: 'M11 17h9' }, { cx: 9, cy: 17, r: 2 }],
  menu: [{ d: 'M4 7h16' }, { d: 'M4 12h16' }, { d: 'M4 17h16' }],
  about: [{ cx: 12, cy: 12, r: 8.5 }, { d: 'M12 11v5.5' }, { d: 'M12 7.6v.4' }],
  close: [{ d: 'M6 6l12 12' }, { d: 'M18 6 6 18' }],
} satisfies Record<string, Shape[]>;

const NAV: Array<{ section: NavSection; route: Route; label: MsgKey; icon: Shape[] }> = [
  { section: 'home', route: { name: 'home' }, label: 'nav.home', icon: ICONS.home },
  { section: 'banks', route: { name: 'banks' }, label: 'nav.banks', icon: ICONS.banks },
  { section: 'records', route: { name: 'records' }, label: 'nav.records', icon: ICONS.records },
  { section: 'settings', route: { name: 'settings' }, label: 'nav.settings', icon: ICONS.settings },
];

const HREF: Record<NavSection, string> = { home: '#/', banks: '#/banks', records: '#/records', settings: '#/settings', about: '#/about' };

async function quitProgram(): Promise<void> {
  if (!(await ask({ title: t('shutdown.title'), body: t('shutdown.body'), ok: t('shutdown.ok'), cancel: t('shutdown.cancel') }))) return;
  try {
    await shutdownApp();
  } catch {
    /* the connection may drop when the server shuts down quickly */
  }
  document.getElementById('app')?.replaceChildren(h('div', { class: 'solo' }, h('div', { class: 'panel' },
    h('h2', null, t('shutdown.done.title')), h('p', null, t('shutdown.done.body')))));
}

export function footer(): HTMLElement {
  return h('footer', { class: 'foot' },
    S.info.version ? h('span', { class: 'num' }, t('footer.version', { version: S.info.version })) : null,
    h('span', null, t('footer.disclaimer')),
    S.info.canShutdown ? h('button', { class: 'btn quiet', type: 'button', onclick: () => void quitProgram() }, t('footer.shutdown')) : null);
}

/** Set when the menu was opened from the keyboard, so focus moves into it (a mouse click leaves focus on the button). */
let focusMenuOnOpen = false;

/**
 * Wraps a page in the shell: a slim header with the menu button and the brand, the page (centered), the footer.
 * The navigation is a small dropdown that opens from the menu button and closes again after a choice, a click outside or Esc,
 * so the page itself keeps the whole width.
 */
export function shell(content: HTMLElement): HTMLElement {
  const active = sectionOf(S.route);
  const open = S.navOpen;

  const menu = open ? h('nav', { class: 'nav-pop', id: 'nav-menu', 'aria-label': t('nav.label') },
    h('ul', { class: 'nav-list' }, NAV.map((n) => h('li', null,
      h('a', {
        class: 'nav-link' + (n.section === active ? ' on' : ''),
        href: HREF[n.section],
        'aria-current': n.section === active ? 'page' : null,
        onclick: (e: MouseEvent) => {
          if (e.ctrlKey || e.metaKey || e.shiftKey || e.button !== 0) return;
          e.preventDefault();
          navigate(n.route);
        },
      }, icon(n.icon), h('span', { class: 'nav-label' }, t(n.label)))))),
    // The version opens the About page (version, what is new); written as "Xexam vX.Y" like everywhere else
    h('a', {
      class: 'nav-link nav-about' + (active === 'about' ? ' on' : ''), href: HREF.about,
      'aria-current': active === 'about' ? 'page' : null, title: t('about.title'),
      onclick: (e: MouseEvent) => {
        if (e.ctrlKey || e.metaKey || e.shiftKey || e.button !== 0) return;
        e.preventDefault();
        navigate({ name: 'about' });
      },
    }, icon(ICONS.about), h('span', { class: 'nav-label num' }, S.info.version ? t('footer.version', { version: S.info.version }) : t('about.title')))) : null;

  const menuBtn = h('button', {
    class: 'icon-btn menu-btn' + (open ? ' on' : ''), id: 'menu-btn', type: 'button',
    'aria-label': t(open ? 'nav.close' : 'nav.open'), title: t(open ? 'nav.close' : 'nav.open'),
    'aria-expanded': String(open), 'aria-controls': 'nav-menu',
    onclick: (e: MouseEvent) => {
      focusMenuOnOpen = !open && e.detail === 0; // detail 0 = activated with Enter / Space, not a pointer
      setNavOpen(!open);
    },
  }, icon(open ? ICONS.close : ICONS.menu));

  // The logo on the left, the menu button on the right (its dropdown opens towards the left, aligned to the right edge)
  const header = h('header', { class: 'topbar' },
    h('a', { class: 'top-brand', href: '#/', onclick: (e: MouseEvent) => { e.preventDefault(); navigate({ name: 'home' }); } }, brand()),
    h('div', { class: 'menu-wrap' }, menuBtn, menu));

  if (open && focusMenuOnOpen) {
    focusMenuOnOpen = false;
    queueMicrotask(() => document.querySelector<HTMLElement>('#nav-menu .nav-link.on, #nav-menu .nav-link')?.focus());
  }
  return h('div', { class: 'shell' }, h('main', { class: 'main', id: 'main' }, header, content, footer()));
}

/** Installs the closing behaviour of the menu (once): Esc, a click outside it. */
export function installShellHandlers(): void {
  document.addEventListener('keydown', (e) => {
    if (e.key === 'Escape' && S.navOpen && !document.querySelector('dialog[open]')) {
      setNavOpen(false);
      queueMicrotask(() => document.getElementById('menu-btn')?.focus());
    }
  });
  document.addEventListener('pointerdown', (e) => {
    if (!S.navOpen) return;
    const wrap = document.querySelector('.menu-wrap');
    if (wrap && e.target instanceof Node && !wrap.contains(e.target)) setNavOpen(false);
  });
}
