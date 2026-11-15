// External IP-location lookup settings (v1.4): the provider presets, the form <-> request body logic, the inline checks that
// mirror ai-service's validation (the server stays the authority) and the texts of a failed test. Pure: no DOM, no network
// (tested in geo-source.test.ts). Preset formats were verified on 2026-11-15 with the public example IP 8.8.8.8; every field
// stays editable. Unverified details: ipinfo's error-object shape, ipapi.co's `key` parameter (paid plans), ip.sb's error format.
import type { MsgKey, Params } from '../i18n/index.ts';
import type { GeoDraft, GeoFailure, GeoLocation, GeoMapping, GeoPlace, GeoPresetId, GeoSettings } from '../types/api.ts';

/** The values of the settings form (the key text is kept apart: it is never part of a loaded form) */
export interface GeoForm {
  preset: GeoPresetId;
  urlTemplate: string;
  /** "Send the key in a request header": the two fields below are sent only while this is on */
  inHeader: boolean;
  headerName: string;
  headerValue: string;
  mapping: GeoMapping;
}

export const DEFAULT_HEADER_NAME = 'Authorization';
export const DEFAULT_HEADER_VALUE = 'Bearer {key}';

export const EMPTY_MAPPING: GeoMapping = {
  country: '', countryCode: '', region: '', city: '', okPath: '', okValue: '', errorPath: '', messagePath: '',
};
export const MAPPING_FIELDS = ['country', 'countryCode', 'region', 'city', 'okPath', 'okValue', 'errorPath', 'messagePath'] as const;

export interface GeoPreset {
  id: GeoPresetId;
  /** Provider name; `null` for "custom" (the page shows the translated "Custom") */
  label: string | null;
  /** `{lang}` (ip-api and ipwho.is) is replaced by the UI language's code (zh-CN / ja / en) when the preset is picked; it is not a placeholder of the server */
  urlTemplate: string;
  mapping: GeoMapping;
}

const map = (m: Partial<GeoMapping>): GeoMapping => ({ ...EMPTY_MAPPING, ...m });

export const GEO_PRESETS: readonly GeoPreset[] = [
  {
    id: 'ipinfo', label: 'ipinfo.io', urlTemplate: 'https://ipinfo.io/{ip}/json?token={key}',
    mapping: map({ countryCode: 'country', region: 'region', city: 'city', errorPath: 'error', messagePath: 'error.message' }),
  },
  {
    id: 'ipapi', label: 'ip-api.com', urlTemplate: 'http://ip-api.com/json/{ip}?lang={lang}',
    mapping: map({ country: 'country', countryCode: 'countryCode', region: 'regionName', city: 'city', okPath: 'status', okValue: 'success', messagePath: 'message' }),
  },
  {
    id: 'ipapico', label: 'ipapi.co', urlTemplate: 'https://ipapi.co/{ip}/json/?key={key}',
    mapping: map({ country: 'country_name', countryCode: 'country_code', region: 'region', city: 'city', errorPath: 'error', messagePath: 'reason' }),
  },
  {
    id: 'ipwhois', label: 'ipwho.is', urlTemplate: 'https://ipwho.is/{ip}?lang={lang}',
    mapping: map({ country: 'country', countryCode: 'country_code', region: 'region', city: 'city', okPath: 'success', okValue: 'true', messagePath: 'message' }),
  },
  {
    id: 'ipsb', label: 'ip.sb', urlTemplate: 'https://api.ip.sb/geoip/{ip}',
    mapping: map({ country: 'country', countryCode: 'country_code', region: 'region', city: 'city' }),
  },
  { id: 'custom', label: null, urlTemplate: '', mapping: EMPTY_MAPPING },
];

/** The `lang` of the ip-api preset for a UI language: zh -> zh-CN, ja -> ja, anything else en */
export function ipApiLang(lang: string): string {
  return lang === 'zh' ? 'zh-CN' : lang === 'ja' ? 'ja' : 'en';
}

/** A form with no provider chosen yet: the first preset, filled for `lang` */
export function blankForm(lang: string): GeoForm {
  return presetDraft('ipinfo', lang, null);
}

/**
 * The form after picking a preset: URL and mapping filled in, the header fields cleared. "custom" keeps the current values
 * (only the preset id changes).
 */
export function presetDraft(id: GeoPresetId, lang: string, current: GeoForm | null): GeoForm {
  const p = GEO_PRESETS.find((x) => x.id === id) ?? GEO_PRESETS[GEO_PRESETS.length - 1]!;
  if (p.id === 'custom') {
    return current
      ? { ...current, preset: 'custom', mapping: { ...current.mapping } }
      : { preset: 'custom', urlTemplate: '', inHeader: false, headerName: DEFAULT_HEADER_NAME, headerValue: DEFAULT_HEADER_VALUE, mapping: { ...EMPTY_MAPPING } };
  }
  return {
    preset: p.id, urlTemplate: p.urlTemplate.replace('{lang}', ipApiLang(lang)), inHeader: false,
    headerName: DEFAULT_HEADER_NAME, headerValue: DEFAULT_HEADER_VALUE, mapping: { ...p.mapping },
  };
}

