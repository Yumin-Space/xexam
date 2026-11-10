// Question banks page: import an Excel workbook into a new bank or an existing one (upload -> preview, which never writes ->
// confirm / cancel), list / inspect / update / delete the banks, the import history, and the Excel format guide.
// Laid out as compactly as the home page: small pill choices in a heading row, one-line rows, foldable bank cards.
// Page-local state (import target, pending preview, busy flags, open panels) lives in this module; shared state only through actions.
import '../styles/banks.css';
import { deleteBank, fetchBanks } from '../api/bank.ts';
import { ApiError } from '../api/client.ts';
import { cancelImport, confirmImport, fetchImports, uploadWorkbook } from '../api/import.ts';
import { pageHead } from '../components/common.ts';
import { icon } from '../components/shell.ts';
import { errorText, LocalError, serverText } from '../i18n/errors.ts';
import { t, tn, type MsgKey } from '../i18n/index.ts';
import { applyBankDeleted, navigate, onRouteChange, onSignedIn, refreshLibrary, render } from '../state/actions.ts';
import { bankById, S } from '../state/store.ts';
import type { BankSummary, ImportPlan, ImportPreview, ImportRecord, ImportTarget, SheetPlan } from '../types/api.ts';
import { ask, askCheck } from '../utils/dialog.ts';
import { h, type Child } from '../utils/dom.ts';
import { toast } from '../utils/toast.ts';

/** Upload limit of the backend (config::MAX_UPLOAD); checked here too so a wrong, huge file is not sent at all */
const MAX_MB = 20;
const MAX_BYTES = MAX_MB * 1024 * 1024;
/** How many imports the history shows */
const HISTORY_SHOWN = 8;
const XLSX_ACCEPT = '.xlsx,application/vnd.openxmlformats-officedocument.spreadsheetml.sheet';

/** Fold / unfold, "new bank" (a plus), "update a bank" (two turning arrows) */
const ICON_CHEVRON = [{ d: 'M6.5 9.5 12 15l5.5-5.5' }];
const ICON_NEW = [{ d: 'M12 5.5v13' }, { d: 'M5.5 12h13' }];
const ICON_UPDATE = [{ d: 'M19.5 11A7.6 7.6 0 0 0 5.6 7.4' }, { d: 'M5 3.8v3.9h3.9' }, { d: 'M4.5 13a7.6 7.6 0 0 0 13.9 3.6' }, { d: 'M19 20.2v-3.9h-3.9' }];

type Busy = 'upload' | 'confirm' | 'cancel' | 'delete' | null;

/** The preview waiting for confirm / cancel (the server keeps exactly one) */
interface Pending {
  pv: ImportPreview;
  target: ImportTarget;
  /** Kept so "update that bank instead" can upload the same file again */
  file: File;
}

/** The last successful import (success box) */
interface Done {
  isNew: boolean;
  bankId: number | null;
  name: string;
  file: string;
  plan: ImportPlan;
  total: number;
}

// ---------------------------------------------------------------- page-local state

let mode: 'new' | 'update' = 'new';
let updateId: number | null = null;
let busy: Busy = null;
let deletingId: number | null = null;
/** A delete dialog is being prepared (current counts are fetched) or is open: a second click must not open another one */
let asking = false;
/** Error of the import panel (ApiError or LocalError) */
let importError: unknown = null;
/** Error of the bank list (a failed delete) */
let listError: unknown = null;
let pending: Pending | null = null;
let done: Done | null = null;

let imports: ImportRecord[] | null = null;
let importsError: unknown = null;
let importsLoading = false;
/** Fetch the import history the next time the page renders */
let importsWanted = true;
let importsSeq = 0;

/** Banks whose worksheet list is unfolded, and preview sheets whose unreadable rows are unfolded (kept across re-renders) */
const openSheets = new Set<number>();
const openSkips = new Set<string>();
/** The Excel format guide is folded until the user opens it (kept across re-renders) */
let guideOpen = false;

/** Scroll / focus to apply once the next render is in the document */
let after: { scroll?: string; focus?: string } | null = null;

// The upload preview never writes, but the server keeps it until confirm / cancel: drop it when the user leaves the page.
onRouteChange((from, to) => {
  if (to.name === 'banks' && from.name !== 'banks') importsWanted = true;
  if (from.name === 'banks' && to.name !== 'banks') {
    if (pending && busy !== 'confirm') {
      pending = null;
      void cancelImport().catch(() => {});
    }
    importError = null;
    listError = null;
    done = null;
    openSkips.clear();
  }
});

// Signed in again (the session had ended): read the import history afresh, forget errors of the lost session
onSignedIn(() => {
  importsWanted = true;
  importsError = null;
  importError = null;
  listError = null;
});

// A file dropped next to the drop zone would make the browser open (download) it and leave the app: swallow it on this page.
const draggingFiles = (e: DragEvent): boolean => Array.from(e.dataTransfer?.types ?? []).includes('Files');
document.addEventListener('dragover', (e) => {
  if (S.route.name !== 'banks' || !draggingFiles(e)) return;
  e.preventDefault();
  const inZone = e.target instanceof Element && e.target.closest('.bk-drop') !== null && busy === null && pending === null;
  if (e.dataTransfer) e.dataTransfer.dropEffect = inZone ? 'copy' : 'none';
});
document.addEventListener('drop', (e) => {
  if (S.route.name === 'banks' && draggingFiles(e)) e.preventDefault();
});

