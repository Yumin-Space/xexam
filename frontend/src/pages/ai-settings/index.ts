// AI settings (#/ai-settings, administrators only; main.ts shows "Not allowed" to everyone else). The page always renders: it
// first reads the state of ai-service from the MAIN service (GET /api/auth/state, `aiService`) and loads the settings only while
// ai-service is connected; otherwise it shows a status panel that names the cause and checks again every 10 s (status.ts).
// v1.3 layout: four status cards on top of every tab, then five category tabs (the hash keeps the tab:
// #/ai-settings?tab=feature | users | usage | safety | gen; a missing or unknown value is the first):
//   · Connections and models (`feature`): 1 connections (cards + form, test before save), 2 the AI+ switch (a default model,
//     tested), 3 seven model rows, each (except the default) with its own global feature switch (v1.9: it applies to everyone,
//     administrators included);
//   · Users: the AI+ users, their feature boxes, the viewer of their chats;   · Usage;   · Safety: retention and limits;
//   · AI banks: content rules and the recent jobs.
// One part per file under this folder; the state they share is in state.ts. Typed keys live only in that state and are
// forgotten after a successful save, when the page is left and on sign-out.
import '../../styles/ai-settings.css';
import { fetchAiSettings } from '../../api/ai-admin.ts';
import { pageHead } from '../../components/common.ts';
import { miniIcon, type MiniIcon } from '../../components/mini-icons.ts';
import { holdTip } from '../../components/tooltip.ts';
import { AI_TABS, hashOfTab, tabOfHash, type AiTab } from '../../domain/ai-settings.ts';
import { errorText } from '../../i18n/errors.ts';
import { t, type MsgKey } from '../../i18n/index.ts';
import { onRouteChange, onSignedIn, render } from '../../state/actions.ts';
import { onClearUserData } from '../../state/store.ts';
import { h, type Child } from '../../utils/dom.ts';
import { renderForbidden } from '../forbidden.ts';
import { connectionsSection, forgetConnectionForm } from './connections.ts';
import { genSection, loadGenJobs } from './gen.ts';
import { enableSection, forgetPending, loadModels, modelsSection } from './models.ts';
import { loadLimits, safetySections } from './safety.ts';
import { A, adopt, hooks, redraw, resetState } from './state.ts';
import { checkService, keyProblemLine, serviceLine, servicePanel, statusCardsRow, stopRecheck, aiDown } from './status.ts';
import { loadUsage, usageSection } from './usage.ts';
import { loadUsers, usersSection } from './users.ts';

/** The category tabs: the icon, and the full and the short label (the short one under PHONE) */
const TABS: Record<AiTab, { icon: MiniIcon; full: MsgKey; short: MsgKey }> = {
  feature: { icon: 'plug', full: 'ai.set.tab.conn', short: 'ai.set.tab.connShort' },
  users: { icon: 'users', full: 'ai.set.tab.users', short: 'ai.set.tab.usersShort' },
  usage: { icon: 'chart', full: 'ai.set.tab.usage', short: 'ai.set.tab.usageShort' },
  safety: { icon: 'shield', full: 'ai.set.tab.safety', short: 'ai.set.tab.safetyShort' },
  gen: { icon: 'aiGen', full: 'ai.set.tab.gen', short: 'ai.set.tab.genShort' },
};

// ---------------------------------------------------------------- loading

async function load(): Promise<void> {
  A.wanted = false;
  A.loading = true;
  try {
    const s = await fetchAiSettings();
    A.loadError = null;
    adopt(s);
    A.retentionText = String(s.retentionDays);
    void loadUsers();
    void loadUsage();
    void loadLimits();
    void loadGenJobs();
    // The merged model list (cached by the service); nothing to list without a connection
    if (s.connections.length && !A.models) void loadModels(false);
  } catch (e) {
    A.loading = false;
    if (aiDown(e)) return;
    A.loadError = e;
  } finally {
    A.loading = false;
  }
  redraw();
}

