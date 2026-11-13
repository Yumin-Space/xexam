// AI file import (v1.1, section 5A, IM-1 .. IM-9): who sees the control and in which state, the file checks (twins of the
// Rust `extract::sniff` vectors), the names, and the models of the card and the confirm dialog. Pure: no DOM, no i18n calls
// (it runs under `node --test`); message keys are returned as strings and the components translate them.
import type { GenJob, GenStatus, ImportFileType } from '../types/api.ts';
import type { ControlView } from './ai-features.ts';
import { bankModeName, genView, stripName, type GenViewState } from './ai-gen.ts';

// ---------------------------------------------------------------- files

export const IMPORT_EXTS: readonly string[] = ['pdf', 'docx', 'xlsx', 'pptx', 'md', 'markdown', 'txt', 'csv'];
export const LEGACY_EXTS: readonly string[] = ['doc', 'xls', 'ppt', 'rtf', 'wps', 'et', 'dps'];
export const IMAGE_EXTS: readonly string[] = ['png', 'jpg', 'jpeg', 'gif', 'webp', 'heic', 'heif', 'bmp', 'tif', 'tiff'];

/** The `accept` attribute of the file input: the extensions and the MIME types */
export const ACCEPT = [
  ...IMPORT_EXTS.map((e) => `.${e}`),
  'application/pdf',
  'application/vnd.openxmlformats-officedocument.wordprocessingml.document',
  'application/vnd.openxmlformats-officedocument.spreadsheetml.sheet',
  'application/vnd.openxmlformats-officedocument.presentationml.presentation',
  'text/markdown', 'text/plain', 'text/csv',
].join(',');

/** Hard ceiling of the file size in MiB (the AI setting can only lower it) */
export const MAX_MB_CEILING = 20;

/** The lower-case text after the last `.` of a file name; '' when there is none */
export function extOf(name: string): string {
  const i = name.lastIndexOf('.');
  return i < 0 ? '' : name.slice(i + 1).toLowerCase();
}

/** The file type of an extension (`markdown` is `md`); null when it is not importable */
export function fileTypeOf(ext: string): ImportFileType | null {
  const e = ext.toLowerCase();
  if (e === 'markdown') return 'md';
  return (['pdf', 'docx', 'xlsx', 'pptx', 'md', 'txt', 'csv'] as const).find((x) => x === e) ?? null;
}

export type AcceptError = 'type' | 'legacy' | 'image' | 'empty' | 'tooLarge';
export type AcceptResult = { ok: true; type: ImportFileType } | { ok: false; error: AcceptError; ext: string };

/** The checks of IM-3, in the order legacy, image, type, empty, too large (the server repeats them, IJ-2) */
export function acceptFile(name: string, size: number, maxMb: number): AcceptResult {
  const ext = extOf(name);
  if (LEGACY_EXTS.includes(ext)) return { ok: false, error: 'legacy', ext };
  if (IMAGE_EXTS.includes(ext)) return { ok: false, error: 'image', ext };
  const type = fileTypeOf(ext);
  if (!type) return { ok: false, error: 'type', ext };
  if (size === 0) return { ok: false, error: 'empty', ext };
  if (size > Math.min(maxMb, MAX_MB_CEILING) * 1_048_576) return { ok: false, error: 'tooLarge', ext };
  return { ok: true, type };
}

/** "850 KB" (rounded up) / "2.3 MB" (one decimal); the same in every language */
export function fmtSize(bytes: number): string {
  if (bytes < 1_048_576) return `${Math.max(1, Math.ceil(bytes / 1024))} KB`;
  return `${(bytes / 1_048_576).toFixed(1)} MB`;
}

/** The file name without its extension; `file` when nothing is left */
export function fileStem(name: string): string {
  const i = name.lastIndexOf('.');
  const stem = (i > 0 ? name.slice(0, i) : i === 0 ? '' : name).trim();
  return stem === '' ? 'file' : stem;
}

/**
 * The name of an imported bank (SEC-11, twin of `gen::bank_mode_name` with a file stem): control / format characters dropped,
 * `/`, `\` and `:` become `_`, leading dots are trimmed, an empty stem is `file`; the whole name fits the bank-name limit.
 */
