import { t } from '../i18n/index.ts';
import { h, type Child } from './dom.ts';

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

let titleSeq = 0;

/**
 * A new id for a dialog's title (v0.8): every dialog gets its own, so a confirmation opened on top of another dialog (AI
 * settings' viewer) is named by its own title, not by the first element with a shared id.
 */
export function dialogTitleId(): string {
  titleSeq += 1;
  return `dlg-title-${titleSeq}`;
}

function open(opts: AskOptions, extra: HTMLElement | null, onClose: (ok: boolean) => void): void {
  const { title, body, ok = t('common.ok'), cancel = t('common.cancel'), danger = false } = opts;
  const titleId = dialogTitleId();
  const d = h('dialog', { 'aria-labelledby': titleId },
    h('h3', { id: titleId }, title),
    h('p', null, body),
    extra,
    h('div', { class: 'acts' },
      h('button', { class: 'btn', type: 'button', onclick: () => d.close('cancel') }, cancel),
      // A destructive confirm (delete, disable) is solid red instead of the primary colour
      h('button', { class: 'btn ' + (danger ? 'danger solid' : 'primary'), type: 'button', onclick: () => d.close('ok') }, ok)));
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

export interface FormDialogOptions {
  title: string;
  /** A short text under the title */
  body?: string;
  /** The fields (and any note) of the form */
  content: Child;
  /** The submit button, and its text while the request runs */
  ok: string;
  busy: string;
  cancel?: string;
  danger?: boolean;
  /** Runs on submit (button or Enter). Resolves with an error text to keep the dialog open and show it, or null to close it. */
  submit: () => Promise<string | null>;
  /** The element that gets the focus when the dialog opens (default: the first field) */
  focus?: HTMLElement | null;
}

/**
 * A small form in a dialog (edit a user, set a password). The dialog stays open while the request runs and when it fails
 * (the error is shown above the buttons), so nothing typed is lost. Resolves true when the form was submitted successfully,
 * false when it was cancelled (button, Esc, or closed from outside, for example when the sign-in page takes over).
 */
export function formDialog(o: FormDialogOptions): Promise<boolean> {
  return new Promise((resolve) => {
    let busy = false;
    const titleId = dialogTitleId();
    const err = h('div', { class: 'err dlg-err', role: 'alert', hidden: true });
    const okBtn = h('button', { class: 'btn ' + (o.danger ? 'danger solid' : 'primary'), type: 'submit' }, o.ok);
    const cancelBtn = h('button', { class: 'btn', type: 'button', onclick: () => d.close('cancel') }, o.cancel ?? t('common.cancel'));
    const form = h('form', {
      class: 'dlg-form', novalidate: true,
      onsubmit: (e: SubmitEvent) => {
        e.preventDefault();
        void run();
      },
    },
    h('h3', { id: titleId }, o.title),
    o.body ? h('p', null, o.body) : null,
    o.content,
    err,
    h('div', { class: 'acts' }, cancelBtn, okBtn));
    const d = h('dialog', { class: 'dlg-wide', 'aria-labelledby': titleId }, form);

    async function run(): Promise<void> {
      if (busy) return;
      busy = true;
      okBtn.disabled = true;
      cancelBtn.disabled = true;
      okBtn.textContent = o.busy;
      d.setAttribute('aria-busy', 'true');
      let problem: string | null;
      try {
        problem = await o.submit();
      } catch {
        problem = t('err.generic');
      }
      busy = false;
      if (!d.open) return;
      d.removeAttribute('aria-busy');
      okBtn.disabled = false;
      cancelBtn.disabled = false;
      okBtn.textContent = o.ok;
      if (problem == null) {
        d.close('ok');
        return;
      }
      err.replaceChildren(h('p', null, problem));
      err.hidden = false;
    }

    // Esc while the request runs: wait for it
    d.addEventListener('cancel', (e) => {
      if (busy) e.preventDefault();
    });
    d.addEventListener('close', () => {
      d.remove();
      resolve(d.returnValue === 'ok');
    });
    document.body.append(d);
    if (typeof d.showModal === 'function') d.showModal();
    else d.setAttribute('open', '');
    const first = o.focus ?? form.querySelector<HTMLElement>('input:not([type="hidden"]):not(:disabled), select, textarea');
    first?.focus();
  });
}