hooks.load = load;
hooks.selectTab = (k) => selectTab(k);

onRouteChange((from, to) => {
  if (to.name === 'aiSettings') {
    A.wanted = true;
    A.forbidden = false;
  }
  if (from.name === 'aiSettings' && to.name !== 'aiSettings') {
    stopRecheck();
    // Typed keys never stay in memory once the page is left
    forgetConnectionForm();
    A.connMsg = null;
    A.enableMsg = null;
    A.manualOpen = null;
  }
});
onClearUserData(resetState);
onSignedIn(() => {
  A.wanted = true;
  A.loadError = null;
});

// ---------------------------------------------------------------- the page

/** The row of category tabs under the status cards: icon + label, short labels under PHONE, one row */
function tabsRow(tab: AiTab): HTMLElement {
  return h('div', { class: 'seg ais-tabs', role: 'group', 'aria-label': t('ai.set.tabs.aria') },
    AI_TABS.map((k) => {
      const d = TABS[k];
      const b = h('button', {
        type: 'button', id: `ais-tab-${k}`, 'aria-pressed': String(k === tab), 'aria-controls': 'ais-card', onclick: () => selectTab(k),
      }, miniIcon(d.icon, 'ais-tab-ic'), h('span', { class: 'ais-tl-full' }, t(d.full)), h('span', { class: 'ais-tl-short', 'aria-hidden': 'true' }, t(d.short)));
      // The full name is the accessible name and, by hold (touch) or hover, the tip
      holdTip(b, t(d.full));
      return b;
    }));
}

/** Shows a category: the hash keeps it (`#/ai-settings?tab=users`), replaced in place, so Back does not walk through tabs */
function selectTab(k: AiTab): void {
  if (tabOfHash(window.location.hash) === k && window.location.hash.includes('tab=')) return;
  try {
    window.history.replaceState(window.history.state, '', hashOfTab(k));
  } catch {
    window.location.hash = hashOfTab(k);
  }
  // A connection chosen in a model row with no model picked does not survive a category change
  forgetPending();
  render();
  document.getElementById(`ais-tab-${k}`)?.focus();
}

export function renderAiSettings(): HTMLElement {
  if (A.wanted && !A.svcBusy) void checkService();
  if (A.forbidden) return renderForbidden('forbidden.aiSettings');
  const head = pageHead(t('ai.set.title'));
  if (A.phase === 'panel') return h('div', { class: 'page page-ais' }, head, servicePanel());
  const s = A.settings;
  if (!s) {
    let body: Child;
    if (A.phase === 'settings' && A.loadError) {
      body = h('div', { class: 'err', role: 'alert' }, h('p', null, t('ai.set.err', { reason: errorText(A.loadError) })),
        h('div', { class: 'acts' }, h('button', { class: 'btn', type: 'button', onclick: () => void checkService(true) }, t('common.retry'))));
    } else {
      body = h('p', { class: 'dim', role: 'status' }, t('common.loading'));
    }
    return h('div', { class: 'page page-ais' }, head, body);
  }
  const tab = tabOfHash(window.location.hash);
  // Only the shown category is in the DOM; the status cards stay above in every tab
  let card: Child[];
  switch (tab) {
    case 'users':
      card = [usersSection(s)];
      break;
    case 'usage':
      card = [usageSection()];
      break;
    case 'safety':
      card = safetySections();
      break;
    case 'gen':
      card = genSection();
      break;
    default:
      // Connections, the AI+ master switch, then the model rows (each with its feature switch, v1.9)
      card = [connectionsSection(s), enableSection(s), modelsSection(s)];
  }
  return h('div', { class: 'page page-ais' },
    head,
    statusCardsRow(s),
    keyProblemLine(s),
    serviceLine(),
    tabsRow(tab),
    h('section', { class: 'ais-card', id: 'ais-card', 'aria-labelledby': `ais-tab-${tab}` }, card));
}
