// AI settings page, step 1: the connections. One card per connection (name, provider tag, address, key state, status, which
// features use it; edit, test again, delete) and an "add" tile. Edit and add open the form in place. Every save is a TEST FIRST:
// nothing is saved unless the connection answers, so a wrong key or address never replaces a working one. A key that would
// move to another provider or address asks for a confirmation that names the host (the server answers 409 `ai.keyReuseConfirm`).
// A connection address with http:// only gets a note; nothing is restricted. Typed keys are forgotten after a successful save.
// v1.9: every card has an on / off switch. Off keeps all its settings; it is only left out of the pickers and of routing, and a
// feature that uses it says so (nothing falls back to another connection). The list is two collapsible groups, "enabled" and
// "not enabled", both closed at first; their open state is kept for the browser session.
import { ApiError } from '../../api/client.ts';
import { deleteConnection, fetchAiSettings, saveConnection, setConnectionEnabled, testConnection } from '../../api/ai-admin.ts';
import { field } from '../../components/fields.ts';
import { miniIcon } from '../../components/mini-icons.ts';
import { miniSwitch } from '../../components/mini-switch.ts';
import { EFFORT_FAMILIES, EFFORT_PARAM, PROVIDER_NAME, providerIcon, type EffortFamily } from '../../components/provider-icons.ts';
import { refreshAi } from '../../state/actions.ts';
import {
  blankForm, confirmHostOf, connBad, connGroups, connOn, connPayload, connResult, defaultUrl, fieldOfError, fieldsOf, FORMATS, formOf, formProblem, GATEWAY_AUTHS,
  GATEWAY_PLACEHOLDER, headerClash, hostOf, httpWarning, inferFormat, isDirty, isRotation, keyAfter, keyLine, newDefaultChoices,
  PROVIDERS, refKey, parseRefKey, reusableFrom, suggestName, urlAfterSwitch, type ConnForm, type ConnResult,
} from '../../domain/ai-settings.ts';
import { aiCodeText, errorText } from '../../i18n/errors.ts';
import { t, uiLang, type MsgKey } from '../../i18n/index.ts';
import type { AiConnection, AiProvider, AiSettings, AiSlot, ModelRef } from '../../types/api.ts';
import { ask, dialogTitleId } from '../../utils/dialog.ts';
import { h } from '../../utils/dom.ts';
import { toast } from '../../utils/toast.ts';
import { followFamily, inferFamily, savedFamily, type FamilyChoice } from './family.ts';
import { loadModels } from './models.ts';
import { reasonText, aiDown } from './status.ts';
import { A, adopt, listSep, redraw, setGroupOpen } from './state.ts';

const PROVIDER_LABEL: Record<AiProvider, MsgKey> = {
  openai: 'ai.set.provider.openai', anthropic: 'ai.set.provider.anthropic', gateway: 'ai.set.provider.gateway',
};
const GATEWAY_AUTH_LABEL: Record<string, string> = {
  bearer: 'Authorization: Bearer', 'x-api-key': 'x-api-key', 'cf-aig-authorization': 'cf-aig-authorization',
};

const resultText = (r: ConnResult): string => (r.ok ? t(r.key, { n: r.n }) : t(r.key, { reason: aiCodeText(r.code) }));

/** The names of the slots a connection serves ("Default, Chat") */
export const usedByText = (slots: readonly AiSlot[]): string => slots.map((k) => t(`ai.set.slot.${k}` as MsgKey)).join(listSep(uiLang()));

// ---------------------------------------------------------------- the seconds shown while a test runs

/** While a test runs: the seconds so far on the button, so a slow provider does not look like a dead button */
function startTicker(buttonId: string): void {
  stopTicker();
  const started = Date.now();
  A.connSecs = 0;
  A.connTimer = window.setInterval(() => {
    A.connSecs = Math.round((Date.now() - started) / 1000);
    const el = document.getElementById(buttonId);
    if (el) el.textContent = t('ai.set.conn.testing', { n: A.connSecs });
  }, 1000);
}