// ---------------------------------------------------------------- helpers

const sameFileName = (a: string, b: string): boolean => a !== '' && a.toLowerCase() === b.toLowerCase();
const stripXlsx = (name: string): string => name.replace(/\.xlsx$/i, '') || name;
const onBanksPage = (): boolean => S.route.name === 'banks';

/** Joins sentences the way the UI language does (a space in English, nothing in Chinese / Japanese) */
function sentences(...parts: Array<string | null | false | undefined>): string {
  const kept = parts.filter((p): p is string => !!p);
  return kept.reduce((a, b) => t('banks.join2', { a, b }));
}

/** Joins the parts that are present with `sep`. Text parts are kept on one line each, so a wrapped line breaks between parts. */
function joined(sep: string, parts: Array<Child | null | false | undefined>): Child[] {
  const out: Child[] = [];
  for (const p of parts) {
    if (p == null || p === false || p === '') continue;
    if (out.length) out.push(sep);
    out.push(typeof p === 'string' ? h('span', { class: 'bk-nw' }, p) : p);
  }
  return out;
}

/** Short facts in a row: "3 usable questions, 1 updated" */
const facts = (...parts: Array<string | null | false | undefined>): Child[] => joined(t('banks.sep'), parts);
/** The parts of a one-line summary ("file · 7 questions · 2 sheets"), separated by a middle dot */
const dotted = (...parts: Array<Child | null | false | undefined>): Child[] => joined(t('banks.dot'), parts);

/** A row's name and its short facts, wrapping as one line of text */
const line = (name: string, sub: Child): HTMLElement =>
  h('span', { class: 'bk-line' }, h('span', { class: 'name' }, name), h('span', { class: 'sub' }, sub));

function nameOf(target: ImportTarget): string {
  return target === 'new' ? '' : bankById(target)?.name ?? t('common.unknownBank');
}

/** Keeps the import target valid when banks come and go. A vanished update target never silently becomes another bank. */
function normalizeTarget(): void {
  if (updateId != null && !bankById(updateId)) {
    updateId = null;
    if (mode === 'update') mode = 'new';
  }
  if (!S.banks.length) mode = 'new';
  if (updateId == null) updateId = S.banks[0]?.id ?? null;
}

function currentTarget(): ImportTarget | null {
  if (mode === 'new') return 'new';
  return updateId != null && bankById(updateId) ? updateId : null;
}

const smooth = (): ScrollBehavior =>
  typeof window.matchMedia === 'function' && window.matchMedia('(prefers-reduced-motion: reduce)').matches ? 'auto' : 'smooth';

function scrollAndFocus(scrollId: string | undefined, focusId: string | undefined): void {
  if (scrollId) document.getElementById(scrollId)?.scrollIntoView({ block: 'start', behavior: smooth() });
  if (focusId) document.getElementById(focusId)?.focus({ preventScroll: !!scrollId });
}

/** "Excel format guide" link of the import panel: unfold the guide, show it and put the focus on its summary */
function openGuide(): void {
  guideOpen = true;
  const d = document.getElementById('bk-guide-d');
  if (d instanceof HTMLDetailsElement) d.open = true;
  scrollAndFocus('bk-guide', 'bk-guide-s');
}

/**
 * Runs after the page is in the document (render() inserts it synchronously after renderBanks returns): applies a requested
 * scroll / focus, or else keeps the focus on the element with the same id (the whole page is rebuilt on every render,
 * including background re-renders such as the records refresh after a delete).
 */
function applyAfter(): void {
  const a = after;
  after = null;
  const old = document.activeElement;
  const keep = !a?.focus && old instanceof HTMLElement && old.id && old.closest('.page-banks') ? old.id : null;
  if (!a && !keep) return;
  queueMicrotask(() => {
    if (a) scrollAndFocus(a.scroll, a.focus);
    if (keep && !document.activeElement?.closest('.page-banks')) document.getElementById(keep)?.focus({ preventScroll: true });
  });
}

/** Rich text of the guide: parts in `backticks` are Excel content and shown as code */
function rich<K extends 'p' | 'li' | 'td'>(tag: K, text: string): HTMLElementTagNameMap[K] {
  return h(tag, null, text.split('`').map((s, i) => (i % 2 ? h('code', { class: 'bk-code' }, s) : s)));
}

// ---------------------------------------------------------------- import history

async function loadImports(): Promise<void> {
  importsWanted = false;
  importsLoading = true;
  const seq = ++importsSeq;
  try {
    const list = await fetchImports();
    if (seq !== importsSeq) return;
    imports = list;
    importsError = null;
  } catch (e) {
    if (seq !== importsSeq) return;
    importsError = e;
  } finally {
    if (seq === importsSeq) importsLoading = false;
  }
  if (!onBanksPage()) return;
  // Swap only the history panel: a full re-render would take the keyboard focus away from where the user is
  const old = document.getElementById('bk-history');
  if (old) old.replaceWith(historyPanel());
  else render();
}

// ---------------------------------------------------------------- upload -> preview -> confirm / cancel

