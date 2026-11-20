// Bank download (v1.6): the request path, the file name, and what the format dialog offers. Pure, no DOM; MsgKey is a type
// import only, so this file runs under `node --test` as it is.
import type { MsgKey } from '../i18n/index.ts';

export type DownloadFormat = 'xlsx' | 'docx';
export const DOWNLOAD_FORMATS: readonly DownloadFormat[] = ['xlsx', 'docx'];

/** v1.9: the languages a download can hold */
export type DownloadLang = 'zh' | 'en' | 'ja';

/** The checkboxes of the dialog, in UI order: 中文, English, 日本語 */
export const DOWNLOAD_LANGS: readonly DownloadLang[] = ['zh', 'en', 'ja'];

/** The checkbox labels: the languages' own names, never translated */
export const LANG_LABEL: Record<DownloadLang, string> = { zh: '中文', en: 'English', ja: '日本語' };

/** The ticked languages in the server's order (English, Chinese, Japanese), without duplicates; used by the query and the file name */
export function canonicalLangs(langs: readonly DownloadLang[]): DownloadLang[] {
  return (['en', 'zh', 'ja'] as const).filter((l) => langs.includes(l));
}

/** All three languages (or none given) = the file of v1.8 */
const isAll = (langs: readonly DownloadLang[] | undefined): boolean => langs === undefined || canonicalLangs(langs).length === 3;

/**
 * `/api/banks/download?id=<id>&format=<format>[&langs=en,zh]`. Undefined or all three languages leaves `langs` out (the
 * server then holds all three); a subset is written in canonical order with a literal comma.
 */
export function downloadPath(bankId: number, format: DownloadFormat, langs?: readonly DownloadLang[]): string {
  const base = `/api/banks/download?id=${encodeURIComponent(String(bankId))}&format=${format}`;
  return isAll(langs) ? base : `${base}&langs=${canonicalLangs(langs ?? []).join(',')}`;
}

/** The file name of a Content-Disposition value: `filename*=UTF-8''…` (percent-decoded) first, then `filename="…"` or
 *  bare `filename=…`; null when there is none or it cannot be decoded. */
export function filenameFromDisposition(header: string | null): string | null {
  if (!header) return null;
  const star = /filename\*\s*=\s*UTF-8''([^;\s]+)/i.exec(header);
  if (star?.[1]) {
    try {
      const v = decodeURIComponent(star[1]);
      if (v) return v;
    } catch {
      // malformed escape: fall through to the plain filename
    }
  }
  const quoted = /(?:^|[;\s])filename\s*=\s*"((?:[^"\\]|\\.)*)"/i.exec(header);
  if (quoted?.[1]) return quoted[1].replace(/\\(.)/g, '$1') || null;
  const bare = /(?:^|[;\s])filename\s*=\s*([^;\s"]+)/i.exec(header);
  return bare?.[1] ?? null;
}

/**
 * The same rule as the backend `file_stem` + extension (fallback when the header is missing). A subset of the languages adds
 * `_<codes>` (codes in en, zh, ja order joined by `-`, for example `Garden_en-zh.docx`); all three add nothing.
 */
export function fallbackFileName(bankName: string, format: DownloadFormat, langs?: readonly DownloadLang[]): string {
  const cleaned = Array.from(bankName)
    .map((c) => (/[\u0000-\u001f\u007f-\u009f]/.test(c) || '/\\:*?"<>|'.includes(c) ? '_' : c))
    .slice(0, 120)
    .join('');
  const stem = cleaned.replace(/^[\s.]+|[\s.]+$/g, '');
  const suffix = isAll(langs) ? '' : `_${canonicalLangs(langs ?? []).join('-')}`;
  return `${stem || 'bank'}${suffix}.${format}`;
}

/** What the format dialog shows, in order: Excel first, then Word. */
export interface FormatChoice { format: DownloadFormat; icon: 'excel' | 'word'; label: MsgKey; hint: MsgKey; aria: MsgKey }

export const FORMAT_CHOICES: readonly FormatChoice[] = [
  { format: 'xlsx', icon: 'excel', label: 'banks.dl.excel', hint: 'banks.dl.excelHint', aria: 'banks.dl.excelAria' },
  { format: 'docx', icon: 'word', label: 'banks.dl.word', hint: 'banks.dl.wordHint', aria: 'banks.dl.wordAria' },
];