function stopTicker(): void {
  if (A.connTimer != null) window.clearInterval(A.connTimer);
  A.connTimer = null;
}

// ---------------------------------------------------------------- the thinking-strength family of the open form (v2.1)

/** The family chosen in the open form; it follows the format and address until chosen by hand (a saved connection: only when it differs from the inferred one) */
let fam: FamilyChoice = { value: 'openai', byHand: false };

/** Re-infers the family after the format, address or provider changed, and shows it in the select */
function refollow(): void {
  const f = A.form;
  if (!f) return;
  fam = followFamily(fam, f.provider, f.format, f.baseUrl);
  const sel = document.getElementById('ai-c-family');
  if (sel instanceof HTMLSelectElement) sel.value = fam.value;
}

/** The select label of a family: the provider and the parameter it sends */
const famLabel = (k: EffortFamily): string => `${PROVIDER_NAME[k]} · ${EFFORT_PARAM[k]}`;

// ---------------------------------------------------------------- opening, closing, saving

function openForm(c: AiConnection | null): void {
  const s = A.settings;
  if (!s || A.connBusy) return;
  A.open = c ? c.id : 'new';
  A.form = c ? formOf(c) : blankForm(s.defaults);
  fam = c ? savedFamily(c.effortFamily, A.form.provider, A.form.format, A.form.baseUrl) : { value: inferFamily(A.form.provider, A.form.format, A.form.baseUrl), byHand: false };
  A.fieldErr = {};
  A.connMsg = null;
  redraw();
  document.getElementById('ai-c-name')?.focus();
}

/** Closes the form; the typed keys are gone with it */
export function closeForm(): void {
  const was = A.open;
  A.open = null;
  A.form = null;
  A.fieldErr = {};
  A.connMsg = null;
  stopTicker();
  redraw();
  document.getElementById(was === 'new' || was == null ? 'ai-c-add' : `ai-c-edit-${was}`)?.focus();
}

/** Page left or signed out: the typed keys are forgotten, the form stays closed */
export function forgetConnectionForm(): void {
  A.open = null;
  A.form = null;
  A.fieldErr = {};
  A.connMsg = null;
  stopTicker();
}

async function saveForm(confirmHost?: string): Promise<void> {
  const s = A.settings;
  const f = A.form;
  if (!s || !f || A.connBusy || A.open == null) return;
  const saved = typeof A.open === 'number' ? s.connections.find((c) => c.id === A.open) ?? null : null;
  if (formProblem(f)) {
    A.fieldErr = { name: aiCodeText('ai.badName') };
    redraw();
    document.getElementById('ai-c-name')?.focus();
    return;
  }
  if (saved && !isDirty(saved, f) && fam.value === saved.effortFamily) {
    toast(t('ai.set.nothing'));
    closeForm();
    return;
  }
  const rotation = isRotation(saved, f);
  A.connBusy = true;
  A.fieldErr = {};
  A.connMsg = null;
  redraw();
  startTicker('ai-c-save');
  let again: string | null = null;
  try {
    const a = await saveConnection({ ...connPayload(saved, f, confirmHost), effortFamily: fam.value });
    adopt(a.settings);
    const r = connResult(a, rotation);
    if (a.ok && a.saved) {
      // Saved: the typed keys are forgotten, the card shows the result line
      A.open = null;
      A.form = null;
      A.connMsg = { ok: true, text: resultText(r) };
      // The new connection starts on: its group opens so it can be seen
      setGroupOpen('on', true);
      toast(resultText(r));
      A.models = null;
      void loadModels(false);
      void refreshAi(true);
    } else {
      A.connMsg = { ok: false, text: resultText(r) };
    }
  } catch (e) {
    if (e instanceof ApiError && e.code === 'ai.keyReuseConfirm') {
      const host = typeof e.body?.host === 'string' ? e.body.host : confirmHostOf(f.baseUrl) ?? '';
      const oldHost = typeof e.body?.oldHost === 'string' ? e.body.oldHost : saved ? hostOf(saved.baseUrl) : '';
      again = host;
      A.connBusy = false;
      stopTicker();
      redraw();
      if (!(await ask({ title: t('ai.set.reuse.title'), body: t('ai.set.reuse.body', { host, oldHost }), ok: t('ai.set.reuse.ok'), danger: true }))) again = null;
    } else {
      const where = e instanceof ApiError ? fieldOfError(e) : null;
      if (where) A.fieldErr[where] = errorText(e);
      else if (!aiDown(e)) A.connMsg = { ok: false, text: errorText(e) };
      // Someone else saved this connection meanwhile: read it again (the form keeps what is typed)
      if (e instanceof ApiError && e.code === 'ai.conflict') {
        try {
          adopt(await fetchAiSettings());
        } catch {
          /* the error line is shown anyway */
        }
      }
    }
  }
  A.connBusy = false;
  stopTicker();
  redraw();
  if (again) {
    await saveForm(again);
    return;
  }
  document.getElementById(A.open == null ? 'ai-c-add' : 'ai-c-save')?.focus();
}

