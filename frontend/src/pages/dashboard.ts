// Home dashboard (v2.5, v2.6): a greeting, the page tiles, the AI study assessment card, the leaderboard and the chalk curve of the
// viewer's most missed questions. One calm column. The data is one read-only request per visit (state/dashboard.ts); the
// stems are resolved from the loaded bank, so no question text travels over the API. The old Home (exam setup) is the
// "Exam & practice" page (pages/study.ts).
import { planeDoodle } from '../components/doodle.ts';
import { rankBoard, rankBoardSkeleton, rankPoints } from '../components/rank-board.ts';
import '../styles/dashboard.css';
import { AI_BADGE_TEXT } from '../components/ai-badge.ts';
import { aiPlusNow } from '../components/ai-plus-controls.ts';
import { boardSkeleton, chalkBoard, keepPick } from '../components/chalk-board.ts';
import { bankName } from '../components/common.ts';
import { miniIcon } from '../components/mini-icons.ts';
import { ICONS, icon } from '../components/shell.ts';
import { assessExtra, controlState, newestRecordAt } from '../domain/ai-plus.ts';
import type { AskError } from '../domain/ai-study.ts';
import { assessDay, capsuleAction, capsuleVisible, remainingNote } from '../domain/dash-assess.ts';
import { greetName, rankNote, wrongRows, type WrongRow } from '../domain/dash.ts';
import { studyErrorText } from '../i18n/errors.ts';
import { t } from '../i18n/index.ts';
import { routeHash, type Route } from '../router.ts';
import { navigate, onRouteChange } from '../state/actions.ts';
import { navAllowed } from '../state/bank-access.ts';
import { aiStatus } from '../state/ai.ts';
import { assessView, ensureAssess, generateAssess, stopAssess } from '../state/dash-assess.ts';
import { dashView, ensureDashboard, retryDashboard } from '../state/dashboard.ts';
import { S, onClearUserData } from '../state/store.ts';
import { h, type Child } from '../utils/dom.ts';
import { resumeCard } from './study.ts';

const CHEVRON = [{ d: 'M9.5 6l6 6-6 6' }];

/** An <a> that opens in a new tab as usual and navigates in place on a plain click */
function tile(route: Route, cls: string, ...kids: Child[]): HTMLAnchorElement {
  return h('a', {
    class: cls, href: routeHash(route),
    onclick: (e: MouseEvent) => {
      if (e.defaultPrevented || e.button !== 0 || e.metaKey || e.ctrlKey || e.shiftKey || e.altKey) return;
      e.preventDefault();
      navigate(route);
    },
  }, ...kids);
}

function tiles(): HTMLElement {
  const small = (route: Route, ico: (typeof ICONS)[keyof typeof ICONS], label: string): HTMLAnchorElement =>
    tile(route, 'dash-tile', icon(ico, 'ico dash-ico'), h('span', { class: 'dash-tile-label' }, label));
  // Owner's order: study records, wrong-answer notebook, AI Chat, bank management, settings, profile
  const row: HTMLElement[] = [small({ name: 'records' }, ICONS.records, t('nav.records')), small({ name: 'wrong' }, ICONS.notebook, t('dash.tile.wrong'))];
  if (navAllowed('ai', S.user, aiStatus())) row.push(small({ name: 'ai' }, ICONS.sparkle, t('nav.aiChat')));
  // The same rule as the menu entry: whoever may manage or download banks
  if (navAllowed('banks', S.user)) row.push(small({ name: 'banks' }, ICONS.banks, t('nav.banks')));
  row.push(small({ name: 'settings' }, ICONS.gear, t('nav.settings')), small({ name: 'profile' }, ICONS.user, t('nav.profile')));
  return h('nav', { class: 'dash-tiles', 'aria-label': t('dash.tiles') },
    tile({ name: 'study' }, 'dash-tile dash-hero',
      icon(ICONS.study, 'ico dash-ico'),
      h('span', { class: 'dash-hero-text' },
        h('span', { class: 'dash-hero-label' }, t('nav.study')),
        h('span', { class: 'dash-hero-sub' }, t('dash.hero.sub'))),
      planeDoodle(),
      icon(CHEVRON, 'ico dash-chev')),
    // data-n lets the stylesheet lay the tiles out in rows of three (four: two by two on phones)
    h('div', { class: 'dash-row', 'data-n': String(row.length) }, ...row));
}

