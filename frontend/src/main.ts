// App entry: applies the cached settings (first paint), maps the route to a page inside the shell, registers the redraw function, and starts.
// Before anything else, the sign-in pages: while S.authView is set, only the sign-in page or "choose a new password" is drawn.
import './styles/main.css';
import { studyTransport } from './api/ai.ts';
import { isTooManyRequests } from './api/client.ts';
import { installAiPanel, reanchorAiPanel } from './components/ai-panel.ts';
import { installAiSummary } from './components/ai-summary.ts';
import type { StudyUiDeps } from './components/ai-study-ui.ts';
import { brand } from './components/common.ts';
import { githubLine } from './components/github-line.ts';
import { afterShellRender, installShellHandlers, resetTopH, shell } from './components/shell.ts';
import { installTooltips } from './components/tooltip.ts';
import { t, uiLang } from './i18n/index.ts';
import { errorText, setAdminForTexts, studyErrorText } from './i18n/errors.ts';
import { renderAbout } from './pages/about.ts';
import { renderAiChat, renderAiNoAccess, renderAiUnavailable } from './pages/ai.ts';
import { renderAiSettings } from './pages/ai-settings.ts';
import { renderNewPassword, renderSignIn } from './pages/auth.ts';
import { renderBanks } from './pages/banks.ts';
import { renderForbidden } from './pages/forbidden.ts';
import { renderHome } from './pages/home.ts';
import { renderLogs } from './pages/logs.ts';
import { installQuizKeys, renderQuiz } from './pages/quiz.ts';
import { renderRecords } from './pages/records.ts';
import { renderResult } from './pages/result.ts';
import { renderSettings } from './pages/settings.ts';
import { renderUsers } from './pages/users.ts';
import { boot, refreshAi, setRenderer } from './state/actions.ts';
import { aiStatus, onCallBlockChange } from './state/ai.ts';
import { applySettings, watchSystemTheme } from './state/preferences.ts';
import { S, canManageBanks, isAdmin } from './state/store.ts';
import { h, keepFocus, must } from './utils/dom.ts';
import { BAR_MQ, matches } from './utils/media.ts';

function solo(...kids: Array<HTMLElement | null>): HTMLElement {
  return h('div', { class: 'solo' }, h('div', { class: 'solo-brand' }, brand()), kids);
}

/** The page for the current route, and its title */
function page(): { el: HTMLElement; title: string } {
  const r = S.route;
  switch (r.name) {
    case 'banks':
      // Bank managers and administrators; anyone else sees why they cannot use it (owner, v0.6.5)
      return canManageBanks()
        ? { el: renderBanks(), title: t('nav.banks') }
        : { el: renderForbidden('forbidden.banks'), title: t('forbidden.title') };
    case 'users':
      // Administrators only; everyone else sees why (US-12), like the other administrator pages
      return isAdmin()
        ? { el: renderUsers(), title: t('users.title') }
        : { el: renderForbidden('forbidden.users'), title: t('forbidden.title') };
    case 'logs':
      // Administrators only (v0.7); bank managers and users see why they cannot open it
      return isAdmin()
        ? { el: renderLogs(), title: t('logs.title') }
        : { el: renderForbidden('forbidden.logs'), title: t('forbidden.title') };
    case 'records':
    case 'record':
    case 'wrong':
      return { el: renderRecords(r), title: t('nav.records') };
    case 'result':
      return { el: renderResult(), title: t('title.result') };
    case 'settings':
      return { el: renderSettings(), title: t('nav.settings') };
    case 'about':
      return { el: renderAbout(), title: t('about.title') };
    case 'ai': {
      // v0.8: decided only after the first AI status answer (or failure), so a reload never flashes "Not allowed"
      const ai = aiStatus();
      if (ai.state === 'loading') {
        return { el: h('div', { class: 'page page-ai' }, h('p', { class: 'dim', role: 'status' }, t('ai.loading'))), title: t('ai.title') };
      }
      if (ai.state === 'ready' && ai.allowed) return { el: renderAiChat('full'), title: t('ai.title') };
      // AI+ or AI switched off, but the user has chats or memories: they can still read and delete them
      if (ai.state === 'ready' && ai.hasData) return { el: renderAiChat('readOnly'), title: t('ai.title') };
      // The AI service cannot be reached: say so (with Retry) instead of blaming the account
      if (ai.state === 'unavailable') return { el: renderAiUnavailable(), title: t('ai.title') };
      // No AI+ function / paused / the feature off, and nothing to read: say why and what to do, not a bare "Not allowed"
      return { el: renderAiNoAccess(), title: t('ai.title') };
    }
    case 'aiSettings':
      return isAdmin()
        ? { el: renderAiSettings(), title: t('ai.set.title') }
        : { el: renderForbidden('forbidden.aiSettings'), title: t('forbidden.title') };
    default:
      return { el: renderHome(), title: t('nav.home') };
  }
}

/**
 * The field an input method (Chinese, Japanese) is composing in. Rebuilding the page then would end the composition and
 * lose the word being composed, so a render asked for meanwhile (a background load, the AI status) waits until the
 * composition ends or the field loses focus.
 */