async function retest(c: AiConnection): Promise<void> {
  const cur = A.retest[c.id];
  if (cur?.busy) return;
  A.retest[c.id] = { busy: true, msg: null };
  redraw();
  startTicker(`ai-c-retest-${c.id}`);
  try {
    const a = await testConnection(c.id);
    adopt(a.settings);
    A.retest[c.id] = { busy: false, msg: { ok: a.ok, text: resultText(connResult(a, false)) } };
    if (a.ok) {
      A.models = null;
      void loadModels(false);
    }
    void refreshAi(true);
  } catch (e) {
    A.retest[c.id] = { busy: false, msg: aiDown(e) ? null : { ok: false, text: errorText(e) } };
  }
  stopTicker();
  redraw();
  document.getElementById(`ai-c-retest-${c.id}`)?.focus();
}

// ---------------------------------------------------------------- on / off (v1.9)

/** Switches a connection on or off. Turning off one that features use asks first; the answer only changes the switch (nothing is cancelled) */
async function toggleConn(c: AiConnection, on: boolean): Promise<void> {
  if (A.connOnBusy != null) return;
  if (!on && c.usedBy.length) {
    const go = await ask({
      title: t('ai.set.conn.offConfirm', { name: c.name }), body: t('ai.set.conn.offConfirmBody', { list: usedByText(c.usedBy) }),
      ok: t('ai.set.conn.offConfirmGo'), danger: true,
    });
    if (!go) {
      document.getElementById(`ai-c-on-${c.id}`)?.focus();
      return;
    }
  }
  A.connOnBusy = c.id;
  A.retest[c.id] = { busy: false, msg: null };
  redraw();
  try {
    adopt((await setConnectionEnabled(c.id, on)).settings);
    // The card moves to the other group: open that group so it does not disappear
    setGroupOpen(on ? 'on' : 'off', true);
    void refreshAi(true);
    void loadModels(false);
  } catch (e) {
    if (!aiDown(e)) A.retest[c.id] = { busy: false, msg: { ok: false, text: errorText(e) } };
  }
  A.connOnBusy = null;
  redraw();
  document.getElementById(`ai-c-on-${c.id}`)?.focus();
}

// ---------------------------------------------------------------- delete

