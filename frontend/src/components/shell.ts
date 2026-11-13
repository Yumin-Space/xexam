// App shell: a slim header (logo left; the account label and the menu button right, the navigation opens as a small
// dropdown) + the centered page + footer. The quiz page and the sign-in pages do not use the shell.
import '../styles/shell.css';
import { shutdownApp } from '../api/import.ts';
import { ownBadgeState } from '../domain/badge-state.ts';
import { barStep, initialBar } from '../domain/top-bar.ts';
import { t, type MsgKey } from '../i18n/index.ts';
import { routeHash, sectionOf, type NavSection, type Route } from '../router.ts';
import { navigate, setNavOpen, signOut } from '../state/actions.ts';
import { aiStatus } from '../state/ai.ts';
import { navAllowed, type NavNeed } from '../state/bank-access.ts';
import { S, isAdmin } from '../state/store.ts';
import type { AuthUser, Role } from '../types/api.ts';
import { ask } from '../utils/dialog.ts';
import { h, isAppScroll } from '../utils/dom.ts';
import { matches, PHONE_MQ } from '../utils/media.ts';
import { offList, readFeatures } from '../domain/ai-features.ts';
import { aiBadge, badgeReasonText, badgeTipText } from './ai-badge.ts';
import { brand } from './common.ts';
import { holdTip } from './tooltip.ts';
import { githubLine } from './github-line.ts';

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
  /** v1.0 (MN-3), Settings: a hub ring with eight teeth, so it no longer looks like AI settings' sliders */
  gear: [
    { cx: 12, cy: 12, r: 3 }, { cx: 12, cy: 12, r: 6.2 },
    { d: 'M18.2 12h2.4' }, { d: 'M16.38 7.62l1.7-1.7' }, { d: 'M12 5.8V3.4' }, { d: 'M7.62 7.62l-1.7-1.7' },
    { d: 'M5.8 12H3.4' }, { d: 'M7.62 16.38l-1.7 1.7' }, { d: 'M12 18.2v2.4' }, { d: 'M16.38 16.38l1.7 1.7' },
  ],
  menu: [{ d: 'M4 7h16' }, { d: 'M4 12h16' }, { d: 'M4 17h16' }],
  about: [{ cx: 12, cy: 12, r: 8.5 }, { d: 'M12 11v5.5' }, { d: 'M12 7.6v.4' }],
  close: [{ d: 'M6 6l12 12' }, { d: 'M18 6 6 18' }],
  user: [{ cx: 12, cy: 8.5, r: 3.8 }, { d: 'M4.8 20c.9-3.6 3.8-5.6 7.2-5.6s6.3 2 7.2 5.6' }],
  users: [{ cx: 9.5, cy: 8.5, r: 3.3 }, { d: 'M3.5 19.5c.7-3.2 3-5 6-5s5.3 1.8 6 5' }, { d: 'M15.3 5.6a3.2 3.2 0 0 1 0 6' }, { d: 'M17.4 14.8c1.7.6 2.8 2.2 3.2 4.7' }],
  signOut: [{ d: 'M14 4.5H6.5v15H14' }, { d: 'M10.5 12h10' }, { d: 'M17 8.5l3.5 3.5-3.5 3.5' }],
  logs: [{ d: 'M5.5 4.5h13v15h-13z' }, { d: 'M8.5 8.5h7' }, { d: 'M8.5 12h7' }, { d: 'M8.5 15.5h4' }],
  /** v0.8, AI Chat: a four-point star and a small one */
  sparkle: [
    { d: 'M10 3.5c.7 4.2 2.6 6.1 6.8 6.8-4.2.7-6.1 2.6-6.8 6.8-.7-4.2-2.6-6.1-6.8-6.8 4.2-.7 6.1-2.6 6.8-6.8Z' },
    { d: 'M18 14.5c.3 1.9 1.1 2.7 3 3-1.9.3-2.7 1.1-3 3-.3-1.9-1.1-2.7-3-3 1.9-.3 2.7-1.1 3-3Z' },
  ],
  /** v1.0 (MN-3), AI settings: two sliders and a small spark above them */
  aiSliders: [
    { d: 'M4 11h3' }, { d: 'M11 11h2' }, { cx: 9, cy: 11, r: 2 },
    { d: 'M4 18h9' }, { d: 'M17 18h3' }, { cx: 15, cy: 18, r: 2 },
    { d: 'M18.5 3.5c.3 1.9 1.1 2.7 3 3-1.9.3-2.7 1.1-3 3-.3-1.9-1.1-2.7-3-3 1.9-.3 2.7-1.1 3-3Z' },
  ],
} satisfies Record<string, Shape[]>;