function hello(): HTMLElement {
  // The same newest time the AI study assessment uses (the largest `at`, not the first row by id)
  const at = newestRecordAt(S.records);
  const sub = S.records == null ? null : at !== null
    ? h('p', { class: 'dash-sub' }, t('dash.last', { at }))
    : h('p', { class: 'dash-sub' }, t('dash.new'));
  return h('header', { class: 'dash-hello' }, h('h1', null, t('dash.hello', { name: S.user ? greetName(S.user) : '' })), sub);
}

// ---------------------------------------------------------------- the AI study assessment card

function waitReason(r: 'running' | 'daily' | 'few'): string {
  return r === 'daily' ? t('dash.ai.wait.daily') : r === 'few' ? t('aiplus.why.assessFew') : t('dash.ai.writing');
}

/** An inline error of the card: today's assessment used up has its own text, the rest are the AI texts */
function assessErrorText(e: AskError): string {
  return e.code === 'ai.assessDaily' ? t('dash.ai.err.daily') : studyErrorText(e);
}

/** The card: the saved text, a button, and a new text streaming in; null when there is nothing to show */
// Owner: the AI assessment card is folded every time the Home opens; its title row opens and closes it (a running
// assessment opens it, so the new text is seen)
let aiOpen = false;
// A run opens the card once per run, so folding it while the text streams is respected on the next delta
let autoOpened = false;
onRouteChange((from, to) => { if (to.name === 'home' && from.name !== 'home') { aiOpen = false; autoOpened = false; } });
// A sign-out starts the next visitor with a folded card
onClearUserData(() => { aiOpen = false; autoOpened = false; });

function aiCard(): HTMLElement | null {
  ensureAssess();
  const { latest, run } = assessView();
  const saved = latest?.assess ?? null;
  const streaming = run.phase === 'streaming';
  const extra = assessExtra(S.records);
  const action = capsuleAction({
    ctl: controlState(aiPlusNow(), null, 'assess'),
    hasSaved: saved !== null, remaining: latest?.remaining ?? null, perDay: latest?.perDay ?? null, running: streaming || latest?.running === true, few: extra !== null,
  });
  if (!capsuleVisible({ hasSaved: saved !== null, action, run })) return null;

  // While a new text arrives it takes the place of the old one; an error leaves the old one standing
  const fresh = run.phase === 'streaming' || run.phase === 'done';
  const text = fresh ? run.text : saved?.text ?? '';
  const body = text
    ? h('p', { class: 'dash-ai-text' }, text)
    : h('p', { class: 'dash-ai-text dim' }, streaming ? t('dash.ai.writing') : t('dash.ai.sub'));

  const meta: string[] = [];
  if (saved && !fresh) {
    const day = assessDay(saved);
    if (day) meta.push(t('dash.ai.dated', { date: day }));
    // Owner: the card never names the model (not even for administrators); only when it was made
  }
  // How many assessments are left today (only a viewer with a limit sees it; none left is said by the disabled button)
  // and only while the button can be used (no AI+, too few records or a running text contradict it)
  const left = action.kind === 'ask' ? remainingNote(latest?.remaining ?? null) : null;
  if (left !== null) meta.push(t('dash.ai.remaining', { n: left }));

  if (streaming && !autoOpened) { aiOpen = true; autoOpened = true; } else if (!streaming) autoOpened = false;
  const kids: Child[] = [body];
  if (meta.length > 0) kids.push(h('p', { class: 'dash-ai-meta' }, meta.join(' · ')));
  if (run.phase === 'error') kids.push(h('p', { class: 'dash-ai-err', role: 'alert' }, assessErrorText(run.error)));

  if (streaming) {
    kids.push(h('div', { class: 'dash-ai-foot' },
      h('button', { id: 'dash-ai-act', class: 'btn quiet', type: 'button', onclick: () => stopAssess() }, t('dash.ai.stop'))));
  } else if (action.kind !== 'none') {
    const wait = action.kind === 'wait';
    const btn = h('button', {
      id: 'dash-ai-act', class: 'btn aip-sum-btn dash-ai-btn', type: 'button', 'aria-disabled': wait ? 'true' : null,
      'aria-describedby': wait ? 'dash-ai-why' : null,
      onclick: () => { if (!wait) generateAssess(); },
    }, miniIcon('sparkle', 'aip-mi'), t(action.label === 'update' ? 'dash.ai.update' : 'dash.ai.generate'));
    kids.push(h('div', { class: 'dash-ai-foot' }, btn,
      action.kind === 'wait' ? h('p', { class: 'dash-note', id: 'dash-ai-why' }, waitReason(action.reason)) : null));
  }
  const inner = h('div', { class: 'dash-ai-body', id: 'dash-ai-body', hidden: !aiOpen }, ...kids);
  const toggle = h('button', {
    id: 'dash-ai-toggle', class: 'dash-ai-toggle', type: 'button', 'aria-expanded': String(aiOpen), 'aria-controls': 'dash-ai-body',
    onclick: () => {
      aiOpen = !aiOpen;
      toggle.setAttribute('aria-expanded', String(aiOpen));
      inner.hidden = !aiOpen;
    },
  },
  h('span', { class: 'dash-ai-title' }, t('dash.ai.title')),
  h('span', { class: 'ai-badge', 'aria-hidden': 'true' },
    h('span', { class: 'ai-badge-t' }, AI_BADGE_TEXT.slice(0, -1), h('span', { class: 'ai-badge-plus' }, AI_BADGE_TEXT.slice(-1)))),
  icon(CHEVRON, 'ico dash-ai-chev'));
  return h('section', { class: 'dash-ai', 'aria-labelledby': 'dash-ai-h', 'aria-busy': streaming ? 'true' : null },
    h('h2', { id: 'dash-ai-h', class: 'dash-ai-head' }, toggle), inner);
}

