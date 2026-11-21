import assert from 'node:assert/strict';
import { test } from 'node:test';
import { hasKey, setUiLang, t, type MsgKey } from '../i18n/index.ts';
import type { GeoFailureCode, GeoPresetId } from '../types/api.ts';
import {
  GEO_PRESETS, MAPPING_FIELDS, blankForm, checkDraft, codeText, dotPathOk, draftBody, failureText, formFromSettings, ipApiLang,
  isInsecureHttp, isServiceDown, placeLocation, presetDraft, urlTemplateOk, type GeoForm,
} from './geo-source.ts';

const preset = (id: GeoPresetId, lang = 'en'): GeoForm => presetDraft(id, lang, null);

test('presets: exact templates and mapping paths', () => {
  assert.deepEqual(GEO_PRESETS.map((p) => p.id), ['ipinfo', 'ipapi', 'ipapico', 'ipwhois', 'ipsb', 'custom']);
  const row = (id: GeoPresetId): string[] => {
    const f = preset(id);
    const m = f.mapping;
    return [f.urlTemplate, m.country, m.countryCode, m.region, m.city, m.okPath, m.okValue, m.errorPath, m.messagePath];
  };
  assert.deepEqual(row('ipinfo'), ['https://ipinfo.io/{ip}/json?token={key}', '', 'country', 'region', 'city', '', '', 'error', 'error.message']);
  assert.deepEqual(row('ipapi'), ['http://ip-api.com/json/{ip}?lang=en', 'country', 'countryCode', 'regionName', 'city', 'status', 'success', '', 'message']);
  assert.deepEqual(row('ipapico'), ['https://ipapi.co/{ip}/json/?key={key}', 'country_name', 'country_code', 'region', 'city', '', '', 'error', 'reason']);
  assert.deepEqual(row('ipwhois'), ['https://ipwho.is/{ip}?lang=en', 'country', 'country_code', 'region', 'city', 'success', 'true', '', 'message']);
  assert.deepEqual(row('ipsb'), ['https://api.ip.sb/geoip/{ip}', 'country', 'country_code', 'region', 'city', '', '', '', '']);
  for (const p of GEO_PRESETS) {
    if (p.id === 'custom') continue;
    const f = preset(p.id);
    assert.equal(f.inHeader, false);
    assert.equal(checkDraft(f), null, `${p.id} passes its own checks`);
    assert.deepEqual(Object.keys(f.mapping), [...MAPPING_FIELDS]);
  }
});

test('presets: the ip-api language follows the UI language', () => {
  assert.equal(ipApiLang('zh'), 'zh-CN');
  assert.equal(ipApiLang('ja'), 'ja');
  assert.equal(ipApiLang('en'), 'en');
  assert.equal(ipApiLang('fr'), 'en');
  assert.equal(preset('ipapi', 'zh').urlTemplate, 'http://ip-api.com/json/{ip}?lang=zh-CN');
  assert.equal(preset('ipapi', 'ja').urlTemplate, 'http://ip-api.com/json/{ip}?lang=ja');
  assert.equal(preset('ipapi', 'en').urlTemplate, 'http://ip-api.com/json/{ip}?lang=en');
  // ipwho.is takes the same language codes (checked with the public example IP: zh-CN and ja give Chinese / Japanese names)
  assert.equal(preset('ipwhois', 'zh').urlTemplate, 'https://ipwho.is/{ip}?lang=zh-CN');
  assert.equal(preset('ipwhois', 'ja').urlTemplate, 'https://ipwho.is/{ip}?lang=ja');
});

test('presetDraft: a provider clears the header fields, custom keeps the current values', () => {
  const cur: GeoForm = { ...preset('ipinfo'), inHeader: true, headerName: 'X-Example', headerValue: 'Key {key}', urlTemplate: 'https://provider.example/{ip}' };
  const other = presetDraft('ipwhois', 'en', cur);
  assert.equal(other.inHeader, false);
  assert.equal(other.urlTemplate, 'https://ipwho.is/{ip}?lang=en');
  const custom = presetDraft('custom', 'en', cur);
  assert.equal(custom.preset, 'custom');
  assert.equal(custom.urlTemplate, 'https://provider.example/{ip}');
  assert.equal(custom.inHeader, true);
  assert.equal(custom.headerName, 'X-Example');
  assert.deepEqual(custom.mapping, cur.mapping);
  assert.notEqual(custom.mapping, cur.mapping);
  assert.equal(presetDraft('custom', 'en', null).urlTemplate, '');
  assert.equal(blankForm('en').preset, 'ipinfo');
});