// `need`: who sees the entry (v0.6.5): Question banks needs the bank-manager permission (administrators always have it), User
// management and the System log (v0.7) an administrator; the rest is for everyone. v0.8: AI Chat when the user can use AI
// now or has AI chats or memories to read (hidden, not "unavailable", otherwise); AI settings for administrators, always.
const NAV: Array<{ section: NavSection; route: Route; label: MsgKey; icon: Shape[]; need?: NavNeed }> = [
  { section: 'home', route: { name: 'home' }, label: 'nav.home', icon: ICONS.home },
  { section: 'banks', route: { name: 'banks' }, label: 'nav.banks', icon: ICONS.banks, need: 'banks' },
  { section: 'records', route: { name: 'records' }, label: 'nav.records', icon: ICONS.records },
  { section: 'ai', route: { name: 'ai' }, label: 'nav.aiChat', icon: ICONS.sparkle, need: 'ai' },
  { section: 'users', route: { name: 'users' }, label: 'nav.users', icon: ICONS.users, need: 'admin' },
  { section: 'aiSettings', route: { name: 'aiSettings' }, label: 'nav.aiSettings', icon: ICONS.aiSliders, need: 'admin' },
  { section: 'logs', route: { name: 'logs' }, label: 'nav.logs', icon: ICONS.logs, need: 'admin' },
  { section: 'settings', route: { name: 'settings' }, label: 'nav.settings', icon: ICONS.gear },
];

const HREF: Record<NavSection, string> = {
  home: '#/', banks: '#/banks', records: '#/records', ai: '#/ai', users: '#/users', aiSettings: '#/ai-settings', logs: '#/logs',
  settings: '#/settings', about: '#/about',
};

const ROLE_NAME: Record<Role, MsgKey> = { admin: 'role.admin', user: 'role.user' };

/** The name of a role in the interface language */
export const roleName = (role: Role): string => t(ROLE_NAME[role]);

/** A plain user with the "can manage question banks" permission (v0.7: shown as "Bank manager" next to the role) */
export const isBankManager = (u: Pick<AuthUser, 'role' | 'canManageBanks'>): boolean => u.role === 'user' && u.canManageBanks === true;

/**
 * The small tags of an account (the menu's account area, Settings > Account): the role, plus "Bank manager" for a plain user
 * who may manage question banks (administrators always may, so they get no extra tag).
 */
export function roleTags(u: Pick<AuthUser, 'role' | 'canManageBanks'>): HTMLElement[] {
  return [
    h('span', { class: 'tag ' + (u.role === 'admin' ? 'b' : 'k') }, roleName(u.role)),
    ...(isBankManager(u) ? [h('span', { class: 'tag b' }, t('role.manager'))] : []),
  ];
}

/** The role in words, for an accessible name: "User, Bank manager" */
export const roleText = (u: Pick<AuthUser, 'role' | 'canManageBanks'>): string =>
  [roleName(u.role), isBankManager(u) ? t('role.manager') : null].filter(Boolean).join(t('common.listSep'));