async function uploadFile(file: File | undefined): Promise<void> {
  if (!file || busy) return;
  importError = null;
  done = null;
  pending = null;
  openSkips.clear();
  after = { scroll: 'bk-import' };
  if (!/\.xlsx$/i.test(file.name)) {
    importError = new LocalError('banks.err.notXlsx', { name: file.name });
    render();
    return;
  }
  if (file.size > MAX_BYTES) {
    importError = new LocalError('banks.err.tooLarge', { name: file.name, mb: MAX_MB });
    render();
    return;
  }
  const target = currentTarget();
  if (target == null) {
    importError = new LocalError('banks.err.targetGone');
    mode = 'new';
    render();
    return;
  }

  // Picked from the upload form: stay at the file row, whose status line now reports the progress. Coming from the preview
  // ("update that bank instead"), the form replaces the preview, so the panel top is shown as before.
  if (document.getElementById('bk-drop-t')) after = null;
  busy = 'upload';
  render();
  let pv: ImportPreview | null = null;
  try {
    pv = await uploadWorkbook(file, target);
    if (!pv || typeof pv.token !== 'string' || !pv.token) throw new ApiError('unexpected');
  } catch (e) {
    pv = null;
    importError = e;
    // 404 = the target bank was deleted elsewhere: bring the list (and the target choice) up to date
    if (e instanceof ApiError && e.status === 404) {
      refreshLibrary().then(() => {
        if (onBanksPage()) render();
      }, () => {});
    }
  }
  busy = null;
  if (!onBanksPage()) {
    // The user left while the file was being checked: do not leave a preview pending on the server.
    if (pv) void cancelImport().catch(() => {});
    importError = null;
    return;
  }
  if (pv) {
    pending = { pv, target, file };
    after = { scroll: 'bk-import', focus: 'bk-import-h' };
  } else {
    after = { scroll: 'bk-import' };
  }
  render();
}

async function confirmPending(): Promise<void> {
  const p = pending;
  if (!p || busy) return;
  const pv = p.pv;
  const isNew = p.target === 'new';
  const name = isNew ? stripXlsx(pv.filename) : nameOf(p.target);

  if (pv.removed > 0) {
    const ok = await ask({
      title: tn('banks.retire.askTitle', pv.removed),
      body: sentences(
        tn('banks.retire.askBody', pv.removed, { name }),
        pv.removed_stats ? tn('banks.retire.stats', pv.removed_stats) : null,
      ) + '\n' + t('banks.retire.askEnd', { back: t('banks.retire.cancel') }),
      ok: t('banks.retire.ok'),
      cancel: t('banks.retire.cancel'),
      danger: true,
    });
    if (!ok || pending !== p || busy) return;
  }

  busy = 'confirm';
  importError = null;
  render();
  try {
    const d = await confirmImport(pv.token);
    pending = null;
    let refreshError: unknown = null;
    try {
      await refreshLibrary();
    } catch (e) {
      refreshError = e;
    }
    const id = d.bankId ?? (p.target === 'new' ? null : p.target);
    const bank = id != null ? bankById(id) : undefined;
    const result: Done = {
      isNew,
      bankId: id,
      name: bank?.name ?? name,
      file: pv.filename,
      plan: d.result,
      total: bank?.total ?? (id != null ? d.bank.questions.filter((q) => q.bank === id).length : d.result.usable),
    };
    busy = null;
    importsWanted = true;
    if (refreshError) toast(errorText(refreshError), 6000);
    if (!onBanksPage()) {
      toast(t('banks.done.toast', { name: result.name }));
      return;
    }
    done = result;
    after = { scroll: 'bk-import', focus: 'bk-done' };
  } catch (e) {
    // The confirmation code is used up (or expired) either way
    pending = null;
    busy = null;
    importError = e instanceof ApiError && e.kind === 'http' ? e : new LocalError('banks.err.confirmFailed');
    // The import may have gone through before the connection dropped: show the real state
    try {
      await refreshLibrary();
    } catch {
      /* keep what we have */
    }
    importsWanted = true;
    if (!onBanksPage()) {
      toast(errorText(importError), 9000);
      importError = null;
      return;
    }
    after = { scroll: 'bk-import' };
  }
  render();
}

async function dropPending(): Promise<boolean> {
  busy = 'cancel';
  render();
  try {
    await cancelImport();
  } catch {
    /* fine if it already expired */
  }
  pending = null;
  busy = null;
  openSkips.clear();
  return onBanksPage();
}

async function cancelPending(): Promise<void> {
  if (!pending || busy) return;
  if (!(await dropPending())) return;
  after = { focus: 'bk-choose' };
  render();
}

/** "Update that bank instead": cancel this preview and check the same file against the existing bank. */
async function retargetPending(id: number): Promise<void> {
  const p = pending;
  if (!p || busy) return;
  if (!(await dropPending())) return;
  mode = 'update';
  updateId = id;
  await uploadFile(p.file);
}

// ---------------------------------------------------------------- bank actions

