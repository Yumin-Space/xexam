// Home dashboard (v2.5): a greeting, the page tiles, the AI study assessment, the leaderboard and the most practised questions.
// One calm column. The data is one read-only request per visit (state/dashboard.ts); the stems are resolved from the loaded bank,
// so no question text travels over the API. The old Home (exam setup) is the "Exam & practice" page (pages/study.ts).
import '../styles/dashboard.css';
import type { RankEntry, RankMe } from '../api/dashboard.ts';
import { bankName } from '../components/common.ts';
import { aiPlusNow, assessButton } from '../components/ai-plus-controls.ts';
import { ICONS, icon } from '../components/shell.ts';
import { controlState, assessExtra, newestRecordAt } from '../domain/ai-plus.ts';
import { greetName, popularRows, rankNote, type PopularRow } from '../domain/dash.ts';
import { t } from '../i18n/index.ts';
import { routeHash, type Route } from '../router.ts';
import { navigate } from '../state/actions.ts';
import { navAllowed } from '../state/bank-access.ts';
import { aiStatus } from '../state/ai.ts';
import { dashView, ensureDashboard, retryDashboard } from '../state/dashboard.ts';
import { S } from '../state/store.ts';
import { h, type Child } from '../utils/dom.ts';
import { resumeCard } from './study.ts';

const SKELETON_ROWS = 3;
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
  const row: HTMLElement[] = [small({ name: 'records' }, ICONS.records, t('nav.records'))];
  if (navAllowed('ai', S.user, aiStatus())) row.push(small({ name: 'ai' }, ICONS.sparkle, t('nav.aiChat')));
  row.push(small({ name: 'settings' }, ICONS.gear, t('nav.settings')));
  return h('nav', { class: 'dash-tiles', 'aria-label': t('dash.tiles') },
    tile({ name: 'study' }, 'dash-tile dash-hero',
      icon(ICONS.study, 'ico dash-ico'),
      h('span', { class: 'dash-hero-text' },
        h('span', { class: 'dash-hero-label' }, t('nav.study')),
        h('span', { class: 'dash-hero-sub' }, t('dash.hero.sub'))),
      icon(CHEVRON, 'ico dash-chev')),
    h('div', { class: 'dash-row' }, ...row));
}

function hello(): HTMLElement {
  // The same newest time the AI study assessment uses (the largest `at`, not the first row by id)
  const at = newestRecordAt(S.records);
  const sub = S.records == null ? null : at !== null
    ? h('p', { class: 'dash-sub' }, t('dash.last', { at }))
    : h('p', { class: 'dash-sub' }, t('dash.new'));
  return h('header', { class: 'dash-hello' }, h('h1', null, t('dash.hello', { name: greetName(S.user?.username ?? '') })), sub);
}

function aiRow(): HTMLElement | null {
  const view = aiPlusNow();
  if (!view.show) return null;
  const btn = assessButton(controlState(view, assessExtra(S.records), 'summary'), { anchorKey: 'dash-ai' });
  if (!btn) return null;
  return h('div', { class: 'dash-ai' }, btn, h('p', { class: 'dash-note' }, t('dash.ai.sub')));
}

const skeleton = (): HTMLElement =>
  h('ol', { class: 'dash-list dash-skel', 'aria-hidden': 'true' },
    ...Array.from({ length: SKELETON_ROWS }, () => h('li', null, h('span', { class: 'dash-bar' }))));

function failed(): HTMLElement {
  return h('div', { class: 'dash-fail' },
    h('p', { class: 'dim' }, t('dash.err')),
    h('button', { class: 'btn quiet', type: 'button', onclick: () => retryDashboard() }, t('common.retry')));
}

const head = (id: string, title: string, meta: string): HTMLElement =>
  h('div', { class: 'dash-h' }, h('h2', { id }, title), h('span', { class: 'dash-meta' }, meta));

function rankRow(e: RankEntry): HTMLElement {
  return h('li', { class: 'dash-rank-row' + (e.self ? ' dash-self' : '') },
    h('span', { class: 'num dash-no' + (e.rank === 1 ? ' first' : '') }, String(e.rank)),
    // The name has its own box so it can shrink with an ellipsis while the "You" tag beside it stays whole
    h('span', { class: 'dash-name', title: e.name },
      h('span', { class: 'dash-name-t' }, e.name), e.self ? h('span', { class: 'tag b dash-you' }, t('dash.rank.you')) : null),
    h('span', { class: 'num dash-n' }, String(e.answered)),
    h('span', { class: 'num dash-n dash-good' }, String(e.correct)));
}

/** The viewer's own line after the top three (only when they are ranked outside it) */
function meRow(me: RankMe): HTMLElement {
  return h('li', { class: 'dash-rank-row dash-me dash-self' },
    h('span', { class: 'num dash-no' }, String(me.rank)),
    h('span', { class: 'dash-name' }, h('span', { class: 'tag b dash-you' }, t('dash.rank.you'))),
    h('span', { class: 'num dash-n' }, String(me.answered)),
    h('span', { class: 'num dash-n dash-good' }, String(me.correct)));
}

function rankBody(): HTMLElement {
  const v = dashView();
  if (v.error) return failed();
  if (!v.data) return skeleton();
  const { top, me } = v.data.ranking;
  const outside = me.rank !== null && !top.some((e) => e.self);
  // One line at most: an empty board says only the invitation, never "nobody has answered" as well
  const note = rankNote(top.length, me.rank);
  const noteKey = note === 'noneYou' ? 'dash.rank.noneYou' : 'dash.rank.empty';
  return h('div', null,
    top.length > 0
      ? h('div', null,
        h('p', { class: 'dash-cols' }, t('dash.rank.cols')),
        h('ol', { class: 'dash-list' }, ...top.map(rankRow), outside ? meRow(me) : null),
        note ? h('p', { class: 'dim dash-none' }, t(noteKey)) : null)
      : h('p', { class: 'empty' }, t(noteKey)));
}

function hotRow(r: PopularRow): HTMLElement {
  return h('li', { class: 'dash-hot-row', title: r.stem },
    h('span', { class: 'dash-hot-text' },
      h('span', { class: 'dash-stem' }, r.stem),
      h('span', { class: 'dash-hot-meta' }, `${bankName(r.bankId)} · ${r.sheet}`)),
    h('span', { class: 'tag dash-times' }, t('dash.hot.times', { n: r.times })));
}

function hotBody(): HTMLElement {
  const v = dashView();
  if (v.error) return failed();
  // The stems come from the loaded bank: keep the skeleton until it arrives
  if (!v.data || !S.bank) return skeleton();
  const rows = popularRows(v.data.popular.items, S.bank);
  return rows.length > 0
    ? h('ol', { class: 'dash-list' }, ...rows.map(hotRow))
    : h('p', { class: 'empty' }, t('dash.hot.empty', { days: v.data.popular.days }));
}

export function renderDashboard(): HTMLElement {
  ensureDashboard();
  const days = dashView().data?.popular.days ?? 30;
  return h('div', { class: 'page page-dash' },
    hello(),
    resumeCard(),
    tiles(),
    aiRow(),
    h('section', { class: 'panel dash-rank', 'aria-labelledby': 'dash-rank-h' },
      head('dash-rank-h', t('dash.rank.title'), t('dash.rank.basis')), rankBody()),
    h('section', { class: 'panel dash-hot', 'aria-labelledby': 'dash-hot-h' },
      head('dash-hot-h', t('dash.hot.title'), t('dash.hot.window', { days })), hotBody()));
}