async function quitProgram(): Promise<void> {
  if (!(await ask({ title: t('shutdown.title'), body: t('shutdown.body'), ok: t('shutdown.ok'), cancel: t('shutdown.cancel') }))) return;
  try {
    await shutdownApp();
  } catch {
    /* the connection may drop when the server shuts down quickly */
  }
  document.getElementById('app')?.replaceChildren(h('div', { class: 'solo' }, h('div', { class: 'panel' },
    h('h2', null, t('shutdown.done.title')), h('p', null, t('shutdown.done.body'))), githubLine()));
}

export function footer(): HTMLElement {
  return h('footer', { class: 'foot' },
    h('p', { class: 'foot-note' }, t('footer.disclaimer')),
    S.info.canShutdown && isAdmin() ? h('button', { class: 'btn quiet', type: 'button', onclick: () => void quitProgram() }, t('footer.shutdown')) : null,
    githubLine());
}

// ---------------------------------------------------------------- the slim top bar on phones (SH-7)

/** Whether the top bar is the pinned one: PHONE, and not AI Chat (its page owns the whole height, CH-1) */
const barPinned = (): boolean => matches(PHONE_MQ) && S.route.name !== 'ai';

let bar = initialBar();
/** The route the bar state belongs to: a new page always starts with the bar shown */
let barRoute = '';

function barEl(): HTMLElement | null {
  return document.querySelector<HTMLElement>('.topbar');
}

/** `--top-h` (spec 2.12 e): the height of what is pinned at the top, 0 while the bar is hidden or not pinned */
function setTopH(px: number): void {
  document.documentElement.style.setProperty('--top-h', `${px}px`);
}

/** Clears `--top-h` on pages without the shell (sign-in, loading); the quiz page sets its own */
export function resetTopH(): void {
  setTopH(0);
}

function paintBar(): void {
  const el = barEl();
  if (!el) return;
  const pinned = barPinned();
  const hidden = pinned && bar.hidden && !S.navOpen;
  el.classList.toggle('topbar-hide', hidden);
  el.classList.toggle('topbar-scrolled', pinned && window.scrollY > 0);
  setTopH(pinned && !hidden ? el.offsetHeight : 0);
}

function showBar(): void {
  if (!bar.hidden) return;
  bar = { ...bar, hidden: false, dir: 0 };
  paintBar();
}

/** A tap that closed the menu must not also press the control under it: eat the click that follows (for about 400 ms) */
function swallowNextClick(): void {
  const eat = (ev: Event): void => {
    ev.stopPropagation();
    ev.preventDefault();
    off();
  };
  const off = (): void => {
    window.clearTimeout(t);
    document.removeEventListener('click', eat, true);
  };
  const t = window.setTimeout(off, 400);
  document.addEventListener('click', eat, true);
}

/** The menu may not be taller than the room under its button (MN-1); it scrolls inside when the list is longer */
function fitMenu(): void {
  const nav = document.getElementById('nav-menu');
  if (!nav) return;
  const top = Math.max(0, Math.round(nav.getBoundingClientRect().top));
  nav.style.maxHeight = `calc(100dvh - ${top}px - 8px - env(safe-area-inset-bottom))`;
}

/** Called by main.ts right after every shell page was put into the document (the bar needs its measured height) */
export function afterShellRender(): void {
  paintBar();
  fitMenu();
}

