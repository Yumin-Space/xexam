import { h } from './dom.ts';

export interface AskOptions {
  title: string;
  body: string;
  ok?: string;
  cancel?: string;
  danger?: boolean;
}

/** Confirmation dialog. Falls back to the system confirm() when <dialog> is unavailable (Safari before iOS 15.4). */
export function ask({ title, body, ok = '确定', cancel = '取消', danger = false }: AskOptions): Promise<boolean> {
  return new Promise((resolve) => {
    const d = h('dialog', null,
      h('h3', null, title),
      h('p', null, body),
      h('div', { class: 'acts' },
        h('button', { class: 'btn', type: 'button', onclick: () => d.close('cancel') }, cancel),
        h('button', { class: 'btn ' + (danger ? 'danger' : 'primary'), type: 'button', onclick: () => d.close('ok') }, ok)));
    if (typeof d.showModal !== 'function') {
      resolve(window.confirm(title + '\n' + body));
      return;
    }
    d.addEventListener('close', () => {
      d.remove();
      resolve(d.returnValue === 'ok');
    });
    document.body.append(d);
    d.showModal();
  });
}