/** The delete dialog: what uses the connection, a new default when it held the default, a warning when nothing else works */
function deleteDialog(c: AiConnection, s: AiSettings): Promise<{ go: boolean; newDefault?: ModelRef }> {
  return new Promise((resolve) => {
    const titleId = dialogTitleId();
    const holdsDefault = c.usedBy.includes('default');
    const features = c.usedBy.filter((k) => k !== 'default');
    const choices = holdsDefault ? newDefaultChoices(s, c.id) : [];
    const select = h('select', { id: 'ai-del-default', class: 'ais-select' },
      choices.map((m) => h('option', { value: refKey({ connectionId: m.connectionId, model: m.model }) }, `${m.connectionName} · ${m.model}`)));
    const body: Array<HTMLElement | null> = [
      features.length ? h('p', null, t('ai.set.del.usedBy', { list: usedByText(features) })) : null,
      holdsDefault && choices.length
        ? h('div', { class: 'fld' }, h('label', { class: 'fld-l', for: 'ai-del-default' }, t('ai.set.del.newDefault')), select)
        : null,
      holdsDefault && !choices.length ? h('p', { class: 'ais-warn' }, t('ai.set.del.noneLeft')) : null,
    ];
    const d = h('dialog', { class: 'dlg dlg-choice', 'aria-labelledby': titleId },
      h('h3', { id: titleId, tabindex: '-1' }, t('ai.set.del.title', { name: c.name })),
      ...body,
      h('div', { class: 'acts' },
        h('button', { class: 'btn', type: 'button', onclick: () => d.close('cancel') }, t('common.cancel')),
        h('button', { class: 'btn danger solid', type: 'button', onclick: () => d.close('go') }, t('ai.set.conn.delete'))));
    d.addEventListener('close', () => {
      const go = d.returnValue === 'go';
      const pick = choices.length ? parseRefKey(select.value) : null;
      d.remove();
      resolve(go ? (pick ? { go, newDefault: pick } : { go }) : { go: false });
    });
    document.body.append(d);
    if (typeof d.showModal === 'function') d.showModal();
    else d.setAttribute('open', '');
    document.getElementById(titleId)?.focus();
  });
}

/** A save / test result: success is the green status line, a failure is the shared red .err block */
function msgBlock(m: { ok: boolean; text: string }): HTMLElement {
  return m.ok
    ? h('p', { class: 'ais-ok', role: 'status' }, m.text)
    : h('div', { class: 'err', role: 'alert' }, h('p', null, m.text));
}

async function removeConnection(c: AiConnection): Promise<void> {
  const s = A.settings;
  if (!s) return;
  const inUse = c.usedBy.length > 0;
  let opts: { force?: boolean; newDefault?: ModelRef } = {};
  if (inUse) {
    const r = await deleteDialog(c, s);
    if (!r.go) return;
    opts = { force: true, ...(r.newDefault ? { newDefault: r.newDefault } : {}) };
  } else if (!(await ask({ title: t('ai.set.del.title', { name: c.name }), body: '', ok: t('ai.set.conn.delete'), danger: true }))) {
    return;
  }
  try {
    const a = await deleteConnection(c.id, opts);
    adopt(a.settings);
    delete A.retest[c.id];
    A.models = null;
    void loadModels(false);
    void refreshAi(true);
  } catch (e) {
    if (!aiDown(e)) toast(errorText(e), 6000);
    // The connection may have been used or changed meanwhile: show the current state
    try {
      adopt(await fetchAiSettings());
    } catch {
      /* unchanged */
    }
  }
  redraw();
  document.getElementById('ai-c-add')?.focus();
}

// ---------------------------------------------------------------- the cards

function keyText(c: AiConnection): string {
  return keyLine(c).map((k) => {
    if (k.state === 'saved') return k.hint ? t('ai.set.conn.keySaved', { hint: k.hint }) : t('ai.set.keySavedShort');
    return k.state === 'unreadable' ? t('ai.set.keyUnreadable') : t('ai.set.keyNone');
  }).join(' · ');
}

