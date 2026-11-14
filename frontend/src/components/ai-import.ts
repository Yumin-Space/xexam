// AI file import, the UI (v1.1, section 5A, IM-2 .. IM-8): the card in Bank management, the notice dialog before
// the upload, the card of a file job and the confirm dialog. The decisions are the pure functions of domain/ai-import.ts;
// the state (upload, status, polling) is state/ai-gen.ts. Nothing here is drawn on #/quiz.
import '../styles/ai-import.css';
import { awaitsConfirm, acceptFile, ACCEPT, cardLines, confirmText, fmtSize, importView, type CardAction, type CardModel, type CountLabels } from '../domain/ai-import.ts';
import { jobReasonText } from '../i18n/errors.ts';
import { localeTag, t, tn, type MsgKey } from '../i18n/index.ts';
import { aiGenStatus } from '../state/ai.ts';
import { cancelJob, confirmJob, currentUpload, onConfirmDue, scrollToCard, startUpload, subscribeGen, type UploadView } from '../state/ai-gen.ts';
import type { GenJob } from '../types/api.ts';
import { NAME_MAX_BANK } from '../domain/ai-gen.ts';
import { ask, askChoice, dialogTitleId, formDialog } from '../utils/dialog.ts';
import { h } from '../utils/dom.ts';
import { toast } from '../utils/toast.ts';
import { cancelButton, dismissButton, whileSending, practiceBank, usageNote, quotaLine, reasonText, viewOf } from './ai-gen.ts';
import { iconButton, withReason } from './icon-button.ts';
import { miniIcon } from './mini-icons.ts';

const msg = (k: string): MsgKey => k as MsgKey;

const clockOf = (unix: number): string => {
  const d = new Date(unix * 1000);
  return `${String(d.getHours()).padStart(2, '0')}:${String(d.getMinutes()).padStart(2, '0')}`;
};

// ---------------------------------------------------------------- the card (IM-6)

function lineText(job: GenJob | null, l: { key: string; params: Record<string, string | number> }): string {
  const params: Record<string, string | number> = { ...l.params };
  if (l.key === 'aigen.imp.card.failed' && job) params['reason'] = jobReasonText(job);
  if (l.key === 'aigen.imp.card.done' || l.key === 'aigen.imp.card.confirmQ') return tn(msg(l.key), Number(params['made'] ?? params['n'] ?? 0), params);
  return t(msg(l.key), params);
}

/** The action buttons of a card, by the model's list */
function actionButtons(m: CardModel, job: GenJob | null, upload: UploadView | null): HTMLElement[] {
  return m.actions.flatMap((a: CardAction): HTMLElement[] => {
    switch (a) {
      case 'cancelUpload':
        return [cancelButton(t('aigen.imp.card.cancelUpload'), () => upload?.abort())];
      case 'cancel':
        return job ? [cancelButton(t('aigen.imp.card.cancel'), (btn) => void askCancelImport(job, m.cancelBodyKey ?? 'aigen.card.cancelBody', btn, m.cancelKeep))] : [];
      case 'cancelDisabled':
        return [withReason(cancelButton(t('aigen.imp.card.cancel'), () => undefined), t('aigen.err.tooLate'))];
      case 'import':
        return job ? [h('button', { type: 'button', class: 'btn primary ai gen-import', onclick: () => openConfirm(job) }, t('aigen.imp.card.import'))] : [];
      case 'decline':
        return job ? [h('button', { type: 'button', class: 'btn quiet danger-text gen-decline', onclick: () => void askDecline(job) }, t('aigen.imp.card.decline'))] : [];
      case 'practice':
        return job ? [h('button', { type: 'button', class: 'btn primary', onclick: () => practiceBank(job) }, t('aigen.card.practice'))] : [];
      case 'dismiss':
        // drawn in the card's top-right corner by fileCard (A-OWNER-33), not in the action row
        return [];
    }
  });
}

async function askCancelImport(job: GenJob, bodyKey: string, btn: HTMLButtonElement, finished?: number): Promise<void> {
  if (finished) {
    // A-OWNER-66: finished questions exist, so the answer is "save them" or "discard all"
    const r = await askChoice({
      title: t('aigen.imp.card.cancelAsk'), body: tn(msg('aigen.imp.card.keepBody'), finished),
      keep: t(msg('aigen.imp.card.keepOk'), { n: finished }), discard: t('aigen.imp.card.keepDiscard'), back: t('aigen.card.cancelKeep'),
    });
    if (!r) return;
    await whileSending(btn, () => cancelJob(job, false, r === 'keep'));
    return;
  }
  const ok = await ask({
    title: t('aigen.imp.card.cancelAsk'), body: t(msg(bodyKey)), ok: t('aigen.imp.card.cancelOk'), cancel: t('aigen.card.cancelKeep'), danger: true,
  });
  if (!ok) return;
  await whileSending(btn, () => cancelJob(job));
}