test('isInsecureHttp: only http:// warns', () => {
  assert.equal(isInsecureHttp('http://provider.example/{ip}'), true);
  assert.equal(isInsecureHttp('  HTTP://provider.example/{ip}'), true);
  assert.equal(isInsecureHttp('https://provider.example/{ip}'), false);
  assert.equal(isInsecureHttp(''), false);
  assert.equal(preset('ipapi').urlTemplate.startsWith('http://'), true);
});

test('draftBody: key semantics (absent, null, string) and header fields', () => {
  const f = preset('ipinfo');
  assert.equal('key' in draftBody(f, '', false), false);
  assert.equal(draftBody(f, '', true).key, null);
  assert.equal(draftBody(f, 'TESTKEY-geo-0123456789abcdef', false).key, 'TESTKEY-geo-0123456789abcdef');
  assert.equal(draftBody(f, '  TESTKEY-geo-0123456789abcdef \n', true).key, 'TESTKEY-geo-0123456789abcdef');
  assert.equal(draftBody(f, '   ', false).key, undefined);
  // header fields are empty while the box is off, whatever they hold
  const off = { ...f, inHeader: false, headerName: 'Authorization', headerValue: 'Bearer {key}' };
  assert.equal(draftBody(off, '', false).keyHeaderName, '');
  assert.equal(draftBody(off, '', false).keyHeaderValue, '');
  const on = { ...off, inHeader: true };
  assert.equal(draftBody(on, '', false).keyHeaderName, 'Authorization');
  assert.equal(draftBody(on, '', false).keyHeaderValue, 'Bearer {key}');
  const body = draftBody(f, '', false);
  assert.deepEqual(Object.keys(body.mapping), [...MAPPING_FIELDS]);
  assert.equal(body.preset, 'ipinfo');
});

test('formFromSettings: header box on only when a header name is saved; the key is never part of it', () => {
  const settings = {
    preset: 'custom' as const, urlTemplate: 'https://provider.example/{ip}', keyHeaderName: 'Authorization', keyHeaderValue: 'Bearer {key}',
    key: { set: true, hint: '…cdef', unreadable: false }, mapping: preset('ipwhois').mapping, host: 'provider.example', insecureHttp: false,
    testedAt: 1, updatedAt: 1,
  };
  const f = formFromSettings(settings);
  assert.equal(f.inHeader, true);
  assert.equal(f.headerName, 'Authorization');
  assert.equal(JSON.stringify(f).includes('cdef'), false);
  const none = formFromSettings({ ...settings, keyHeaderName: '', keyHeaderValue: '' });
  assert.equal(none.inHeader, false);
  assert.equal(none.headerName, 'Authorization');
});

test('urlTemplateOk: every rule both ways', () => {
  for (const ok of [
    'https://provider.example/{ip}/json', 'http://provider.example:8080/json/{ip}?token={key}', 'https://provider.example/{ip}',
    'https://provider.example/lookup?ip={ip}&lang=en', 'https://provider.example/a/{key}/{ip}',
  ]) assert.equal(urlTemplateOk(ok), true, ok);
  for (const bad of [
    '', 'ftp://provider.example/{ip}', 'HTTPS://provider.example/{ip}', 'https://provider.example/json', 'https://provider.example/{ip}/{other}',
    'https://provider.example/{ip}}', 'https://{ip}/json', 'https://{key}.provider.example/{ip}', 'https://user@provider.example/{ip}',
    'https://provider.example/{ip}#frag', 'https://provider.example/{ip} x', 'https://provider.example/{ip}\n', 'https:///{ip}',
    `https://provider.example/${'a'.repeat(500)}/{ip}`,
  ]) assert.equal(urlTemplateOk(bad), false, bad);
});

