// App shell: a slim header (logo left; the account label and the menu button right, the navigation opens as a small
// dropdown) + the centered page + footer. The quiz page and the sign-in pages do not use the shell.
import { shutdownApp } from '../api/import.ts';
import { t, type MsgKey } from '../i18n/index.ts';
import { sectionOf, type NavSection, type Route } from '../router.ts';
import { navigate, setNavOpen, signOut } from '../state/actions.ts';
import { S, isAdmin } from '../state/store.ts';
import type { Role } from '../types/api.ts';
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
  user: [{ cx: 12, cy: 8.5, r: 3.8 }, { d: 'M4.8 20c.9-3.6 3.8-5.6 7.2-5.6s6.3 2 7.2 5.6' }],
  users: [{ cx: 9.5, cy: 8.5, r: 3.3 }, { d: 'M3.5 19.5c.7-3.2 3-5 6-5s5.3 1.8 6 5' }, { d: 'M15.3 5.6a3.2 3.2 0 0 1 0 6' }, { d: 'M17.4 14.8c1.7.6 2.8 2.2 3.2 4.7' }],
  signOut: [{ d: 'M14 4.5H6.5v15H14' }, { d: 'M10.5 12h10' }, { d: 'M17 8.5l3.5 3.5-3.5 3.5' }],
} satisfies Record<string, Shape[]>;

const NAV: Array<{ section: NavSection; route: Route; label: MsgKey; icon: Shape[]; admin?: true }> = [
  { section: 'home', route: { name: 'home' }, label: 'nav.home', icon: ICONS.home },
  { section: 'banks', route: { name: 'banks' }, label: 'nav.banks', icon: ICONS.banks, admin: true },
  { section: 'records', route: { name: 'records' }, label: 'nav.records', icon: ICONS.records },
  { section: 'users', route: { name: 'users' }, label: 'nav.users', icon: ICONS.users, admin: true },
  { section: 'settings', route: { name: 'settings' }, label: 'nav.settings', icon: ICONS.settings },
];

const HREF: Record<NavSection, string> = {
  home: '#/', banks: '#/banks', records: '#/records', users: '#/users', settings: '#/settings', about: '#/about',
};

const ROLE_NAME: Record<Role, MsgKey> = { admin: 'role.admin', user: 'role.user' };

/** The name of a role in the interface language */
export const roleName = (role: Role): string => t(ROLE_NAME[role]);

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
    S.info.canShutdown && isAdmin() ? h('button', { class: 'btn quiet', type: 'button', onclick: () => void quitProgram() }, t('footer.shutdown')) : null);
}

/** Set when the menu was opened from the keyboard, so focus moves into it (a mouse click leaves focus on the button). */
let focusMenuOnOpen = false;
/** The button that opened the menu (Esc gives it the focus back) */
let opener: 'menu-btn' | 'acct-btn' = 'menu-btn';

/**
 * Wraps a page in the shell: a slim header with the menu button and the brand, the page (centered), the footer.
 * The navigation is a small dropdown that opens from the menu button and closes again after a choice, a click outside or Esc,
 * so the page itself keeps the whole width.
 */
export function shell(content: HTMLElement): HTMLElement {
  const active = sectionOf(S.route);
  const open = S.navOpen;
  const user = S.user;
  const admin = isAdmin();

  // Who is signed in, and Sign out: the bottom of the menu, under a hairline
  const account = user ? h('div', { class: 'nav-account' },
    h('div', { class: 'nav-who' },
      icon(ICONS.user),
      h('span', { class: 'nav-who-name', title: user.username }, user.username),
      h('span', { class: 'tag ' + (admin ? 'b' : 'k') }, roleName(user.role))),
    h('button', { class: 'nav-link nav-signout', type: 'button', onclick: () => void signOut() },
      icon(ICONS.signOut), h('span', { class: 'nav-label' }, t('nav.signOut')))) : null;

  const menu = open ? h('nav', { class: 'nav-pop', id: 'nav-menu', 'aria-label': t('nav.label') },
    h('ul', { class: 'nav-list' }, NAV.filter((n) => admin || !n.admin).map((n) => h('li', null,
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
    }, icon(ICONS.about), h('span', { class: 'nav-label num' }, S.info.version ? t('footer.version', { version: S.info.version }) : t('about.title'))),
    account) : null;

  const menuBtn = h('button', {
    class: 'icon-btn menu-btn' + (open ? ' on' : ''), id: 'menu-btn', type: 'button',
    'aria-label': t(open ? 'nav.close' : 'nav.open'), title: t(open ? 'nav.close' : 'nav.open'),
    'aria-expanded': String(open), 'aria-controls': 'nav-menu',
    onclick: (e: MouseEvent) => {
      focusMenuOnOpen = !open && e.detail === 0; // detail 0 = activated with Enter / Space, not a pointer
      opener = 'menu-btn';
      setNavOpen(!open);
    },
  }, icon(open ? ICONS.close : ICONS.menu));

  // The account label left of the menu button: who is signed in (and "Administrator"); a click opens the same menu
  const accountBtn = user ? h('button', {
    class: 'acct-btn' + (open ? ' on' : ''), id: 'acct-btn', type: 'button', title: user.username,
    'aria-label': t('nav.accountLabel', { name: user.username, role: roleName(user.role) }),
    'aria-haspopup': 'true', 'aria-expanded': String(open), 'aria-controls': 'nav-menu',
    onclick: (e: MouseEvent) => {
      focusMenuOnOpen = !open && e.detail === 0;
      opener = 'acct-btn';
      setNavOpen(!open);
    },
  },
  icon(ICONS.user, 'ico acct-ico'),
  h('span', { class: 'acct-name' }, user.username),
  // Phones show the short form ("Admin"), so the username keeps its room; the full role is in the label and in the menu
  admin ? h('span', { class: 'tag b acct-role' },
    h('span', { class: 'acct-role-l' }, roleName('admin')),
    h('span', { class: 'acct-role-s' }, t('role.admin.short'))) : null) : null;

  // The logo on the left, the account label and the menu button on the right (the dropdown opens towards the left,
  // aligned to the right edge)
  const header = h('header', { class: 'topbar' },
    h('a', { class: 'top-brand', href: '#/', onclick: (e: MouseEvent) => { e.preventDefault(); navigate({ name: 'home' }); } }, brand()),
    h('div', { class: 'menu-wrap' }, accountBtn, menuBtn, menu));

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
      // Back to the button that opened the menu
      queueMicrotask(() => document.getElementById(opener)?.focus());
    }
  });
  document.addEventListener('pointerdown', (e) => {
    if (!S.navOpen) return;
    const wrap = document.querySelector('.menu-wrap');
    if (wrap && e.target instanceof Node && !wrap.contains(e.target)) setNavOpen(false);
  });
}