async function askDecline(job: GenJob): Promise<void> {
  const ok = await ask({
    title: t('aigen.imp.card.declineAsk'), body: t('aigen.imp.card.declineBody'), ok: t('aigen.imp.card.decline'), cancel: t('aigen.card.cancelKeep'), danger: true,
  });
  if (!ok) return;
  const problem = await cancelJob(job, true);
  if (problem) toast(problem, 4000);
}

/** A-OWNER-72 / 73: the file's question count and, when over the limit, the limit: two labels, both red with white text when over */
function countsEl(c: CountLabels | undefined): HTMLElement[] {
  if (!c) return [];
  const over = c.limit != null;
  return [h('p', { class: 'gen-counts' },
    h('span', { class: 'gen-cnt' + (over ? ' gen-cnt-red' : '') }, tn(c.fileKey, c.total, { n: c.total.toLocaleString(localeTag()) })),
    ...(over ? [h('span', { class: 'gen-cnt gen-cnt-red' }, t('aigen.imp.cnt.limit', { n: c.limit as number }))] : []))];
}

/** The card of a file job, or of the client-only upload (`job` null) */
export function fileCard(job: GenJob | null, upload: UploadView | null): HTMLElement {
  const m = cardLines(job, upload, clockOf);
  const card = h('section', { class: 'panel gen-card gen-file', 'data-gen-card': String(job?.id ?? 'upload'), tabindex: '-1' });
  if (!m) return card;
  card.setAttribute('aria-label', t(m.titleKey, { file: m.file }));
  const barEl = m.bar.mode === 'none' ? null : h('div', {
    class: 'gen-bar' + (m.bar.mode === 'indeterminate' ? ' ind' : ''), role: 'progressbar', 'aria-label': t('aigen.card.progressLabel'),
    'aria-valuemin': '0', 'aria-valuemax': '100', 'aria-valuenow': m.bar.mode === 'determinate' ? String(m.bar.pct) : null,
  }, h('div', { class: 'gen-fill', style: m.bar.mode === 'determinate' ? `width:${m.bar.pct}%` : '' }));
  const failed = m.kind === 'failed';
  const acts = actionButtons(m, job, upload);
  card.append(
    ...(job && m.actions.includes('dismiss') ? [dismissButton(job)] : []),
    h('h3', { class: 'gen-title gen-file-title', title: m.file }, t(m.titleKey, { file: m.file })),
    ...(barEl ? [barEl] : []),
    ...countsEl(m.counts),
    ...m.lines.map((l, i) => h('p', { class: (l.muted ? 'dim gen-note' : 'gen-line') + (failed && i === 0 ? ' gen-failed' : ''), ...(i === 0 ? { 'aria-live': 'polite' } : {}) }, lineText(job, l))),
    ...(job ? usageNote(job) : []),
    ...(acts.length ? [h('div', { class: 'acts gen-acts' + (m.kind === 'confirm' ? ' gen-confirm-acts' : '') }, ...acts)] : []));
  return card;
}

// ---------------------------------------------------------------- the confirm dialog (IM-7)

/**
 * The confirm dialog: the recognised result, the required bank name (A-OWNER-57: filled in advance with the suggested name,
 * the import button stays off until the trimmed name is not empty) and the two choices. Also opened by the card's button.
 */