export function importName(stem: string, username: string, at: string): string {
  const cleaned = stripName(stem).replace(/[/\\:]/g, '_').trim().replace(/^\.+/, '');
  return bankModeName(cleaned === '' ? 'file' : cleaned, username, at);
}

// ---------------------------------------------------------------- who sees what (IM-1)

export type ImportReason = 'paused' | 'off' | 'down' | 'unavailable' | 'loading' | 'featureOff' | 'busy' | 'quota' | 'ownImport';

export type ImportViewState =
  | { show: false }
  | { show: true; usable: true }
  | { show: true; usable: false; reason: ImportReason; tag: boolean; key: string; params: Record<string, number> };

/**
 * `importView` of IM-1: hidden without AI+ / bank management; greyed with the tag when AI is not usable or the feature
 * `import` is off; greyed without the tag when busy or the quota is used up; the user's own active import (or a client
 * upload in progress) replaces the button by the status link (`ownImport`). `view` = `aiControlView('import', true, ctx)`.
 */
export function importView(view: ControlView, gen: GenStatus | null, uploading: boolean): ImportViewState {
  // the generation view carries every rule except the scope counts, so reuse it with counts that always pass
  const g: GenViewState = genView(view, gen, { mode: 'bank', have: Number.MAX_SAFE_INTEGER });
  if (!g.show) return { show: false };
  if (g.usable) {
    if (uploading) return { show: true, usable: false, reason: 'ownImport', tag: false, key: 'aigen.imp.busyShort', params: {} };
    return { show: true, usable: true };
  }
  if (g.reason === 'busy') {
    if (gen?.job?.source === 'file' || uploading) return { show: true, usable: false, reason: 'ownImport', tag: false, key: 'aigen.imp.busyShort', params: {} };
    return { show: true, usable: false, reason: 'busy', tag: false, key: g.key, params: g.params };
  }
  const reason = g.reason as ImportReason;
  return { show: true, usable: false, reason, tag: g.tag, key: g.key, params: g.params };
}

// ---------------------------------------------------------------- the card of a file job (IM-6)

export interface UploadState {
  name: string;
  size: number;
  type: ImportFileType;
  loaded: number;
  total: number;
}

export type CardAction = 'cancelUpload' | 'cancel' | 'cancelDisabled' | 'import' | 'decline' | 'practice' | 'dismiss';
export type CardKind = 'uploading' | 'queuedRead' | 'reading' | 'readingStep' | 'confirm' | 'queuedWrite' | 'writing' | 'saving' | 'done' | 'failed' | 'expired';

export interface CardLine {
  key: string;
  params: Record<string, string | number>;
  /** The muted second line style (leave-the-page / confirm-by notes) */
  muted?: boolean;
}

export interface CardModel {
  kind: CardKind;
  titleKey: 'aigen.imp.card.title' | 'aigen.imp.card.titleDone' | 'aigen.imp.card.titleFailed' | 'aigen.imp.card.titleExpired';
  file: string;
  bar: { mode: 'determinate'; pct: number } | { mode: 'indeterminate' } | { mode: 'none' };
  lines: CardLine[];
  /** The confirm step's count labels (A-OWNER-72) */
  counts?: CountLabels;
  actions: CardAction[];
  /** The body of the cancel confirm (the title is `aigen.card.cancelAsk`) */
  cancelBodyKey?: string;
  /** A-OWNER-66: finished questions a cancel can keep (the confirm then offers "save N" / "discard all"); absent = plain confirm */
  cancelKeep?: number;
}

const pctOf = (a: number, b: number): number => (b > 0 ? Math.min(100, Math.max(0, Math.round((a / b) * 100))) : 0);

/**
 * Which lines, bar and actions the IM-6 card shows. `job` null with an `upload` = the client-only uploading state. `at`
 * formats `confirmBy` (unix seconds) as local `HH:mm`. A failed job's reason text is added by the component (`errorText`).
 */