function connCard(c: AiConnection): HTMLElement {
  const unreadable = c.keys.apiKey.unreadable || c.keys.gatewayKey.unreadable;
  const on = connOn(c);
  // A connection that is off is not a problem, whatever its last status was (no red border)
  const bad = connBad(c);
  const host = hostOf(c.baseUrl);
  const rt = A.retest[c.id];
  const busy = rt?.busy === true;
  const chip = !on ? h('span', { class: 'ais-chip off' }, t('ai.set.conn.statusOff'))
    : h('span', { class: 'ais-chip ' + (bad ? 'fail' : 'ok') },
      bad ? t('ai.set.conn.statusBad', { reason: unreadable ? aiCodeText('ai.keyProblem') : reasonText(c.errorCode) }) : t('ai.set.conn.statusOk'));
  return h('li', { class: 'ais-conn' + (bad ? ' bad' : '') + (on ? '' : ' off'), id: `ais-conn-${c.id}` },
    h('div', { class: 'ais-conn-head' },
      h('span', { class: 'ais-conn-name' }, c.name),
      chip,
      miniSwitch({
        id: `ai-c-on-${c.id}`, checked: on, label: t('ai.set.conn.enableAria', { name: c.name }), disabled: A.connOnBusy != null || busy || A.connBusy,
        onToggle: (v) => void toggleConn(c, v),
      })),
    // A-OWNER-95: provider and address are two parts; on a phone the address is on its own line and never split
    h('p', { class: 'ais-conn-addr' },
      h('span', { class: 'ais-sum-p' }, t(PROVIDER_LABEL[c.provider]), h('span', { class: 'ais-sum-dot', 'aria-hidden': 'true' }, ' · ')),
      h('span', { class: 'ais-sum-host', title: c.baseUrl }, host)),
    h('p', { class: 'ais-conn-fam', title: EFFORT_PARAM[c.effortFamily] }, providerIcon(c.effortFamily),
      h('span', null, t('ai.set.conn.familyOf', { name: PROVIDER_NAME[c.effortFamily] }))),
    h('p', { class: 'ais-note ais-conn-keys' }, keyText(c)),
    c.usedBy.length ? h('p', { class: 'ais-note ais-conn-used' }, t('ai.set.conn.usedBy', { list: usedByText(c.usedBy) })) : null,
    c.insecureHttp ? h('p', { class: 'ais-note ais-conn-http' }, t('ai.set.conn.httpNote')) : null,
    rt?.msg ? msgBlock(rt.msg) : null,
    h('div', { class: 'acts ais-conn-acts' },
      h('button', { class: 'btn', id: `ai-c-edit-${c.id}`, type: 'button', disabled: A.connBusy, onclick: () => openForm(c) }, t('ai.set.conn.edit')),
      h('button', { class: 'btn', id: `ai-c-retest-${c.id}`, type: 'button', disabled: busy || A.connBusy, onclick: () => void retest(c) },
        busy ? t('ai.set.conn.testing', { n: A.connSecs }) : t('ai.set.conn.retest')),
      h('button', { class: 'btn danger', id: `ai-c-del-${c.id}`, type: 'button', disabled: busy || A.connBusy, onclick: () => void removeConnection(c) }, t('ai.set.conn.delete'))));
}

// ---------------------------------------------------------------- the form

function keyHint(saved: AiConnection | null, slot: 'apiKey' | 'gatewayKey'): string {
  const k = saved?.keys[slot];
  if (!k || !k.set) return t('ai.set.keyNone');
  const state = k.unreadable ? t('ai.set.keyUnreadable') : k.hint ? t('ai.set.keySaved', { hint: k.hint }) : t('ai.set.keySavedShort');
  return `${state} · ${t('ai.set.keyKeep')}`;
}

function fieldError(where: 'name' | 'baseUrl' | 'apiKey' | 'gatewayKey'): HTMLElement | null {
  const text = A.fieldErr[where];
  return text ? h('p', { class: 'ais-ferr', role: 'alert' }, text) : null;
}

/** Updates what depends on the typed text without a redraw (the caret stays) */
function syncForm(): void {
  const f = A.form;
  const s = A.settings;
  if (!f || !s) return;
  const warn = document.getElementById('ai-c-http');
  if (warn) warn.hidden = !httpWarning(f.baseUrl);
  const saved = typeof A.open === 'number' ? s.connections.find((c) => c.id === A.open) ?? null : null;
  const clash = document.getElementById('ai-c-clash');
  if (clash) clash.hidden = !headerClash(f, keyAfter(saved, f, 'apiKey'));
}