function chooseUpdate(b: BankSummary): void {
  if (pending || busy === 'upload' || busy === 'confirm' || busy === 'cancel') {
    if (pending) toast(t('banks.list.previewPending'));
    scrollAndFocus('bk-import', 'bk-import-h');
    return;
  }
  mode = 'update';
  updateId = b.id;
  importError = null;
  done = null;
  // The target choice is in the panel's heading row: the checked pill, the picked bank and the file button show together
  after = { scroll: 'bk-import', focus: 'bk-choose' };
  render();
}

/**
 * Asks before deleting a bank. Its `studied` / `records` counts change with every submitted exam and every record deletion,
 * but the bank list in the store is only re-read after imports and bank deletions: fetch the current numbers first, so the
 * dialog states the right counts and offers "also delete this bank's study records" exactly when there is something to delete.
 * Returns null when the user cancels or the bank cannot be checked (the error is then shown above the list).
 */
async function askDelete(card: BankSummary): Promise<{ b: BankSummary; withRecords: boolean } | null> {
  let b: BankSummary | undefined;
  try {
    b = (await fetchBanks()).find((x) => x.id === card.id);
  } catch (e) {
    if (!onBanksPage()) return null;
    listError = e;
    after = { focus: 'bk-list-h' };
    render();
    return null;
  }
  if (!onBanksPage()) return null;
  if (!b) {
    // Deleted elsewhere (for example in another tab): show the real list
    listError = new LocalError('banks.err.bankGone', { name: card.name });
    try {
      await refreshLibrary();
    } catch {
      /* keep what we have */
    }
    if (onBanksPage()) {
      after = { focus: 'bk-list-h' };
      render();
    }
    return null;
  }

  const title = t('banks.delete.title', { name: b.name });
  const body = tn('banks.delete.body', b.total + b.inactive);
  if (b.studied === 0 && b.records === 0) {
    const ok = await ask({ title, body: body + '\n' + t('banks.delete.noHistory'), ok: t('banks.delete.ok'), danger: true });
    return ok ? { b, withRecords: false } : null;
  }
  const r = await askCheck({
    title,
    body,
    ok: t('banks.delete.ok'),
    danger: true,
    check: {
      label: t('banks.delete.check'),
      hint: t('banks.delete.hint', { studied: tn('banks.delete.studied', b.studied), records: tn('banks.delete.records', b.records) }),
    },
  });
  return r.ok ? { b, withRecords: r.checked } : null;
}

async function removeBank(card: BankSummary): Promise<void> {
  if (busy || asking) return;
  asking = true;
  let choice: Awaited<ReturnType<typeof askDelete>>;
  try {
    choice = await askDelete(card);
  } finally {
    asking = false;
  }
  if (!choice || busy) return;
  const { b, withRecords } = choice;

  busy = 'delete';
  deletingId = b.id;
  listError = null;
  render();
  try {
    const d = await deleteBank(b.id, withRecords);
    applyBankDeleted(d);
    // The server drops a pending import that targets this bank
    if (pending && pending.target === b.id) pending = null;
    if (done && done.bankId === b.id) done = null;
    importsWanted = true;
    toast(t(withRecords ? 'banks.delete.doneWithRecords' : 'banks.delete.done', { name: b.name }));
    after = { focus: 'bk-list-h' };
  } catch (e) {
    listError = e;
    // For example "this bank no longer exists": show the real list
    try {
      await refreshLibrary();
    } catch {
      /* keep what we have */
    }
  }
  busy = null;
  deletingId = null;
  if (onBanksPage()) render();
}

// ---------------------------------------------------------------- import panel

/** The preview's counts as one line of small chips. Zero counts are left out, unless every count is zero. */
function chips(defs: Array<[n: number, key: MsgKey, cls: string]>): HTMLElement {
  const shown = defs.some(([n]) => n > 0) ? defs.filter(([n]) => n > 0) : defs;
  return h('ul', { class: 'bk-chips', 'aria-label': t('banks.preview.summary') },
    shown.map(([n, key, cls]) => h('li', { class: 'bk-chip ' + (n ? cls : 'zero') }, h('span', null, t(key)), h('b', { class: 'num' }, String(n)))));
}

/** One line per sheet; a sheet with unreadable rows is a fold whose summary is that line and whose body lists the reasons */
function sheetPlanRow(s: SheetPlan): HTMLElement {
  const skipped = s.skippedCount ?? s.skipped.length;
  const text = line(s.name, facts(
    tn('banks.sheet.usable', s.usable),
    s.added ? tn('banks.sheet.added', s.added) : null,
    s.updated ? tn('banks.sheet.updated', s.updated) : null,
    s.restored ? tn('banks.sheet.restored', s.restored) : null,
    s.removed ? tn('banks.sheet.retired', s.removed) : null));
  if (!skipped) return h('li', { class: 'bk-row' }, text);
  const d = h('details', { class: 'bk-skip', open: openSkips.has(s.name) },
    h('summary', { title: tn('banks.skip.summary', skipped) },
      text,
      h('span', { class: 'bk-skipn' }, tn('banks.sheet.unreadable', skipped)),
      icon(ICON_CHEVRON, 'ico bk-chev')),
    h('ul', null,
      s.skipped.map((k) => h('li', null, t('banks.skip.row', { row: k.row, reason: serverText(k.reason) }))),
      s.skippedMore ? h('li', { class: 'dim' }, tn('banks.skip.more', s.skippedMore)) : null));
  d.addEventListener('toggle', () => {
    if (d.open) openSkips.add(s.name);
    else openSkips.delete(s.name);
  });
  return h('li', { class: 'bk-row' }, d);
}

