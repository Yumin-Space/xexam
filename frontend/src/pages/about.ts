// About page: version, server, license, and what is new in each version (the changelog of the README in the UI language).
// The changelog is a separate chunk (dynamic import), loaded only when this page is opened.
import '../styles/about.css';
import { pageHead } from '../components/common.ts';
import { blocks, inlineTokens, type ChangelogEntry } from '../domain/changelog.ts';
import { t, uiLang } from '../i18n/index.ts';
import { render } from '../state/actions.ts';
import { S } from '../state/store.ts';
import { h } from '../utils/dom.ts';

type Changelog = Record<'en' | 'zh' | 'ja', ChangelogEntry[]>;

let data: Changelog | null = null;
let state: 'idle' | 'loading' | 'failed' = 'idle';

function load(): void {
  if (data || state === 'loading') return;
  state = 'loading';
  import('virtual:changelog').then((m) => {
    data = m.default;
    state = 'idle';
    if (S.route.name === 'about') render();
  }, () => {
    state = 'failed';
    if (S.route.name === 'about') render();
  });
}

/** One line of Markdown as DOM: text, bold, inline code; links become text (they point into the repository, not the app). */
function inline(text: string): Array<Node | string> {
  return inlineTokens(text).map((tok) => {
    switch (tok.kind) {
      case 'bold':
        return h('strong', null, tok.text);
      case 'code':
        return h('code', null, tok.text);
      case 'link':
        return /^https:\/\//.test(tok.href) ? h('a', { href: tok.href, target: '_blank', rel: 'noopener noreferrer' }, tok.text) : tok.text;
      default:
        return tok.text;
    }
  });
}

function entryBody(e: ChangelogEntry): HTMLElement {
  return h('div', { class: 'ab-body' }, blocks(e.lines).map((b) => b.kind === 'para'
    ? h('p', null, inline(b.text))
    : h('ul', null, b.items.map((it) => h('li', null, inline(it.text),
      it.children.length ? h('ul', null, it.children.map((c) => h('li', null, inline(c)))) : null)))));
}

function versionHead(e: ChangelogEntry): Array<HTMLElement | string> {
  return [h('span', { class: 'ab-ver num' }, 'v' + e.version), e.date ? h('span', { class: 'ab-date num' }, e.date) : ''];
}

function news(): HTMLElement {
  const panel = h('section', { class: 'panel ab-news', 'aria-labelledby': 'ab-news-h' },
    h('h2', { id: 'ab-news-h' }, t('about.news')),
    h('p', { class: 'dim ab-lead' }, t('about.newsLead')));
  if (!data) {
    load();
    panel.append(state === 'failed'
      ? h('div', { class: 'err', role: 'alert' }, h('p', null, t('about.failed')))
      : h('p', { class: 'dim', role: 'status' }, t('about.loading')));
    return panel;
  }
  const lang = uiLang();
  const entries = data[lang]?.length ? data[lang] : data.en;
  const [first, ...rest] = entries;
  if (!first) {
    panel.append(h('p', { class: 'empty' }, t('about.empty')));
    return panel;
  }
  // The newest version is open; the earlier ones are folded so the page stays short.
  // "This version" only when the server really runs it (0.5 and 0.5.0 are the same version).
  const running = (e: ChangelogEntry): boolean => !!S.info.version && (S.info.version === e.version || S.info.version === e.version + '.0');
  panel.append(h('article', { class: 'ab-entry ab-first' },
    h('h3', null, versionHead(first), running(first) ? h('span', { class: 'ab-tag' }, t('about.current')) : null),
    entryBody(first)));
  if (rest.length) {
    panel.append(h('h3', { class: 'ab-older-h' }, t('about.older')),
      ...rest.map((e) => h('details', { class: 'ab-entry' },
        h('summary', null, versionHead(e), running(e) ? h('span', { class: 'ab-tag' }, t('about.current')) : null), entryBody(e))));
  }
  return panel;
}

export function renderAbout(): HTMLElement {
  const facts = h('dl', { class: 'ab-facts' },
    h('dt', null, t('about.version')), h('dd', { class: 'num' }, S.info.version ? 'v' + S.info.version : '—'),
    S.info.arch ? [h('dt', null, t('about.server')), h('dd', { class: 'num' }, S.info.arch)] : null,
    h('dt', null, t('about.license')), h('dd', null, t('about.licenseValue')));
  return h('div', { class: 'page page-about' },
    pageHead(t('about.title')),
    h('section', { class: 'panel ab-intro' }, h('p', null, t('about.lead')), facts),
    news());
}