test('dotPathOk: segments, depth and characters', () => {
  for (const ok of ['city', 'a.b', 'error.message', 'data.0.city', 'country_code', 'x-y.z_1', 'a.b.c.d.e.f.g.h']) assert.equal(dotPathOk(ok), true, ok);
  for (const bad of ['', 'a..b', '.a', 'a.', 'a b', 'a.b.c.d.e.f.g.h.i', 'a/b', 'a[0]', `${'x'.repeat(65)}`]) assert.equal(dotPathOk(bad), false, bad);
});

test('checkDraft mirrors the server rules and names the field', () => {
  const base = preset('ipinfo');
  const field = (f: GeoForm, keyState?: Parameters<typeof checkDraft>[1]): string | null => checkDraft(f, keyState)?.field ?? null;
  assert.equal(checkDraft(base), null);
  assert.equal(field({ ...base, urlTemplate: 'https://provider.example/json' }), 'urlTemplate');
  assert.equal(checkDraft({ ...base, urlTemplate: '' })?.i18nKey, 'users.geo.code.badUrl');
  // header: off = never checked
  assert.equal(field({ ...base, inHeader: false, headerName: 'Host', headerValue: 'x' }), null);
  assert.equal(field({ ...base, inHeader: true, headerName: 'Host', headerValue: 'Bearer {key}' }), 'keyHeaderName');
  assert.equal(field({ ...base, inHeader: true, headerName: 'Bad Name', headerValue: 'Bearer {key}' }), 'keyHeaderName');
  assert.equal(field({ ...base, inHeader: true, headerName: '', headerValue: 'Bearer {key}' }), 'keyHeaderName');
  assert.equal(field({ ...base, inHeader: true, headerName: 'Authorization', headerValue: 'Bearer token' }), 'keyHeaderValue');
  assert.equal(field({ ...base, inHeader: true, headerName: 'Authorization', headerValue: 'Bearer {key} {other}' }), 'keyHeaderValue');
  assert.equal(field({ ...base, inHeader: true, headerName: 'Authorization', headerValue: 'Bearer {key}' }), null);
  assert.equal(checkDraft({ ...base, inHeader: true, headerName: 'Cookie', headerValue: '{key}' })?.i18nKey, 'users.geo.code.badHeader');
  // mapping
  assert.equal(field({ ...base, mapping: { ...base.mapping, city: 'a..b' } }), 'mapping.city');
  assert.equal(field({ ...base, mapping: { ...base.mapping, errorPath: 'a b' } }), 'mapping.errorPath');
  assert.equal(field({ ...base, mapping: { ...base.mapping, countryCode: '', region: '', city: '' } }), 'mapping.country');
  assert.equal(field({ ...base, mapping: { ...base.mapping, okPath: 'status', okValue: '' } }), 'mapping.okValue');
  assert.equal(checkDraft({ ...base, mapping: { ...base.mapping, okPath: 'status', okValue: 'success' } }), null);
});

test('checkDraft: key checks need the key state', () => {
  const withPath = { ...preset('custom'), preset: 'custom' as const, urlTemplate: 'https://provider.example/{key}/{ip}', mapping: preset('ipwhois').mapping };
  const key = (typed: string, saved: boolean, cleared: boolean) => checkDraft(withPath, { typed, saved, cleared })?.i18nKey ?? null;
  assert.equal(key('', false, false), 'users.geo.code.keyRequired');
  assert.equal(key('', true, false), null);
  assert.equal(key('', true, true), 'users.geo.code.keyRequired');
  assert.equal(key('TESTKEY-geo-0123456789abcdef', false, false), null);
  assert.equal(key('has space', false, false), 'users.geo.code.badKey');
  // {key} only in the query works without a key (the server drops the parameter)
  assert.equal(checkDraft(preset('ipinfo'), { typed: '', saved: false, cleared: false }), null);
  // {key} in the header value needs a key too
  const hdr = { ...preset('ipsb'), inHeader: true, headerName: 'Authorization', headerValue: 'Bearer {key}' };
  assert.equal(checkDraft(hdr, { typed: '', saved: false, cleared: false })?.field, 'key');
  assert.equal(checkDraft(hdr, { typed: 'TESTKEY-geo-0123456789abcdef', saved: false, cleared: false }), null);
});