function onPageScroll(): void {
  if (!barPinned() || !barEl()) return;
  const y = window.scrollY;
  const root = document.documentElement;
  // A scroll the app made (the panel's, a jump) never moves the bar; the next user scroll starts from here
  if (isAppScroll()) {
    bar = { ...bar, lastY: Math.max(0, y), dir: 0 };
    paintBar();
    return;
  }
  const next = barStep(bar, y, root.scrollHeight - window.innerHeight);
  const was = bar.hidden;
  bar = S.navOpen ? { ...next, hidden: false } : next;
  if (bar.hidden !== was || (y > 0) !== barEl()?.classList.contains('topbar-scrolled')) paintBar();
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
  const ai = aiStatus();
  // v0.8: the AI+ badge (round 2: bright while the user can use AI now, dark while they have the AI+ service but cannot use
  // it now, none without the service; domain/badge-state.ts)
  const badge = ownBadgeState(user, ai);
  // v1.1 (BT-2): the features that are off name themselves in the bright badge's tip (administrators have them all)
  const off = ai.state === 'ready' && user?.role !== 'admin' ? offList(readFeatures(ai.features)) : [];

  // A new page, or an open menu, always shows the bar
  const here = routeHash(S.route);
  if (here !== barRoute || open) {
    barRoute = here;
    bar = initialBar(Math.max(0, window.scrollY));
  }

  // Who is signed in, and Sign out: the bottom of the menu, under a hairline
  const account = user ? h('div', { class: 'nav-account' },
    h('div', { class: 'nav-who' },
      icon(ICONS.user),
      h('span', { class: 'nav-who-name', title: user.username }, user.username),
      h('span', { class: 'nav-who-tags' }, roleTags(user), aiBadge(badge, { off }))),
    // MN-4: the dark badge's reason in words under the name; a tooltip is not reachable on a touch screen
    badge.kind === 'off' && badge.reason === 'suspended' ? h('p', { class: 'nav-paused' }, t('nav.aiPaused')) : null,
    h('button', { class: 'nav-link nav-signout', type: 'button', onclick: () => void signOut() },
      icon(ICONS.signOut), h('span', { class: 'nav-label' }, t('nav.signOut')))) : null;

  const menu = open ? h('nav', { class: 'nav-pop', id: 'nav-menu', 'aria-label': t('nav.label') },
    h('ul', { class: 'nav-list' }, NAV.filter((n) => navAllowed(n.need, user, ai)).map((n) => h('li', null,
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
  // (v0.8: with the badge the label says "AI+ on" too, because the aria-label replaces what the button shows; round 2: with
  // the dark badge it says why AI+ cannot be used now, since nothing inside a button can be focused to read its tooltip)
  const accountLabel = !user ? ''
    : badge.kind === 'on' ? t('nav.accountLabelAi', { name: user.username, role: roleText(user) })
      : badge.kind === 'off' ? t('nav.accountLabelAiOff', { name: user.username, role: roleText(user), reason: badgeReasonText(badge.reason) })
        : t('nav.accountLabel', { name: user.username, role: roleText(user) });
  // v1.1 (BT-6): no native title on the button (it would cover the badge's tip); on touch the badge tip is a tap-and-hold
  const accountBtn0 = user ? h('button', {
    class: 'acct-btn' + (open ? ' on' : '') + (badge.kind !== 'none' ? ' has-ai' : ''), id: 'acct-btn', type: 'button',
    'aria-label': accountLabel,
    'aria-haspopup': 'true', 'aria-expanded': String(open), 'aria-controls': 'nav-menu',
    onclick: (e: MouseEvent) => {
      focusMenuOnOpen = !open && e.detail === 0;
      opener = 'acct-btn';
      setNavOpen(!open);
    },
  },
  icon(ICONS.user, 'ico acct-ico'),
  h('span', { class: 'acct-name' }, user.username),
  aiBadge(badge, { inButton: true, off }),
  // Phones show the short form ("Admin"), so the username keeps its room; the full role is in the label and in the menu
  admin ? h('span', { class: 'tag b acct-role' },
    h('span', { class: 'acct-role-l' }, roleName('admin')),
    h('span', { class: 'acct-role-s' }, t('role.admin.short'))) : null) : null;
  const accountTip = badgeTipText(badge, 'own', off);
  const accountBtn = accountBtn0 && accountTip ? holdTip(accountBtn0, accountTip, { touchOnly: true }) : accountBtn0;

  // The logo on the left, the account label and the menu button on the right (the dropdown opens towards the left,
  // aligned to the right edge)
  // The classes follow the bar state from the start, so a redraw while the bar is hidden does not make it slide in and out
  const pinned = S.route.name !== 'ai';
  const barClass = 'topbar' + (pinned ? ' topbar-pin' : '') + (pinned && bar.hidden && !open ? ' topbar-hide' : '')
    + (pinned && window.scrollY > 0 ? ' topbar-scrolled' : '');
  const header = h('header', { class: barClass },
    h('a', { class: 'top-brand', href: '#/', onclick: (e: MouseEvent) => { e.preventDefault(); navigate({ name: 'home' }); } }, brand()),
    h('div', { class: 'menu-wrap' }, accountBtn, menuBtn, menu));

  if (open && focusMenuOnOpen) {
    focusMenuOnOpen = false;
    queueMicrotask(() => document.querySelector<HTMLElement>('#nav-menu .nav-link.on, #nav-menu .nav-link')?.focus());
  }
  // .main-body takes the height left over, so on a short page the footer sits at the screen bottom (owner, v1.0)
  return h('div', { class: 'shell' }, h('main', { class: 'main' + (pinned ? ' main-pin' : ''), id: 'main' },
    h('div', { class: 'main-body' }, header, content), footer()));
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
  // MN-2: outside the menu and its buttons only a tap closes it (the pointer is lifted within 10 px and 500 ms of going
  // down), so a swipe that scrolls the page, or a press that turns into a long press, leaves it open
  let down: { x: number; y: number; at: number } | null = null;
  const outside = (e: Event): boolean => {
    const wrap = document.querySelector('.menu-wrap');
    return !!wrap && e.target instanceof Node && !wrap.contains(e.target);
  };
  document.addEventListener('pointerdown', (e) => {
    down = S.navOpen && outside(e) ? { x: e.clientX, y: e.clientY, at: e.timeStamp } : null;
  });
  document.addEventListener('pointercancel', () => {
    down = null;
  });
  document.addEventListener('pointerup', (e) => {
    const d = down;
    down = null;
    if (!d || !S.navOpen || !outside(e)) return;
    if (Math.hypot(e.clientX - d.x, e.clientY - d.y) <= 10 && e.timeStamp - d.at <= 500) {
      setNavOpen(false);
      swallowNextClick();
    }
  });
  // The top bar (SH-7): hides on a scroll down, comes back on a scroll up, on focus inside it and while the menu is open
  window.addEventListener('scroll', onPageScroll, { passive: true });
  window.addEventListener('resize', afterShellRender);
  document.addEventListener('focusin', (e) => {
    if (e.target instanceof Node && barEl()?.contains(e.target)) showBar();
    // Tab leaving the menu closes it without moving focus (otherwise focus lands under the open menu)
    if (S.navOpen && outside(e)) closeMenuKeepingFocus(e.target);
  });
}

/** The child indexes from <body> down to `node` (the closed menu changes only .menu-wrap, so paths outside it stay valid) */
function pathOf(node: Node): number[] | null {
  const out: number[] = [];
  let n: Element | null = node instanceof Element ? node : null;
  while (n && n !== document.body) {
    const parent: Element | null = n.parentElement;
    if (!parent) return null;
    out.unshift(Array.prototype.indexOf.call(parent.children, n));
    n = parent;
  }
  return n ? out : null;
}

/**
 * Closes the menu after Tab took the focus outside it. The render rebuilds the page and keepFocus only finds elements with
 * an id, so the element that took the focus is found again by its place in the page (otherwise the focus would fall back
 * to <body> and the next Tab would start at the top).
 */
function closeMenuKeepingFocus(target: EventTarget | null): void {
  const path = target instanceof Node ? pathOf(target) : null;
  setNavOpen(false);
  if (!path || (target instanceof Node && target.isConnected)) return;
  let el: Element | undefined = document.body;
  for (const i of path) el = el?.children[i];
  if (el instanceof HTMLElement && document.activeElement !== el) el.focus({ preventScroll: true });
}