function openConfirm(job: GenJob): void {
  const gen = aiGenStatus();
  if (!gen || !awaitsConfirm(job) || document.querySelector('dialog.gen-cfm[open]')) return;
  const m = confirmText(job, gen);
  const titleId = dialogTitleId();
  const note = m.left == null ? t('aigen.dlg.unlimited') : t('aigen.imp.cfm.note', { left: m.left });
  const nameId = `gen-cfm-name-${job.id}`;
  const needId = `${nameId}-need`;
  const name = h('input', {
    type: 'text', id: nameId, class: 'gen-cfm-name', value: job.name, maxlength: String(NAME_MAX_BANK), autocomplete: 'off', spellcheck: 'false',
    required: '', 'aria-describedby': needId, placeholder: t('aigen.imp.cfm.namePh'),
  });
  const need = h('p', { class: 'fld-hint gen-cfm-need', id: needId });
  const err = h('p', { class: 'err gen-cfm-err', role: 'alert', hidden: '' });
  const go = h('button', { type: 'button', class: 'btn primary ai', onclick: () => void submit() }, t('aigen.imp.card.import'));
  let busy = false;
  const sync = (): void => {
    const v = name.value.trim();
    const bad = v === '' ? t('aigen.imp.cfm.nameNeed') : [...v].length > NAME_MAX_BANK ? t('aigen.dlg.nameLong', { max: NAME_MAX_BANK }) : '';
    need.textContent = bad;
    name.setAttribute('aria-invalid', bad ? 'true' : 'false');
    const off = bad !== '' || busy;
    go.disabled = off;
    go.setAttribute('aria-disabled', off ? 'true' : 'false');
    err.hidden = true;
  };
  async function submit(): Promise<void> {
    if (go.disabled) return;
    busy = true;
    sync();
    const problem = await confirmJob(job, name.value.trim());
    busy = false;
    if (problem == null) {
      d.close('ok');
      return;
    }
    sync();
    err.textContent = problem;
    err.hidden = false;
    name.focus();
  }
  name.addEventListener('input', sync);
  const d = h('dialog', { class: 'dlg dlg-wide gen-cfm', 'aria-labelledby': titleId },
    h('form', { class: 'dlg-form', method: 'dialog', onsubmit: (e: Event) => { e.preventDefault(); void submit(); } },
      h('h3', { id: titleId, tabindex: '-1' }, t('aigen.imp.cfm.title')),
      ...countsEl(m.counts),
      h('p', null, tn(m.bodyKey, m.n, m.params)),
      h('div', { class: 'fld' }, h('label', { class: 'fld-l', for: nameId }, t('aigen.imp.cfm.nameLabel')), name, need),
      err,
      h('p', { class: 'dim' }, note),
      h('div', { class: 'acts' },
        h('button', { type: 'button', class: 'btn quiet danger-text', onclick: () => { d.close('decline'); } }, t('aigen.imp.card.decline')),
        go)));
  d.addEventListener('close', () => {
    const r = d.returnValue;
    d.remove();
    if (r === 'decline') void cancelJob(job, true).then((p) => { if (p) toast(p, 4000); });
  });
  sync();
  document.body.append(d);
  if (typeof d.showModal === 'function') d.showModal();
  else d.setAttribute('open', '');
}

onConfirmDue(openConfirm);

// ---------------------------------------------------------------- the card in Bank management (IM-2 .. IM-4)

/** The notice before the upload (IM-4); resolves true when the user starts it */
function noticeDialog(file: File): Promise<boolean> {
  const gen = aiGenStatus();
  if (!gen) return Promise.resolve(false);
  const max = gen.import?.maxQuestions ?? 50;
  const n = gen.bank.single + gen.bank.multi;
  return formDialog({
    title: t('aigen.imp.dlg.title'),
    content: h('div', { class: 'gen-form' },
      h('p', { class: 'gen-file-line' }, t('aigen.imp.dlg.file', { name: file.name, size: fmtSize(file.size) })),
      h('p', null, t('aigen.imp.dlg.body', { max, n })),
      h('p', { class: 'gen-privacy' }, miniIcon('info', 'gen-ic'), h('span', null, t('aigen.imp.dlg.privacy'))),
      h('p', { class: 'gen-quota' }, quotaLine(gen, 'aigen.imp.dlg.quota'))),
    ok: t('aigen.imp.dlg.start'), busy: t('aigen.imp.dlg.start'), cancel: t('aigen.dlg.cancel'),
    submit: async () => null,
  });
}

/**
 * The "AI 导入" card of Bank management (A-OWNER-35): its own card under the Excel one with the same layout (heading, the
 * accepted types / size / question limit, the AI+ orange button and "or drop it here", the notice that the file goes to the
 * AI service). null when hidden. Its refusals show in its own error box. The card redraws itself when the job state changes
 * (the "AI 导入中…" link, busy, quota).
 */