// ---------------------------------------------------------------- blocks

function failed(): HTMLElement {
  return h('div', { class: 'dash-fail' },
    h('p', { class: 'dim' }, t('dash.err')),
    h('button', { class: 'btn quiet', type: 'button', onclick: () => retryDashboard() }, t('common.retry')));
}

const head = (id: string, title: string, meta: string): HTMLElement =>
  h('div', { class: 'dash-h' }, h('h2', { id }, title), h('span', { class: 'dash-meta' }, meta));

function rankBody(): HTMLElement {
  const v = dashView();
  if (v.error) return failed();
  if (!v.data) return rankBoardSkeleton();
  const { top, me } = v.data.ranking;
  // One line at most: an empty board says only the invitation, never "nobody has answered" as well
  const note = rankNote(top.length, me.rank);
  const noteKey = note === 'noneYou' ? 'dash.rank.noneYou' : 'dash.rank.empty';
  // v2.6 (owner): a hand-drawn rising line on the board (components/rank-board.ts); the viewer's own point follows the top
  // three when they are ranked outside it
  return h('div', null,
    top.length > 0
      ? h('div', null,
        rankBoard(rankPoints(top, me)),
        note ? h('p', { class: 'dim dash-none' }, t(noteKey)) : null)
      : h('p', { class: 'empty' }, t(noteKey)));
}

// ---------------------------------------------------------------- the viewer's most missed questions (components/chalk-board.ts)

const wrongStats = (row: WrongRow): string =>
  t('dash.wrong.stats', { bank: bankName(row.bankId), sheet: row.sheet, attempts: row.attempts, wrong: row.wrong });

function wrongBody(): HTMLElement {
  const v = dashView();
  if (v.error) return failed();
  // The stems come from the loaded bank: keep the skeleton until it arrives
  if (!v.data || !S.bank) return boardSkeleton();
  const rows = wrongRows(v.data.wrongTop.items, S.bank, S.settings.questionLang);
  keepPick(rows.map((r) => r.id));
  return rows.length > 0 ? chalkBoard(rows, wrongStats) : h('p', { class: 'empty' }, t('dash.wrong.empty'));
}

export function renderDashboard(): HTMLElement {
  ensureDashboard();
  return h('div', { class: 'page page-dash' },
    hello(),
    resumeCard(),
    tiles(),
    h('section', { class: 'panel dash-rank', 'aria-labelledby': 'dash-rank-h' },
      head('dash-rank-h', t('dash.rank.title'), t('dash.rank.basis')), rankBody()),
    h('section', { class: 'panel dash-wrong', 'aria-labelledby': 'dash-wrong-h' },
      head('dash-wrong-h', t('dash.wrong.title'), t('dash.wrong.meta')), wrongBody()),
    aiCard());
}