/** The form of the saved settings (the key is never part of it) */
export function formFromSettings(s: GeoSettings): GeoForm {
  return {
    preset: s.preset, urlTemplate: s.urlTemplate, inHeader: s.keyHeaderName !== '',
    headerName: s.keyHeaderName || DEFAULT_HEADER_NAME, headerValue: s.keyHeaderValue || DEFAULT_HEADER_VALUE, mapping: { ...s.mapping },
  };
}

/** The place of a lookup as the backend would store it: a 2-letter country code (not XX / T1) wins over the country name */
export function placeLocation(p: GeoPlace): GeoLocation {
  const code = (p.countryCode ?? '').trim().toUpperCase();
  const country = /^[A-Z]{2}$/.test(code) && code !== 'XX' && code !== 'T1' ? code : (p.country ?? '').trim() || null;
  return { country, region: p.region, city: p.city };
}

/** A URL that starts with http:// (the visitor IP then travels unencrypted) */
export function isInsecureHttp(url: string): boolean {
  return /^\s*http:\/\//i.test(url);
}

/**
 * The body of test / save. `key`: a typed key is sent as it is (trimmed); an empty field with "remove key" pressed is `null`
 * (no key); an empty field otherwise leaves the property out (the server keeps the saved key). The header fields are sent
 * empty while "Send the key in a request header" is off.
 */
export function draftBody(form: GeoForm, keyInput: string, keyCleared: boolean): GeoDraft {
  const m = form.mapping;
  const body: GeoDraft = {
    preset: form.preset,
    urlTemplate: form.urlTemplate.trim(),
    keyHeaderName: form.inHeader ? form.headerName.trim() : '',
    keyHeaderValue: form.inHeader ? form.headerValue.trim() : '',
    mapping: {
      country: m.country.trim(), countryCode: m.countryCode.trim(), region: m.region.trim(), city: m.city.trim(),
      okPath: m.okPath.trim(), okValue: m.okValue.trim(), errorPath: m.errorPath.trim(), messagePath: m.messagePath.trim(),
    },
  };
  const typed = keyInput.trim();
  if (typed !== '') body.key = typed;
  else if (keyCleared) body.key = null;
  return body;
}

// ---------------------------------------------------------------- inline checks (mirror ai-service 5.1; the server decides)

export interface GeoProblem {
  /** `urlTemplate`, `keyHeaderName`, `keyHeaderValue`, `key` or `mapping.<name>` (the server's `field` names) */
  field: string;
  i18nKey: MsgKey;
}

const FORBIDDEN_HEADERS = new Set([
  'host', 'content-length', 'transfer-encoding', 'connection', 'cookie', 'accept', 'user-agent', 'upgrade', 'te', 'trailer',
  'proxy-authorization', 'keep-alive',
]);

/** The URL template is valid: scheme, `{ip}`, only `{ip}` / `{key}`, no placeholder or `@` in the authority, no `#` */
export function urlTemplateOk(url: string): boolean {
  if (url.length < 1 || url.length > 500) return false;
  if (/[\s\u0000-\u001f\u007f]/.test(url)) return false;
  if (!(url.startsWith('http://') || url.startsWith('https://'))) return false;
  if (!url.includes('{ip}')) return false;
  if (/[{}]/.test(url.replace(/\{ip\}|\{key\}/g, ''))) return false;
  if (url.includes('#')) return false;
  const rest = url.slice(url.indexOf('://') + 3);
  const authority = rest.split(/[/?]/, 1)[0] ?? '';
  if (authority === '' || authority.includes('{') || authority.includes('@')) return false;
  try {
    return new URL(url.replace(/\{ip\}/g, '203.0.113.5').replace(/\{key\}/g, 'k')).hostname !== '';
  } catch {
    return false;
  }
}

/** A dot path of 1 to 8 segments of letters, digits, `_` and `-` (each 1-64 characters) */
export function dotPathOk(path: string): boolean {
  const segs = path.split('.');
  return segs.length >= 1 && segs.length <= 8 && segs.every((s) => /^[A-Za-z0-9_-]{1,64}$/.test(s));
}

const printable = (s: string): boolean => /^[ -~]*$/.test(s);

/**
 * The first problem of a draft, or null. `keyState` (optional) adds the key checks: the typed text must be 1-400 visible ASCII
 * characters without a space, and a key must exist when `{key}` is in the URL path or the header value.
 */