export function importCard(): HTMLElement | null {
  const host = h('section', { class: 'panel bk-import bk-drop bk-aicard', id: 'bk-aiimport', 'aria-labelledby': 'bk-aiimport-h' });
  const input = h('input', { type: 'file', accept: ACCEPT, hidden: true, id: 'bk-aifile' });
  let usable = false;
  let errText: string | null = null;
  const errBox = h('div', { class: 'err', role: 'alert' });
  const drawErr = (): void => {
    errBox.hidden = errText == null;
    errBox.replaceChildren(...(errText == null ? [] : [h('p', null, errText)]));
  };
  const setError = (text: string | null): void => {
    errText = text;
    drawErr();
  };
  const pick = async (file: File): Promise<void> => {
    const gen = aiGenStatus();
    const a = acceptFile(file.name, file.size, gen?.import?.maxMb ?? 20);
    if (!a.ok) {
      const key: Record<string, string> = { type: 'aigen.imp.err.type', legacy: 'aigen.imp.err.legacy', image: 'aigen.imp.err.image', empty: 'aigen.imp.err.empty', tooLarge: 'aigen.imp.err.tooLarge' };
      setError(t(msg(key[a.error] ?? 'aigen.imp.err.type'), { ext: a.ext ? `.${a.ext}` : '', types: t('aigen.imp.types'), mb: gen?.import?.maxMb ?? 20 }));
      return;
    }
    setError(null);
    if (!(await noticeDialog(file))) return;
    const r = await startUpload(file, a.type);
    if (r != null && r !== 'aborted') setError(r);
  };
  input.addEventListener('change', () => {
    const f = input.files?.[0];
    input.value = '';
    if (f) void pick(f);
  });
  // a dropped file starts the same way as a chosen one, when the button is usable
  host.addEventListener('dragover', (e) => {
    e.preventDefault();
    if (usable) host.classList.add('over');
  });
  host.addEventListener('dragleave', (e) => {
    if (!(e.relatedTarget instanceof Node && host.contains(e.relatedTarget))) host.classList.remove('over');
  });
  host.addEventListener('drop', (e) => {
    e.preventDefault();
    host.classList.remove('over');
    const f = e.dataTransfer?.files[0];
    if (usable && f) void pick(f);
  });
  const draw = (): void => {
    const gen = aiGenStatus();
    const st = importView(viewOf('import'), gen, currentUpload() != null);
    if (!st.show) {
      host.replaceChildren();
      host.hidden = true;
      return;
    }
    host.hidden = false;
    usable = st.usable;
    const n = gen ? gen.bank.single + gen.bank.multi : 20;
    const max = gen?.import?.maxQuestions ?? 50;
    const note = h('p', { class: 'bk-hint gen-impnote', id: 'bk-aiimport-note' }, t('aigen.imp.note', { types: t('aigen.imp.types'), mb: gen?.import?.maxMb ?? 20, max, n }));
    let btn: HTMLElement;
    let why: string | null = null;
    if (st.usable) {
      btn = iconButton({ icon: 'aiImport', label: t('aigen.imp.actionAria'), tone: 'ai', showLabel: true, onClick: () => input.click() });
      btn.querySelector('.ibtn-lbl')?.replaceChildren(t('aigen.imp.action'));
      btn.setAttribute('aria-label', t('aigen.imp.actionAria'));
    } else if (st.reason === 'ownImport') {
      btn = h('button', { type: 'button', class: 'btn quiet gen-impbusy', onclick: () => scrollToCard() }, miniIcon('aiImport', 'gen-ic'), t('aigen.imp.busyShort'));
    } else {
      const b = iconButton({ icon: 'aiImport', label: t('aigen.imp.actionAria'), tone: 'ai', showLabel: true });
      b.querySelector('.ibtn-lbl')?.replaceChildren(t('aigen.imp.action'));
      b.setAttribute('aria-label', t('aigen.imp.actionAria'));
      if (st.tag) b.append(h('span', { class: 'gen-na', 'aria-hidden': 'true' }, t('aiplus.na')));
      // the real button stays in the reason wrapper (same box as the Excel button, greyed); the reason is only in its tip (A-OWNER-65c)
      b.classList.add('gen-impbtn');
      why = reasonText({ show: true, usable: false, reason: 'busy', tag: st.tag, key: st.key, params: st.params, skippedNote: false });
      btn = withReason(b, why, { tapTip: true });
      btn.classList.add('gen-impwrap');
    }
    if (st.usable || st.reason === 'ownImport') btn.classList.add('gen-impbtn');
    drawErr();
    host.replaceChildren(
      h('div', { class: 'bk-head' }, h('h2', { id: 'bk-aiimport-h' }, t('aigen.imp.group'))),
      note,
      h('div', { class: 'bk-droprow bk-aidrop', role: 'group', 'aria-label': t('banks.drop.label') },
        btn,
        st.usable ? h('span', { class: 'bk-or' }, t('banks.drop.or')) : null,
        input),
      h('p', { class: 'bk-hint gen-imppriv' }, miniIcon('info', 'gen-ic'), t('aigen.imp.privacy')),
      errBox);
  };
  draw();
  subscribeGen(host, draw);
  return host.hidden ? null : host;
}