test('failureText: a text for every failure code, in all three languages', () => {
  const codes: GeoFailureCode[] = [
    'timeout', 'unreachable', 'redirect', 'httpStatus', 'notJson', 'tooLarge', 'providerFail', 'noFields', 'keyProblem', 'rateLimited',
    'notConfigured', 'refused',
  ];
  for (const lang of ['en', 'zh', 'ja'] as const) {
    setUiLang(lang);
    for (const code of codes) {
      const { key, params } = failureText({ code, status: code === 'httpStatus' ? 500 : null, message: code === 'providerFail' ? 'reserved range' : null });
      assert.equal(hasKey(lang, key), true, `${lang}:${key}`);
      const text = t(key, params);
      assert.equal(/\{\w+\}/.test(text), false, `${lang}:${code} leaves a placeholder: ${text}`);
    }
  }
  setUiLang('en');
  assert.equal(t(failureText({ code: 'httpStatus', status: 503, message: null }).key, failureText({ code: 'httpStatus', status: 503, message: null }).params), 'The provider answered HTTP 503');
  assert.equal(failureText({ code: 'providerFail', status: null, message: null }).key, 'users.geo.err.providerFailNoMsg');
  assert.equal(failureText({ code: 'providerFail', status: null, message: '  ' }).key, 'users.geo.err.providerFailNoMsg');
  assert.deepEqual(failureText({ code: 'providerFail', status: null, message: 'reserved range' }), { key: 'users.geo.err.providerFail', params: { message: 'reserved range' } });
});

test('codeText: every ai.geo.* code has a catalog text; other codes none', () => {
  for (const c of ['badUrl', 'badHeader', 'badKey', 'keyRequired', 'badMapping', 'testFirst', 'keyProblem', 'busy', 'linkDown']) {
    const k = codeText(`ai.geo.${c}`) as MsgKey;
    assert.equal(k, `users.geo.code.${c}`);
    for (const lang of ['en', 'zh', 'ja'] as const) assert.equal(hasKey(lang, k), true, `${lang}:${k}`);
  }
  assert.equal(codeText('ai.busy'), null);
  assert.equal(codeText(undefined), null);
  setUiLang('zh');
  assert.equal(t('users.geo.code.badUrl'), '查询地址无效：须以 http:// 或 https:// 开头，包含 {ip}，且只能使用 {ip} 和 {key}。');
  setUiLang('en');
});

test('placeLocation: a country code wins over the name, as the backend stores it', () => {
  assert.deepEqual(placeLocation({ country: 'Example Land', countryCode: 'zz', region: 'Example Region', city: 'Example City' }), { country: 'ZZ', region: 'Example Region', city: 'Example City' });
  assert.deepEqual(placeLocation({ country: 'Example Land', countryCode: null, region: null, city: null }), { country: 'Example Land', region: null, city: null });
  assert.deepEqual(placeLocation({ country: 'Example Land', countryCode: 'XX', region: null, city: null }).country, 'Example Land');
  assert.deepEqual(placeLocation({ country: null, countryCode: 'T1', region: null, city: null }).country, null);
});

test('isServiceDown: ai-service unreachable shapes', () => {
  assert.equal(isServiceDown({ kind: 'network' }), true);
  assert.equal(isServiceDown({ code: 'ai.unavailable', status: 502 }), true);
  assert.equal(isServiceDown({ code: 'ai.backendDown' }), true);
  assert.equal(isServiceDown({ status: 503 }), true);
  assert.equal(isServiceDown({ status: 403, code: 'auth.forbidden' }), false);
  assert.equal(isServiceDown(null), false);
});

test('the privacy line and the source labels exist in all three languages', () => {
  for (const lang of ['en', 'zh', 'ja'] as const) {
    for (const k of ['users.geo.title', 'users.geo.cf', 'users.geo.cf.hint', 'users.geo.api', 'users.geo.api.hint', 'users.geo.http'] as const) {
      assert.equal(hasKey(lang, k), true, `${lang}:${k}`);
    }
  }
  setUiLang('en');
  assert.match(t('users.geo.api.hint'), /visitor’s IP address is sent to the chosen provider/);
});