function connForm(s: AiSettings): HTMLElement {
  const f = A.form as ConnForm;
  const saved = typeof A.open === 'number' ? s.connections.find((c) => c.id === A.open) ?? null : null;
  const busy = A.connBusy;
  const fields = fieldsOf(f.provider);
  const pick = (p: AiProvider): void => {
    if (!A.form || A.form.provider === p) return;
    const prev = A.form;
    const url = urlAfterSwitch(prev.baseUrl, prev.provider, p, s.defaults);
    const follow = !saved && prev.name === suggestName(prev.baseUrl);
    A.form = { ...prev, provider: p, baseUrl: url, format: p === 'gateway' ? inferFormat(url) : prev.format, name: follow ? suggestName(url) : prev.name };
    fam = followFamily(fam, A.form.provider, A.form.format, A.form.baseUrl);
    A.fieldErr = {};
    redraw();
  };
  const provider = h('fieldset', { class: 'fld ais-provider' },
    h('legend', { class: 'fld-l' }, t('ai.set.conn.provider')),
    h('div', { class: 'ais-seg' }, PROVIDERS.map((p) => h('label', { class: 'ais-seg-o' },
      h('input', { type: 'radio', name: 'ai-c-prov', id: `ai-c-prov-${p}`, value: p, checked: p === f.provider, disabled: busy, onchange: () => pick(p) }),
      h('span', null, t(PROVIDER_LABEL[p])), miniIcon('check', 'ais-seg-ck')))));

  const name = field({
    id: 'ai-c-name', label: t('ai.set.conn.name'), value: f.name, autocomplete: 'off', maxlength: 40, disabled: busy,
    onInput: (v) => {
      if (A.form) A.form = { ...A.form, name: v };
    },
  });
  const url = field({
    id: 'ai-c-url', label: t(f.provider === 'gateway' ? 'ai.set.gatewayUrl' : 'ai.set.baseUrl'), value: f.baseUrl, autocomplete: 'url', maxlength: 300,
    hint: f.provider === 'openai' ? t('ai.set.baseUrlHint.openai') : f.provider === 'anthropic' ? t('ai.set.baseUrlHint.anthropic') : undefined,
    disabled: busy,
    onInput: (v) => {
      if (!A.form) return;
      const prev = A.form;
      // A gateway's API format follows the address while it was not chosen by hand; a new connection's name follows the host
      const followed = prev.provider === 'gateway' && prev.format === inferFormat(prev.baseUrl);
      const nameFollows = !saved && prev.name === suggestName(prev.baseUrl);
      A.form = { ...prev, baseUrl: v, format: followed ? inferFormat(v) : prev.format, name: nameFollows ? suggestName(v) : prev.name };
      const sel = document.getElementById('ai-c-format');
      if (sel instanceof HTMLSelectElement) sel.value = A.form.format;
      refollow();
      if (nameFollows) name.input.value = A.form.name;
      syncForm();
    },
  });
  url.input.type = 'url';
  url.input.inputMode = 'url';
  url.input.placeholder = f.provider === 'gateway' ? GATEWAY_PLACEHOLDER : defaultUrl(f.provider, s.defaults);

  const gateway = fields.gateway ? h('div', { class: 'ais-row2' },
    h('div', { class: 'fld' },
      h('label', { class: 'fld-l', for: 'ai-c-format' }, t('ai.set.format')),
      h('select', {
        id: 'ai-c-format', class: 'ais-select', disabled: busy,
        onchange: (e: Event) => {
          if (A.form) A.form = { ...A.form, format: (e.target as HTMLSelectElement).value as ConnForm['format'] };
          refollow();
          syncForm();
        },
      }, FORMATS.map((x) => h('option', { value: x, selected: x === f.format ? 'selected' : null }, t(x === 'openai' ? 'ai.set.format.openai' : x === 'responses' ? 'ai.set.format.responses' : 'ai.set.format.anthropic'))))),
    h('div', { class: 'fld' },
      h('label', { class: 'fld-l', for: 'ai-c-gwauth' }, t('ai.set.gatewayAuth')),
      h('select', {
        id: 'ai-c-gwauth', class: 'ais-select', disabled: busy,
        onchange: (e: Event) => {
          if (A.form) A.form = { ...A.form, gatewayAuth: (e.target as HTMLSelectElement).value as ConnForm['gatewayAuth'] };
          syncForm();
        },
      }, GATEWAY_AUTHS.map((x) => h('option', { value: x, selected: x === f.gatewayAuth ? 'selected' : null }, GATEWAY_AUTH_LABEL[x] ?? x))))) : null;

  const family = h('div', { class: 'fld ais-family' },
    h('label', { class: 'fld-l', for: 'ai-c-family' }, t('ai.set.conn.family')),
    h('select', {
      id: 'ai-c-family', class: 'ais-select', disabled: busy, 'aria-describedby': 'ai-c-family-help',
      onchange: (e: Event) => {
        fam = { value: (e.target as HTMLSelectElement).value as EffortFamily, byHand: true };
      },
    }, EFFORT_FAMILIES.map((k) => h('option', { value: k, selected: k === fam.value ? 'selected' : null }, famLabel(k)))),
    h('p', { class: 'fld-hint', id: 'ai-c-family-help' }, t('ai.set.conn.familyHelp')));

  const keyFields = fields.slots.map((slot) => {
    const label = slot === 'gatewayKey' ? t('ai.set.gatewayKey') : f.provider === 'gateway' ? t('ai.set.upstreamKey') : t('ai.set.apiKey');
    const k = saved?.keys[slot];
    const fld = field({
      id: `ai-key-${slot}`, label, type: 'password', autocomplete: 'new-password', value: f.keys[slot], maxlength: 400,
      hint: keyHint(saved, slot), disabled: busy,
      onInput: (v) => {
        if (A.form) A.form = { ...A.form, keys: { ...A.form.keys, [slot]: v } };
        syncForm();
      },
    });
    // The reveal button of the field names the key, not "password"
    const eye = fld.el.querySelector<HTMLButtonElement>('.fld-eye');
    if (eye) {
      eye.setAttribute('aria-label', t('ai.set.showKey'));
      eye.title = t('ai.set.showKey');
      eye.addEventListener('click', () => {
        eye.title = t(eye.getAttribute('aria-pressed') === 'true' ? 'ai.set.hideKey' : 'ai.set.showKey');
      });
    }
    const remove = k?.set
      ? h('label', { class: 'ais-remove', for: `ai-rm-${slot}` },
        h('input', {
          type: 'checkbox', id: `ai-rm-${slot}`, checked: f.remove[slot], disabled: busy,
          onchange: (e: Event) => {
            if (A.form) A.form = { ...A.form, remove: { ...A.form.remove, [slot]: (e.target as HTMLInputElement).checked } };
            syncForm();
          },
        }), h('span', null, t('ai.set.keyRemove')))
      : null;
    return h('div', { class: 'ais-key' }, fld.el, remove, fieldError(slot));
  });

  // A new connection can start from the saved keys of another one (the server asks to confirm the new host)
  const donors = saved ? [] : reusableFrom(s);
  const reuse = donors.length
    ? h('div', { class: 'fld ais-reuse' },
      h('label', { class: 'fld-l', for: 'ai-c-reuse' }, t('ai.set.conn.reuseFrom')),
      h('select', {
        id: 'ai-c-reuse', class: 'ais-select', disabled: busy,
        onchange: (e: Event) => {
          const v = Number((e.target as HTMLSelectElement).value);
          if (A.form) A.form = { ...A.form, reuseFrom: v > 0 ? v : null };
        },
      }, h('option', { value: '' }, t('ai.set.conn.reuseNone')),
      donors.map((c) => h('option', { value: String(c.id), selected: f.reuseFrom === c.id ? 'selected' : null }, c.name))))
    : null;

  return h('li', { class: 'ais-conn open', id: 'ais-conn-form' },
    h('form', {
      class: 'ais-form', novalidate: true, 'aria-busy': busy ? 'true' : null,
      onsubmit: (e: SubmitEvent) => {
        e.preventDefault();
        void saveForm();
      },
    },
    name.el, fieldError('name'), provider,
    h('div', { class: 'ais-url' }, url.el,
      h('p', { class: 'ais-warn', id: 'ai-c-http', hidden: !httpWarning(f.baseUrl) }, t('ai.set.conn.httpNote')),
      fieldError('baseUrl')),
    gateway,
    family,
    reuse,
    h('div', { class: 'ais-keys' }, keyFields),
    h('p', { class: 'ais-ferr', id: 'ai-c-clash', hidden: !headerClash(f, keyAfter(saved, f, 'apiKey')) }, aiCodeText('ai.headerClash')),
    A.connMsg ? msgBlock(A.connMsg) : null,
    h('div', { class: 'acts' },
      h('button', { class: 'btn primary', id: 'ai-c-save', type: 'submit', disabled: busy },
        busy ? t('ai.set.conn.testing', { n: A.connSecs }) : t('ai.set.conn.save')),
      h('button', { class: 'btn', id: 'ai-c-cancel', type: 'button', disabled: busy, onclick: () => closeForm() }, t('common.cancel')))));
}