export function checkDraft(form: GeoForm, keyState?: { typed: string; saved: boolean; cleared: boolean }): GeoProblem | null {
  const url = form.urlTemplate.trim();
  if (!urlTemplateOk(url)) return { field: 'urlTemplate', i18nKey: 'users.geo.code.badUrl' };
  if (form.inHeader) {
    const name = form.headerName.trim();
    const value = form.headerValue.trim();
    if (!/^[A-Za-z0-9-]{1,64}$/.test(name) || FORBIDDEN_HEADERS.has(name.toLowerCase())) {
      return { field: 'keyHeaderName', i18nKey: 'users.geo.code.badHeader' };
    }
    if (value.length < 1 || value.length > 500 || !printable(value) || !value.includes('{key}') || /[{}]/.test(value.replace(/\{key\}/g, ''))) {
      return { field: 'keyHeaderValue', i18nKey: 'users.geo.code.badHeader' };
    }
  }
  const m = form.mapping;
  for (const f of MAPPING_FIELDS) {
    const v = m[f].trim();
    if (f === 'okValue') {
      if (v.length > 64 || !printable(v)) return { field: 'mapping.okValue', i18nKey: 'users.geo.code.badMapping' };
    } else if (v !== '' && !dotPathOk(v)) {
      return { field: `mapping.${f}`, i18nKey: 'users.geo.code.badMapping' };
    }
  }
  if (m.okPath.trim() !== '' && m.okValue.trim() === '') return { field: 'mapping.okValue', i18nKey: 'users.geo.code.badMapping' };
  if (![m.country, m.countryCode, m.region, m.city].some((p) => p.trim() !== '')) {
    return { field: 'mapping.country', i18nKey: 'users.geo.code.badMapping' };
  }
  if (keyState) {
    const typed = keyState.typed.trim();
    if (typed !== '' && !/^[!-~]{1,400}$/.test(typed)) return { field: 'key', i18nKey: 'users.geo.code.badKey' };
    const haveKey = typed !== '' || (keyState.saved && !keyState.cleared);
    const path = url.slice(url.indexOf('://') + 3).replace(/^[^/?]*/, '').split('?', 1)[0] ?? '';
    const inPath = path.includes('{key}');
    const inHeader = form.inHeader && form.headerValue.includes('{key}');
    if (!haveKey && (inPath || inHeader)) return { field: 'key', i18nKey: 'users.geo.code.keyRequired' };
  }
  return null;
}

// ---------------------------------------------------------------- texts

/** The catalog key and parameters that say why a test failed (provider text is shown as it came, already cleaned by the server) */
export function failureText(f: GeoFailure): { key: MsgKey; params: Params } {
  switch (f.code) {
    case 'httpStatus': return { key: 'users.geo.err.httpStatus', params: { status: f.status ?? '' } };
    case 'providerFail': {
      const msg = (f.message ?? '').trim();
      return msg ? { key: 'users.geo.err.providerFail', params: { message: msg } } : { key: 'users.geo.err.providerFailNoMsg', params: {} };
    }
    case 'timeout': return { key: 'users.geo.err.timeout', params: {} };
    case 'unreachable': return { key: 'users.geo.err.unreachable', params: {} };
    case 'redirect': return { key: 'users.geo.err.redirect', params: {} };
    case 'notJson': return { key: 'users.geo.err.notJson', params: {} };
    case 'tooLarge': return { key: 'users.geo.err.tooLarge', params: {} };
    case 'noFields': return { key: 'users.geo.err.noFields', params: {} };
    case 'keyProblem': return { key: 'users.geo.err.keyProblem', params: {} };
    case 'rateLimited': return { key: 'users.geo.err.rateLimited', params: {} };
    case 'notConfigured': return { key: 'users.geo.err.notConfigured', params: {} };
    case 'refused': return { key: 'users.geo.err.refused', params: {} };
    default: return { key: 'users.geo.err.unreachable', params: {} };
  }
}

const CODE_KEYS: Record<string, MsgKey> = {
  'ai.geo.badUrl': 'users.geo.code.badUrl',
  'ai.geo.badHeader': 'users.geo.code.badHeader',
  'ai.geo.badKey': 'users.geo.code.badKey',
  'ai.geo.keyRequired': 'users.geo.code.keyRequired',
  'ai.geo.badMapping': 'users.geo.code.badMapping',
  'ai.geo.testFirst': 'users.geo.code.testFirst',
  'ai.geo.keyProblem': 'users.geo.code.keyProblem',
  'ai.geo.busy': 'users.geo.code.busy',
  'ai.geo.linkDown': 'users.geo.code.linkDown',
};

/** The catalog key of an `ai.geo.*` error code; null for any other code */
export function codeText(code: string | undefined): MsgKey | null {
  return (code && CODE_KEYS[code]) || null;
}

/** The answer of an unreachable ai-service (the page then shows the "no service" state) */
export function isServiceDown(e: { status?: number; kind?: string; code?: string } | null | undefined): boolean {
  if (!e) return false;
  return e.kind === 'network' || e.code === 'ai.unavailable' || e.code === 'ai.backendDown' || e.status === 502 || e.status === 503 || e.status === 504;
}