let composingIn: Element | null = null;
let renderWaiting = false;

function compositionOver(): void {
  composingIn = null;
  if (!renderWaiting) return;
  renderWaiting = false;
  // After the committing input event, so the page's state already holds the committed text
  window.setTimeout(render, 0);
}

document.addEventListener('compositionstart', (e) => {
  composingIn = e.target instanceof Element ? e.target : null;
}, true);
document.addEventListener('compositionend', compositionOver, true);
document.addEventListener('focusout', (e) => {
  if (composingIn && e.target === composingIn) compositionOver();
}, true);

/** Every render rebuilds the page; keepFocus puts the keyboard focus (and a text field's caret) back on the element with the same id. */
function render(): void {
  if (composingIn && composingIn.isConnected && document.activeElement === composingIn) {
    renderWaiting = true;
    return;
  }
  composingIn = null;
  renderWaiting = false;
  keepFocus(draw);
}

function draw(): void {
  if (S.timer != null) window.clearInterval(S.timer);
  S.timer = null;
  const app = must<HTMLElement>('#app');

  // Pages without the shell pin nothing at the top (the quiz page sets `--top-h` itself)
  if (!(S.route.name === 'quiz' && S.session) && (S.authView || S.phase !== 'ready')) resetTopH();

  // The sign-in pages set their own document title
  if (S.authView === 'signin') {
    app.replaceChildren(renderSignIn());
    return;
  }
  if (S.authView === 'password') {
    app.replaceChildren(renderNewPassword());
    return;
  }
  if (S.phase === 'loading') {
    app.replaceChildren(solo(h('p', { class: 'dim', role: 'status' }, t('app.loading'))));
    document.title = t('app.name');
    return;
  }
  if (S.phase === 'fatal') {
    // Too many requests (the proxy's rate limit, still refusing after the automatic retries) is its own message: the service
    // is reachable and the sign-in is fine, so neither "cannot reach the service" nor the sign-in hint applies
    const tooMany = isTooManyRequests(S.fatal);
    app.replaceChildren(solo(h('div', { class: 'err', role: 'alert' },
      h('h2', null, t(tooMany ? 'app.tooMany.title' : 'app.fatal.title')),
      h('p', null, tooMany ? t('app.tooMany.body') : S.fatal ? errorText(S.fatal) : t('app.fatal.body')),
      tooMany ? null : h('p', { class: 'dim' }, t('app.fatal.body')),
      h('div', { class: 'acts' }, h('button', { class: 'btn primary', type: 'button', onclick: () => window.location.reload() }, t('app.reload')))),
      // The page's last line, as on every page (the version part only when /api/info was read)
      githubLine()));
    document.title = t('app.name');
    return;
  }
  if (S.route.name === 'quiz' && S.session) {
    app.replaceChildren(renderQuiz());
    reanchorAiPanel();
    document.title = `${t('title.quiz', { i: S.session.idx + 1, n: S.session.qs.length })} - ${t('app.name')}`;
    return;
  }
  const p = page();
  app.replaceChildren(shell(p.el));
  // The bar's measured height (`--top-h`) and the menu's height first: the AI+ panel's scroll rule reads them
  afterShellRender();
  reanchorAiPanel();
  document.title = `${p.title} - ${t('app.name')}`;
}

applySettings(S.settings);
watchSystemTheme();
setRenderer(render);
installShellHandlers();
// v0.9: the AI+ panel (v1.0: a panel inside the page) and the AI summary window (in document.body, outside the page)
const studyDeps: StudyUiDeps = {
  transport: studyTransport,
  uiLang,
  // a known AI code has its own text; anything else is the generic network / status text
  errorText: studyErrorText,
  langName: (l) => t(l === 'en' ? 'ai.langName.en' : l === 'ja' ? 'ai.langName.ja' : 'ai.langName.zh'),
  // actions.ts draws the page again when the status changed
  refreshAi: (force) => void refreshAi(force),
};
installAiPanel(studyDeps);
installAiSummary(studyDeps);
// v1.0: the texts of the AI request caps name the administrator's settings only for administrators; a change of the caps
// block redraws the page, so the AI controls enable or disable themselves
setAdminForTexts(() => S.user?.role === 'admin');
onCallBlockChange(render);
// v1.0: under BAR_MQ the practice action bar sits under the AI+ panel, above it otherwise (spec 2.6): the quiz page is
// drawn again when the width crosses that line. Nothing else depends on the width in code (the rest is CSS).
if (typeof window.matchMedia === 'function') {
  const barQuery = window.matchMedia(BAR_MQ);
  let barShown = matches(BAR_MQ);
  barQuery.addEventListener('change', () => {
    if (barQuery.matches === barShown) return;
    barShown = barQuery.matches;
    if (S.route.name === 'quiz' && S.session) render();
  });
}
// v1.0 (SH-15): the AI+ badge's animation stops while the tab is hidden (`html.is-hidden`, styles/main.css)
document.addEventListener('visibilitychange', () => {
  document.documentElement.classList.toggle('is-hidden', document.hidden);
});
installTooltips();
installQuizKeys();
void boot();