function previewView(p: Pending): Child[] {
  const pv = p.pv;
  const isNew = p.target === 'new';
  const name = nameOf(p.target);
  const nothing = pv.usable === 0;
  const same = !nothing && !pv.added && !pv.updated && !pv.removed && !pv.restored;
  const working = busy !== null;
  const twin = isNew ? S.banks.find((b) => sameFileName(b.filename, pv.filename)) : undefined;
  const target = typeof p.target === 'number' ? bankById(p.target) : undefined;
  const otherFile = target && target.filename && !sameFileName(target.filename, pv.filename) ? target : undefined;

  const confirmLabel = busy === 'confirm' ? t('banks.preview.importing')
    : nothing ? t('banks.preview.noneToImport')
      : same ? t('banks.preview.noChange')
        : isNew ? t('banks.preview.confirmNew') : t('banks.preview.confirmUpdate');

  return [
    h('h2', { id: 'bk-import-h', tabindex: '-1', class: 'bk-ptitle' },
      isNew ? t('banks.preview.titleNew', { file: pv.filename }) : t('banks.preview.titleUpdate', { name, file: pv.filename })),
    nothing
      ? h('div', { class: 'err', role: 'alert' },
        h('p', null, t('banks.preview.nothing')),
        h('p', null, pv.skipped ? t('banks.preview.nothingSkipped') : t('banks.preview.nothingHint')))
      : h('p', { class: 'bk-hint' }, t('banks.preview.lead')),
    chips(isNew
      ? [[pv.added, 'banks.pill.added', 'g'], [pv.skipped, 'banks.pill.unreadable', 'r']]
      : [
        [pv.added, 'banks.pill.added', 'g'], [pv.updated, 'banks.pill.updated', 'b'], [pv.restored, 'banks.pill.restored', 'b'],
        [pv.removed, 'banks.pill.retired', 'r'], [pv.unchanged, 'banks.pill.unchanged', ''], [pv.skipped, 'banks.pill.unreadable', 'r'],
      ]),
    twin ? h('div', { class: 'bk-info' },
      h('p', null, t('banks.preview.sameFile', { name: twin.name, update: t('banks.target.update') })),
      h('div', { class: 'acts' },
        h('button', { class: 'btn bk-mini', type: 'button', disabled: working, onclick: () => void retargetPending(twin.id) },
          t('banks.preview.switchToUpdate', { name: twin.name })))) : null,
    otherFile ? h('div', { class: 'bk-info' },
      h('p', null, t('banks.preview.otherFile', { name: otherFile.name, old: otherFile.filename, file: pv.filename }))) : null,
    pv.removed ? h('div', { class: 'warnbox' },
      h('p', null, sentences(
        tn('banks.retire.box', pv.removed, { name }),
        pv.removed_stats ? tn('banks.retire.stats', pv.removed_stats) : null)),
      pv.removed_samples?.length ? [
        h('p', { class: 'dim' }, t('banks.retire.samples')),
        h('ul', null, pv.removed_samples.map((s) => h('li', { lang: 'ja' }, t('banks.retire.sample', { text: s })))),
      ] : null,
      h('p', { class: 'dim' }, t('banks.retire.onlyAdd'))) : null,
    same ? h('p', { class: 'bk-hint bk-same' }, t('banks.preview.same', { name })) : null,
    // The decision comes right after the summary and the warnings; the per-sheet detail follows as optional reading
    h('div', { class: 'acts bk-acts' },
      h('button', { class: 'btn primary', type: 'button', disabled: nothing || same || working, onclick: () => void confirmPending() }, confirmLabel),
      h('button', { class: 'btn', type: 'button', disabled: working, onclick: () => void cancelPending() }, t('banks.preview.cancel'))),
    pv.sheets.length ? [
      h('h3', { class: 'bk-sub' }, t('banks.preview.sheets')),
      h('ul', { class: 'bk-rows' }, pv.sheets.map(sheetPlanRow)),
    ] : null,
  ];
}

function doneBox(d: Done): HTMLElement {
  return h('div', { class: 'okbox', role: 'status', id: 'bk-done', tabindex: '-1' },
    h('p', { class: 'ok-t' }, d.isNew
      ? t('banks.done.titleNew', { name: d.name, file: d.file })
      : t('banks.done.titleUpdate', { name: d.name, file: d.file })),
    h('p', null, sentences(
      d.isNew ? null : t('banks.done.counts', { added: d.plan.added, updated: d.plan.updated, restored: d.plan.restored, retired: d.plan.removed }),
      tn('banks.done.total', d.total),
      d.plan.skipped ? tn('banks.done.skipped', d.plan.skipped) : null)),
    h('div', { class: 'acts' },
      h('button', { class: 'btn primary', type: 'button', onclick: () => navigate({ name: 'home' }) }, t('banks.done.start')),
      h('button', {
        class: 'btn quiet', type: 'button',
        onclick: () => {
          done = null;
          after = { focus: 'bk-import-h' };
          render();
        },
      }, t('common.close'))));
}

