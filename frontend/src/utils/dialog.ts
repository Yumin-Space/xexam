import { t } from '../i18n/index.ts';
import { h } from './dom.ts';

export interface AskOptions {
  title: string;
  body: string;
  ok?: string;
  cancel?: string;
  danger?: boolean;
}

export interface AskCheckOptions extends AskOptions {
  /** A checkbox shown under the text (for example "also delete the study records") */
  check: { label: string; hint?: string; checked?: boolean };
}

export interface AskCheckResult {
  ok: boolean;
  checked: boolean;
}

function open(opts: AskOptions, extra: HTMLElement | null, onClose: (ok: boolean) => void): void {
  const { title, body, ok = t('common.ok'), cancel = t('common.cancel'), danger = false } = opts;
  const d = h('dialog', { 'aria-labelledby': 'dlg-title' },
    h('h3', { id: 'dlg-title' }, title),
    h('p', null, body),
    extra,
    h('div', { class: 'acts' },
      h('button', { class: 'btn', type: 'button', onclick: () => d.close('cancel') }, cancel),
      h('button', { class: 'btn ' + (danger ? 'danger' : 'primary'), type: 'button', onclick: () => d.close('ok') }, ok)));
  d.addEventListener('close', () => {
    d.remove();
    onClose(d.returnValue === 'ok');
  });
  document.body.append(d);
  d.showModal();
}

const hasDialog = (): boolean => typeof (document.createElement('dialog') as HTMLDialogElement).showModal === 'function';

/** Confirmation dialog. Falls back to the system confirm() when <dialog> is unavailable (Safari before iOS 15.4). */
export function ask(opts: AskOptions): Promise<boolean> {
  return new Promise((resolve) => {
    if (!hasDialog()) {
      resolve(window.confirm(opts.title + '\n' + opts.body));
      return;
    }
    open(opts, null, resolve);
  });
}

/** Confirmation dialog with one checkbox; resolves with the button and the checkbox state. */
export function askCheck(opts: AskCheckOptions): Promise<AskCheckResult> {
  return new Promise((resolve) => {
    if (!hasDialog()) {
      const okd = window.confirm(opts.title + '\n' + opts.body);
      const checked = okd && window.confirm(opts.check.label + (opts.check.hint ? '\n' + opts.check.hint : ''));
      resolve({ ok: okd, checked });
      return;
    }
    const box = h('input', { type: 'checkbox', checked: !!opts.check.checked });
    const extra = h('label', { class: 'dlg-check' }, box,
      h('span', null, h('span', { class: 'dlg-check-l' }, opts.check.label), opts.check.hint ? h('span', { class: 'dlg-check-h' }, opts.check.hint) : null));
    open(opts, extra, (ok) => resolve({ ok, checked: ok && box.checked }));
  });
}