// ---------------------------------------------------------------- the section

/** One collapsible group of the list (enabled / not enabled): a header button with the count, then the cards */
function connGroup(s: AiSettings, k: 'on' | 'off', list: AiConnection[]): HTMLElement {
  // A form that is open for one of its connections keeps the group open (the header then does nothing)
  const forced = typeof A.open === 'number' && A.form != null && list.some((c) => c.id === A.open);
  const open = A.connGroupOpen[k] || forced;
  const id = `ais-cg-${k}`;
  const problems = list.filter(connBad).length;
  const items: HTMLElement[] = list.map((c) => (A.open === c.id && A.form ? connForm(s) : connCard(c)));
  if (!items.length) items.push(h('li', { class: 'ais-note' }, t('ai.set.conn.groupEmpty')));
  return h('div', { class: 'ais-cgroup' },
    h('button', {
      class: 'ais-cgroup-h' + (open ? ' open' : ''), id, type: 'button', 'aria-expanded': String(open), 'aria-controls': `${id}-list`,
      onclick: () => {
        if (forced) return;
        setGroupOpen(k, !open);
        redraw();
        document.getElementById(id)?.focus();
      },
    }, miniIcon('chevDown', 'ais-cg-chev'), h('span', null, t(k === 'on' ? 'ai.set.conn.groupOn' : 'ai.set.conn.groupOff', { n: list.length })),
      // A collapsed group must not hide a broken connection: the header carries a warning with the count
      problems ? h('span', { class: 'ais-cg-warn' }, h('span', { 'aria-hidden': 'true' }, '! '), t('ai.set.conn.groupBad', { n: problems })) : null),
    h('ul', { class: 'ais-conns', id: `${id}-list`, hidden: !open }, items));
}

