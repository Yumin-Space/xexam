// The last line of every page: [GitHub icon] GitHub | Xexam <version>, small and muted, centered. Both are links that open in a
// new tab; the version is the running one from /api/info (as the About page shows it), never a literal.
import { GITHUB_URL, LINK_REL, RELEASES_URL } from '../domain/github-links.ts';
import { t } from '../i18n/index.ts';
import { S } from '../state/store.ts';
import { h } from '../utils/dom.ts';
import { miniIcon } from './mini-icons.ts';

export function githubLine(): HTMLElement {
  return h('p', { class: 'gh-line' },
    h('a', { class: 'gh-link', href: GITHUB_URL, target: '_blank', rel: LINK_REL }, miniIcon('github', 'gh-ic'), t('footer.github')),
    S.info.version ? [h('span', { class: 'gh-sep', 'aria-hidden': 'true' }, '|'),
      h('a', { class: 'gh-link', href: RELEASES_URL, target: '_blank', rel: LINK_REL }, t('footer.release', { version: S.info.version }))] : null);
}