export function cardLines(job: GenJob | null, upload: UploadState | null, at: (unix: number) => string = () => ''): CardModel | null {
  if (!job && upload) {
    const pct = pctOf(upload.loaded, upload.total);
    return {
      kind: 'uploading', titleKey: 'aigen.imp.card.title', file: upload.name, bar: { mode: 'determinate', pct },
      lines: [{ key: 'aigen.imp.card.uploading', params: { pct } }], actions: ['cancelUpload'],
    };
  }
  if (!job || job.source !== 'file') return null;
  const file = job.file?.name ?? '';
  const leave: CardLine = { key: 'aigen.card.leave', params: {}, muted: true };
  const base = { titleKey: 'aigen.imp.card.title' as const, file };
  if (job.state === 'queued' && job.phase === 'writing') {
    return { ...base, kind: 'queuedWrite', bar: { mode: 'determinate', pct: 0 }, lines: [{ key: 'aigen.card.queued', params: {} }], actions: ['cancel'], cancelBodyKey: 'aigen.imp.card.cancelBodyQueued' };
  }
  if (job.state === 'queued') {
    return { ...base, kind: 'queuedRead', bar: { mode: 'indeterminate' }, lines: [{ key: 'aigen.card.queued', params: {} }], actions: ['cancel'], cancelBodyKey: 'aigen.imp.card.cancelBodyRead' };
  }
  if (job.state === 'running') {
    if (job.phase === 'confirm') {
      const lines: CardLine[] = [
        job.mode === 'material' ? { key: 'aigen.imp.card.confirmM', params: { n: job.goal } }
          : cutOf(job) ? { key: approxOf(job) ? 'aigen.imp.card.confirmQCutApprox' : 'aigen.imp.card.confirmQCut', params: { n: job.goal, found: foundOf(job) ?? 0 } }
            : { key: 'aigen.imp.card.confirmQ', params: { n: job.goal } },
      ];
      if (job.confirmBy != null) lines.push({ key: 'aigen.imp.card.confirmBy', params: { time: at(job.confirmBy) }, muted: true });
      const counts = countLabels(job);
      return { ...base, kind: 'confirm', bar: { mode: 'none' }, lines, ...(counts ? { counts } : {}), actions: ['import', 'decline'] };
    }
    if (job.phase === 'writing') {
      const line: CardLine = job.mode === 'material'
        ? { key: 'aigen.card.writing', params: { made: job.made, goal: job.goal } }
        : { key: 'aigen.imp.card.writingQ', params: { made: job.made, goal: job.goal } };
      return { ...base, kind: 'writing', bar: { mode: 'determinate', pct: pctOf(job.made, job.goal) }, lines: [line, leave], actions: ['cancel'], cancelBodyKey: 'aigen.card.cancelBody', ...(job.made > 0 && job.mode !== 'material' ? { cancelKeep: job.made } : {}) };
    }
    if (job.phase === 'saving') {
      return { ...base, kind: 'saving', bar: { mode: 'determinate', pct: 100 }, lines: [{ key: 'aigen.card.saving', params: {} }], actions: ['cancelDisabled'] };
    }
    // reading (or an older phase-less running job)
    if (job.read && job.read.total > 0) {
      return {
        ...base, kind: 'readingStep', bar: { mode: 'determinate', pct: pctOf(job.read.done, job.read.total) },
        lines: [{ key: 'aigen.imp.card.readingStep', params: { done: job.read.done, total: job.read.total } }, leave],
        actions: ['cancel'], cancelBodyKey: 'aigen.imp.card.cancelBodyRead',
      };
    }
    return { ...base, kind: 'reading', bar: { mode: 'indeterminate' }, lines: [{ key: 'aigen.imp.card.reading', params: {} }, leave], actions: ['cancel'], cancelBodyKey: 'aigen.imp.card.cancelBodyRead' };
  }
  if (job.state === 'done') {
    const skipped = job.skipped ?? [];
    const cut = cutOf(job);
    const line: CardLine = cut
      ? { key: approxOf(job) ? 'aigen.imp.card.doneCutApprox' : 'aigen.imp.card.doneCut', params: { file, name: job.name, made: job.made, found: foundOf(job) ?? 0 } }
      : skipped.length > 0
      ? { key: 'aigen.imp.card.donePartial', params: { file, name: job.name, made: job.made, goal: job.goal, n: skipped.length, list: skipped.join(',') } }
      : { key: 'aigen.imp.card.done', params: { file, name: job.name, made: job.made } };
    return { ...base, titleKey: 'aigen.imp.card.titleDone' as const, kind: 'done', bar: { mode: 'none' }, lines: cut && skipped.length > 0 ? [line, { key: 'aigen.imp.card.skippedNote', params: { n: skipped.length, list: skipped.join(',') }, muted: true }] : [line], actions: ['practice', 'dismiss'] };
  }
  if (job.state === 'failed') {
    return { ...base, titleKey: 'aigen.imp.card.titleFailed' as const, kind: 'failed', bar: { mode: 'none' }, lines: [{ key: 'aigen.imp.card.failed', params: {} }, { key: 'aigen.card.notCounted', params: {}, muted: true }], actions: ['dismiss'] };
  }
  if (job.state === 'cancelled' && job.error === 'ai.import.expired') {
    return { ...base, titleKey: 'aigen.imp.card.titleExpired' as const, kind: 'expired', bar: { mode: 'none' }, lines: [{ key: 'aigen.imp.card.expired', params: {} }], actions: ['dismiss'] };
  }
  return null;
}