/** The upload form: the target pills (for the heading row), the bank picker under them, the hint line and the file row */
function uploadForm(): { choice: HTMLElement; pick: HTMLElement | null; hint: HTMLElement; row: HTMLElement } {
  const hasBanks = S.banks.length > 0;
  const locked = busy !== null;
  const statusText = (): string => {
    if (busy === 'upload') return t('banks.drop.busy');
    if (mode === 'new') return t('banks.drop.titleNew');
    return t('banks.drop.titleUpdate', { name: updateId != null ? nameOf(updateId) : '' });
  };

  // Switching the target only updates these nodes (no re-render, so keyboard focus stays on the pill / select).
  // While idle the status names the target for screen readers (it also describes the file button); while a file is
  // checked it is shown and reports the progress.
  const status = h('span', { class: busy === 'upload' ? 'bk-status' : 'sr-only', id: 'bk-drop-t', role: 'status' }, statusText());
  const select = h('select', {
    id: 'bk-target', disabled: locked || !hasBanks,
    onchange: () => {
      updateId = Number(select.value);
      refresh();
    },
  }, S.banks.map((b) => h('option', { value: String(b.id) }, t('banks.target.option', { name: b.name, questions: tn('common.questions', b.total) }))));
  if (updateId != null) select.value = String(updateId);
  const pick = hasBanks ? h('div', { class: 'bk-pick', hidden: mode !== 'update' }, h('label', { for: 'bk-target' }, t('banks.target.select')), select) : null;
  const refresh = (): void => {
    if (pick) pick.hidden = mode !== 'update';
    status.textContent = statusText();
  };

  // Two small pills with radio semantics; the full explanation is the tooltip and the radio's description
  const seg = (val: 'new' | 'update', label: string, about: string, shape: typeof ICON_NEW, disabled: boolean): HTMLElement =>
    h('label', { class: 'bk-seg' + (disabled ? ' off' : ''), title: about },
      h('input', {
        type: 'radio', name: 'bk-target-mode', value: val, id: `bk-mode-${val}`, checked: mode === val, disabled: disabled || locked,
        'aria-describedby': `bk-mode-${val}-d`,
        onchange: () => {
          mode = val;
          refresh();
        },
      }),
      icon(shape, 'ico bk-seg-ico'),
      h('span', null, label));
  const updateAbout = hasBanks ? t('banks.target.updateSub') : t('banks.target.noBanks');
  const choice = h('fieldset', { class: 'bk-target', id: 'bk-target-fs' },
    h('legend', { class: 'sr-only' }, t('banks.target.legend')),
    h('div', { class: 'bk-segs' },
      seg('new', t('banks.target.new'), t('banks.target.newSub'), ICON_NEW, false),
      seg('update', t('banks.target.update'), updateAbout, ICON_UPDATE, !hasBanks)),
    h('span', { class: 'sr-only', id: 'bk-mode-new-d' }, t('banks.target.newSub')),
    h('span', { class: 'sr-only', id: 'bk-mode-update-d' }, updateAbout));

  // A plain space (not a margin) before the link: it disappears when the link wraps to the start of a line
  // The link follows the hint like a second sentence: a space in English, nothing after a full-width 。 in zh / ja
  const hint = h('p', { class: 'bk-hint' }, t('banks.import.hint'), t('banks.join2', { a: '', b: '' }),
    h('button', { class: 'linkbtn', type: 'button', onclick: openGuide }, t('banks.import.formatLink')));

  const input = h('input', {
    type: 'file', accept: XLSX_ACCEPT, hidden: true, id: 'bk-file',
    onchange: () => {
      const f = input.files?.[0];
      input.value = '';
      void uploadFile(f);
    },
  });
  const row = h('div', { class: 'bk-droprow', role: 'group', 'aria-label': t('banks.drop.label') },
    h('button', {
      class: 'btn primary', id: 'bk-choose', type: 'button', disabled: locked, 'aria-describedby': 'bk-drop-t bk-drop-note',
      onclick: () => input.click(),
    }, t('banks.drop.choose')),
    busy === 'upload' ? null : h('span', { class: 'bk-or' }, t('banks.drop.or')),
    status,
    h('span', { class: 'bk-note', id: 'bk-drop-note' }, t('banks.drop.note', { mb: MAX_MB })),
    input);

  return { choice, pick, hint, row };
}

function importPanel(): HTMLElement {
  const working = busy === 'upload' || busy === 'confirm';
  const attrs = { id: 'bk-import', 'aria-labelledby': 'bk-import-h', 'aria-busy': working ? 'true' : null };
  if (pending) return h('section', { ...attrs, class: 'panel bk-import' }, previewView(pending));

  const f = uploadForm();
  // The whole panel accepts a dropped file (.bk-drop also tells the page-wide handler that a drop is welcome here)
  const sec = h('section', { ...attrs, class: 'panel bk-import bk-drop' + (busy === 'upload' ? ' busy' : '') },
    h('div', { class: 'bk-head' }, h('h2', { id: 'bk-import-h', tabindex: '-1' }, t('banks.import.title')), f.choice),
    f.pick,
    importError ? h('div', { class: 'err', role: 'alert' }, h('p', null, errorText(importError, 'banks.err.uploadFailed'))) : null,
    done ? doneBox(done) : null,
    f.hint,
    f.row);
  sec.addEventListener('dragover', (e) => {
    e.preventDefault();
    if (!busy) sec.classList.add('over');
  });
  sec.addEventListener('dragleave', (e) => {
    if (!(e.relatedTarget instanceof Node && sec.contains(e.relatedTarget))) sec.classList.remove('over');
  });
  sec.addEventListener('drop', (e) => {
    e.preventDefault();
    sec.classList.remove('over');
    if (!busy) void uploadFile(e.dataTransfer?.files[0]);
  });
  return sec;
}

