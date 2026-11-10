// Catalog helper: each area of the UI has one catalog file with the same keys in English, Chinese and Japanese.
// TypeScript enforces that zh and ja define every English key. Keys ending in `_one` are English singular forms
// (picked by tn() when n === 1) and are optional in zh / ja, which have no plural forms.
import type { UiLang } from '../types/api.ts';

export type Dict = Record<string, string>;

type RequiredKeys<E> = { [K in keyof E as K extends `${string}_one` ? never : K]: string };
type OptionalKeys<E> = { [K in keyof E as K extends `${string}_one` ? K : never]?: string };
export type Translation<E> = RequiredKeys<E> & OptionalKeys<E>;

export interface Catalog<E extends Dict> {
  en: E;
  zh: Translation<E>;
  ja: Translation<E>;
}

export function defineCatalog<const E extends Dict>(c: Catalog<E>): Catalog<E> {
  return c;
}

export const UI_LANGS: readonly UiLang[] = ['en', 'zh', 'ja'];

export function isUiLang(v: unknown): v is UiLang {
  return v === 'en' || v === 'zh' || v === 'ja';
}