// ---------------------------------------------------------------- the confirm dialog (IM-7)

/** A-OWNER-47: a file of questions that had more than the limit, so only the first `goal` are taken */
export const cutOf = (job: GenJob): boolean => job.mode !== 'material' && foundOf(job) != null && (foundOf(job) as number) > job.goal;

/** The number of questions in the file: the service's `total` (A-OWNER-70), else the older `found`; null when unknown */
export function foundOf(job: GenJob): number | null {
  const n = job.total ?? job.found ?? null;
  return n != null && n > 0 ? n : null;
}

/** The count is an estimate ("about N") */
export const approxOf = (job: GenJob): boolean => job.approx === true;

/** A-OWNER-72 / 73: the two labels of the confirm step: the file's question count (red when over the limit) and, only then, the limit */
export interface CountLabels {
  /** `aigen.imp.cnt.file` / `aigen.imp.cnt.fileApprox` with `n` */
  fileKey: 'aigen.imp.cnt.file' | 'aigen.imp.cnt.fileApprox';
  total: number;
  /** The limit label (`aigen.imp.cnt.limit`, `n` = the number taken); null when the file fits */
  limit: number | null;
}

/** The labels of a questions-mode job at the confirm step; null for material mode or an unknown count */
export function countLabels(job: GenJob): CountLabels | null {
  if (job.mode === 'material') return null;
  const total = foundOf(job);
  if (total == null) return null;
  return { fileKey: approxOf(job) ? 'aigen.imp.cnt.fileApprox' : 'aigen.imp.cnt.file', total, limit: total > job.goal ? job.goal : null };
}

export interface ConfirmModel {
  bodyKey: 'aigen.imp.cfm.questions' | 'aigen.imp.cfm.questionsCut' | 'aigen.imp.cfm.questionsCutApprox' | 'aigen.imp.cfm.material';
  /** The count labels shown above the text (A-OWNER-72); absent in material mode */
  counts?: CountLabels;
  /** `n` drives the English `_one` form */
  n: number;
  params: Record<string, string | number>;
  /** The note's remaining jobs; null = unlimited (the note then uses `aigen.dlg.unlimited`) */
  left: number | null;
}

/** The text of the confirm dialog: questions mode or material mode (the composition comes from `status.gen.bank`) */
export function confirmText(job: GenJob, gen: GenStatus): ConfirmModel {
  const file = job.file?.name ?? '';
  if (job.mode === 'material') {
    return {
      bodyKey: 'aigen.imp.cfm.material', n: job.goal,
      params: { file, n: job.goal, single: gen.bank.single, multi: gen.bank.multi, name: job.name }, left: gen.unlimited ? null : gen.left,
    };
  }
  if (cutOf(job)) {
    return { bodyKey: approxOf(job) ? 'aigen.imp.cfm.questionsCutApprox' : 'aigen.imp.cfm.questionsCut', n: job.goal, params: { file, n: job.goal, found: foundOf(job) ?? 0, name: job.name }, counts: countLabels(job) ?? undefined, left: gen.unlimited ? null : gen.left };
  }
  const counts = countLabels(job);
  return { bodyKey: 'aigen.imp.cfm.questions', n: job.goal, params: { file, n: job.goal, name: job.name }, ...(counts ? { counts } : {}), left: gen.unlimited ? null : gen.left };
}

/** Whether a job is the one waiting for the user's confirmation */
export const awaitsConfirm = (job: GenJob | null): job is GenJob =>
  job != null && job.source === 'file' && job.state === 'running' && job.phase === 'confirm';