export function connectionsSection(s: AiSettings): HTMLElement {
  const full = s.connections.length >= s.maxConnections;
  const groups = connGroups(s);
  // The add tile and an open "new connection" form stay outside the groups
  const items: HTMLElement[] = [];
  if (A.open === 'new' && A.form) items.push(connForm(s));
  else {
    items.push(h('li', { class: 'ais-conn ais-conn-add' },
      h('button', {
        class: 'btn', id: 'ai-c-add', type: 'button', disabled: full || A.connBusy, title: full ? t('ai.set.conn.addFull', { n: s.maxConnections }) : null,
        onclick: () => openForm(null),
      }, miniIcon('plus', 'ais-add-ic'), t('ai.set.conn.add')),
      full ? h('p', { class: 'ais-note' }, t('ai.set.conn.addFull', { n: s.maxConnections })) : null));
  }
  return h('section', { class: 'panel ais-sec', 'aria-labelledby': 'ais-conn-h' },
    h('h2', { id: 'ais-conn-h', tabindex: '-1' }, t('ai.set.card.conn')),
    // The result of the last save (the form is closed then); a failed save shows its line inside the form
    A.connMsg && A.open == null ? msgBlock(A.connMsg) : null,
    // Two empty groups would only be noise before the first connection exists
    s.connections.length ? connGroup(s, 'on', groups.on) : null,
    s.connections.length ? connGroup(s, 'off', groups.off) : null,
    h('ul', { class: 'ais-conns ais-conns-tail' }, items));
}