// ---------------------------------------------------------------- bank list

/** One card per bank: name and the two actions, one line of facts, and a chevron that folds / unfolds its sheet rows */
function bankCard(b: BankSummary): HTMLElement {
  const deleting = deletingId === b.id;
  const open = openSheets.has(b.id);
  const boxId = `bk-sheets-${b.id}`;
  const questions = tn('common.questions', b.total);
  const created = t('banks.list.createdAt', { at: b.createdAt });

  const box = b.sheets.length ? h('div', { class: 'bk-sheetbox', id: boxId, hidden: !open },
    h('ul', { class: 'bk-sheetrows' }, b.sheets.map((s) => h('li', { class: 'bk-row' }, line(s.name, s.usable > 0
      ? facts(tn('banks.sheet.usable', s.usable), s.inactive ? tn('banks.list.sheetRetired', s.inactive) : null)
      : t('banks.sheet.allRetired'))))),
    h('p', { class: 'bk-created num' }, created)) : null;

  const tip = (on: boolean): string => t(on ? 'banks.list.hideSheets' : 'banks.list.showSheets', { name: b.name });
  const expand = box ? h('button', {
    class: 'bk-expand', id: `bk-expand-${b.id}`, type: 'button', 'aria-expanded': String(open), 'aria-controls': boxId,
    'aria-label': tip(open), title: tip(open),
    onclick: () => {
      const now = box.hidden;
      box.hidden = !now;
      if (now) openSheets.add(b.id);
      else openSheets.delete(b.id);
      expand?.setAttribute('aria-expanded', String(now));
      expand?.setAttribute('aria-label', tip(now));
      if (expand) expand.title = tip(now);
    },
  }, icon(ICON_CHEVRON)) : null;

  return h('article', { class: 'bk-bank', 'aria-labelledby': `bk-name-${b.id}` },
    h('div', { class: 'bk-bankhead' },
      h('div', { class: 'bk-bankmain' },
        h('h3', { class: 'bk-bankname', id: `bk-name-${b.id}` }, b.name),
        h('div', { class: 'bk-bankacts' },
          h('button', {
            class: 'btn bk-mini', id: `bk-update-${b.id}`, type: 'button', disabled: busy === 'delete',
            'aria-label': t('banks.list.updateAria', { name: b.name }), title: t('banks.list.updateAria', { name: b.name }),
            onclick: () => chooseUpdate(b),
          }, t('banks.list.update')),
          h('button', {
            class: 'btn bk-mini danger', id: `bk-delete-${b.id}`, type: 'button', disabled: busy !== null,
            'aria-label': deleting ? null : t('banks.list.deleteAria', { name: b.name }), title: deleting ? null : t('banks.list.deleteAria', { name: b.name }),
            onclick: () => void removeBank(b),
          }, deleting ? t('banks.list.deleting') : t('common.delete'))),
        // Without sheet rows there is nothing to unfold, so the creation date is the tooltip of this line
        // The file name may be long and wraps anywhere; the other facts each stay on one line
        h('p', { class: 'bk-bankmeta', title: box ? null : created }, dotted(
          h('span', null, b.filename || t('common.none')),
          b.inactive ? t('banks.list.withRetired', { questions, n: b.inactive }) : questions,
          tn('common.sheets', b.sheets.length),
          h('span', { class: 'num bk-nw' }, t('banks.list.updatedAt', { at: b.updatedAt }))))),
      expand),
    box);
}

function banksPanel(): HTMLElement {
  return h('section', { class: 'panel bk-list', 'aria-labelledby': 'bk-list-h', 'aria-busy': busy === 'delete' ? 'true' : null },
    h('h2', { id: 'bk-list-h', tabindex: '-1' }, t('banks.list.title')),
    listError ? h('div', { class: 'err', role: 'alert' }, h('p', null, t('banks.delete.failed', { reason: errorText(listError) }))) : null,
    S.banks.length
      ? h('div', { class: 'bk-banks' }, S.banks.map(bankCard))
      : h('p', { class: 'empty' }, t('banks.list.empty')));
}

// ---------------------------------------------------------------- import history

