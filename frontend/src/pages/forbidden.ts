// The "Not allowed" page (v0.6.5): shown inside the shell instead of a page the signed-in user may not use, for example
// #/banks opened directly (a bookmark, a link from someone else) by a user without the bank-manager permission. It says why
// and offers the way back to Home, instead of a silent redirect that would look like a broken link.
import '../styles/forbidden.css';
import { icon } from '../components/shell.ts';
import { t, type MsgKey } from '../i18n/index.ts';
import { navigate } from '../state/actions.ts';
import { h } from '../utils/dom.ts';

/** A closed padlock */
const ICON_LOCK = [{ d: 'M6.5 11h11v8.5h-11z' }, { d: 'M8.8 11V8.2a3.2 3.2 0 0 1 6.4 0V11' }, { d: 'M12 14.6v1.8' }];

/** `reason`: why this page is not available (one sentence that also says whom to ask) */
export function renderForbidden(reason: MsgKey): HTMLElement {
  return h('div', { class: 'page page-forbidden' },
    h('section', { class: 'panel forbidden', 'aria-labelledby': 'forbidden-h' },
      icon(ICON_LOCK, 'ico forbidden-ico'),
      h('h1', { id: 'forbidden-h' }, t('forbidden.title')),
      h('p', null, t(reason)),
      h('div', { class: 'acts' },
        h('a', {
          class: 'btn primary', href: '#/', id: 'forbidden-home',
          onclick: (e: MouseEvent) => {
            if (e.ctrlKey || e.metaKey || e.shiftKey || e.button !== 0) return;
            e.preventDefault();
            navigate({ name: 'home' });
          },
        }, t('forbidden.home')))));
}