function historyPanel(): HTMLElement {
  const list = imports;
  const retry = (): void => {
    importsWanted = true;
    render();
  };
  return h('section', { class: 'panel bk-history', id: 'bk-history', 'aria-labelledby': 'bk-history-h' },
    h('h2', { id: 'bk-history-h' }, t('banks.history.title')),
    importsError && !importsLoading ? h('div', { class: 'err', role: 'alert' },
      h('p', null, t('banks.history.failed', { reason: errorText(importsError) })),
      h('div', { class: 'acts bk-retry' }, h('button', { class: 'btn bk-mini', type: 'button', onclick: retry }, t('common.retry')))) : null,
    !list
      ? (importsError && !importsLoading ? null : h('p', { class: 'dim', role: 'status' }, t('common.loading')))
      : !list.length
        ? h('p', { class: 'empty' }, t('banks.history.empty'))
        : [
          // One line per import: when, which file, and only the counts that are not zero
          h('ul', { class: 'bk-hist' }, list.slice(0, HISTORY_SHOWN).map((x) => {
            const counts = facts(
              x.total ? tn('common.questions', x.total) : null,
              x.added ? tn('banks.sheet.added', x.added) : null,
              x.updated ? tn('banks.sheet.updated', x.updated) : null,
              x.restored ? tn('banks.sheet.restored', x.restored) : null,
              x.removed ? tn('banks.sheet.retired', x.removed) : null,
              x.skipped ? tn('banks.sheet.unreadable', x.skipped) : null);
            return h('li', null, dotted(
              h('span', { class: 'num bk-nw bk-hist-at' }, x.at),
              h('span', { class: 'bk-hist-file' }, x.filename),
              counts.length ? counts : null));
          })),
          list.length > HISTORY_SHOWN ? h('p', { class: 'bk-note-l' }, t('banks.history.latest', { n: HISTORY_SHOWN })) : null,
        ]);
}

// ---------------------------------------------------------------- Excel format guide

function columns(rows: Array<[string, MsgKey]>): HTMLElement {
  return h('table', { class: 'bk-cols' },
    h('thead', null, h('tr', null, h('th', { scope: 'col' }, t('banks.guide.colHead')), h('th', { scope: 'col' }, t('banks.guide.contentHead')))),
    h('tbody', null, rows.map(([col, key]) => h('tr', null, h('th', { scope: 'row' }, col), rich('td', t(key))))));
}

function example(title: MsgKey, colA: MsgKey, colC: MsgKey): HTMLElement {
  const cell = (label: MsgKey, text: MsgKey): HTMLElement =>
    h('div', null, h('p', { class: 'bk-ex-col' }, t(label)), h('pre', { class: 'bk-ex-cell' }, t(text)));
  return h('figure', { class: 'bk-ex' },
    h('figcaption', null, t(title)),
    h('div', { class: 'bk-ex-cols' }, cell('banks.guide.exColA', colA), cell('banks.guide.exColC', colC)));
}

function guidePanel(): HTMLElement {
  // Folded by default: the summary is the panel's heading row. Column letters are Excel's own (the same in every language).
  const d = h('details', { class: 'bk-guide-d', id: 'bk-guide-d', open: guideOpen },
    h('summary', { class: 'bk-guide-s', id: 'bk-guide-s' }, h('h2', { id: 'bk-guide-h' }, t('banks.guide.title')), icon(ICON_CHEVRON, 'ico bk-chev')),
    h('div', { class: 'bk-guide-body' },
      h('h3', null, t('banks.guide.basicsTitle')),
      rich('p', t('banks.guide.bank')),
      rich('p', t('banks.guide.rows')),
      h('h3', null, t('banks.guide.cols3Title')),
      columns([['A', 'banks.guide.c3a'], ['B', 'banks.guide.c3b'], ['C', 'banks.guide.c3c']]),
      h('h3', null, t('banks.guide.cols4Title')),
      columns([['A', 'banks.guide.c4a'], ['B', 'banks.guide.c3b'], ['C', 'banks.guide.c4c'], ['D', 'banks.guide.c4d']]),
      rich('p', t('banks.guide.cols4Note')),
      h('h3', null, t('banks.guide.typesTitle')),
      h('ul', { class: 'bk-points' },
        rich('li', t('banks.guide.choice')),
        rich('li', t('banks.guide.hotspot')),
        rich('li', t('banks.guide.headings'))),
      h('h3', null, t('banks.guide.exTitle')),
      example('banks.guide.exChoice', 'banks.guide.exChoiceA', 'banks.guide.exChoiceC'),
      example('banks.guide.exHotspot', 'banks.guide.exHotspotA', 'banks.guide.exHotspotC'),
      h('h3', null, t('banks.guide.importTitle')),
      h('ul', { class: 'bk-points' },
        rich('li', t('banks.guide.whole')),
        rich('li', t('banks.guide.others')),
        rich('li', t('banks.guide.identity')),
        rich('li', t('banks.guide.preview')),
        rich('li', t('banks.guide.files', { mb: MAX_MB })),
        rich('li', t('banks.guide.newOrUpdate', { new: t('banks.target.new'), update: t('banks.target.update') })))));
  d.addEventListener('toggle', () => {
    guideOpen = d.open;
  });
  return h('section', { class: 'panel bk-guide', id: 'bk-guide', 'aria-labelledby': 'bk-guide-h' }, d);
}

// ---------------------------------------------------------------- page

export function renderBanks(): HTMLElement {
  normalizeTarget();
  if (importsWanted && !importsLoading) void loadImports();
  const el = h('div', { class: 'page page-banks' },
    pageHead(t('banks.title')),
    h('div', { class: 'bk-grid' },
      h('div', { class: 'bk-col' }, importPanel(), banksPanel()),
      h('div', { class: 'bk-col' }, historyPanel(), guidePanel())));
  applyAfter();
  return el;
}
