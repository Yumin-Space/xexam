// AI settings page, steps 2 and 3. Step 2 turns AI+ on: a default model is chosen from the merged list and tested, and only a
// passing test switches AI+ on (a failing one changes nothing). Step 3 is six model rows (default, chat, hint + explanation,
// summary, AI questions, AI import). Choosing a model in a row tests it first (skipped when it passed in the last 10 minutes),
// then the row changes at once; running calls are not cancelled and AI+ is never switched off by a pick. When a row's model
// cannot be used the default stands in (a note says so); when the default cannot be used either the row says it is paused.
// Nothing is ever switched to another model automatically. Each row has two pickers: the connection on the left and, on the
// right, the models of that connection only. Changing the connection does not switch the row; picking a model does.
import { fetchModels, pickModel, setEffort, setEnabled as setEnabledApi } from '../../api/ai-admin.ts';
import { miniIcon } from '../../components/mini-icons.ts';
import { miniSwitch } from '../../components/mini-switch.ts';
import { EFFORT_FAMILIES, PROVIDER_NAME, providerIcon, type EffortFamily } from '../../components/provider-icons.ts';
import {
  defaultUnusable, firstChoice, modelGroups, modelKind, parseRefKey, refKey, slotModel, slotNote, slotUsing, SLOTS,
  type ModelGroup,
} from '../../domain/ai-settings.ts';
import { knownDefault, levelForFamily } from '../../domain/effort-defaults.ts';
import { aiCodeText, errorText } from '../../i18n/errors.ts';
import { t, tn, type MsgKey } from '../../i18n/index.ts';
import { onRouteChange, refreshAi } from '../../state/actions.ts';
import { onClearUserData } from '../../state/store.ts';
import type { AiEffort, AiSettings, AiSlot, ModelRef } from '../../types/api.ts';
import { h, scrollToY, type Child } from '../../utils/dom.ts';
import { matches, TOUCH_MQ } from '../../utils/media.ts';
import { toast } from '../../utils/toast.ts';
import { aiDown } from './status.ts';
import { A, adopt, num, redraw } from './state.ts';

const KIND_LABEL: Record<NonNullable<ReturnType<typeof modelKind>>, MsgKey> = {
  rerank: 'ai.set.kind.rerank', embedding: 'ai.set.kind.embedding', guard: 'ai.set.kind.guard', image: 'ai.set.kind.image',
  audio: 'ai.set.kind.audio', parse: 'ai.set.kind.parse', vision: 'ai.set.kind.vision',
};

// ---------------------------------------------------------------- loading the merged list

/** Reads the models of every connection (the service caches them for 10 minutes; `refresh` lists again) */
export async function loadModels(refresh: boolean): Promise<void> {
  if (A.modelsBusy) return;
  A.modelsBusy = true;
  A.modelsError = null;
  redraw();
  try {
    A.models = await fetchModels(refresh);
  } catch (e) {
    if (!aiDown(e)) A.modelsError = t('ai.set.modelsErr', { reason: errorText(e) });
  }
  A.modelsBusy = false;
  redraw();
}

const labelOf = (r: ModelRef | null, s: AiSettings): string => {
  if (!r) return '';
  const name = s.connections.find((c) => c.id === r.connectionId)?.name ?? '';
  return name ? `${name} · ${r.model}` : r.model;
};

// ---------------------------------------------------------------- the model picker (a combobox over the merged list)

type Opt = { kind: 'follow' } | { kind: 'manual' } | { kind: 'model'; ref: ModelRef };

interface PickerOptions {
  id: string;
  /** What the field shows now ('' = nothing chosen) */
  text: string;
  /** A feature row: the first option is "Follow the default model" */
  follow: boolean;
  disabled: boolean;
  placeholder: string;
  onPick: (r: ModelRef | null) => void;
  onManual?: () => void;
  /** Only this connection's models (a model row: the connection is chosen in the picker on its left) */
  connectionId?: number;
  /** The accessible name of the field and its list (a row without a <label for>) */
  label?: string;
  /** The field lost the focus (after its own blur handling) */
  onLeave?: () => void;
}

/**
 * Typing filters the merged list by model name; the list is grouped by connection (the group header shows the name and a
 * problem chip); a tick marks a model that already passed a test. Keyboard: up / down / Enter / Escape.
 */
/** The field being focused by the page itself (after a pick): its list stays closed until the user asks for it */
let quietId: string | null = null;
/** Puts the focus on a model field without opening its list (the list would cover the rows below on a phone) */
function focusQuiet(id: string): void {
  const el = document.getElementById(id);
  if (!el) return;
  quietId = id;
  el.focus();
  quietId = null;
}

function modelPicker(s: AiSettings, o: PickerOptions): HTMLElement {
  let opts: Opt[] = [];
  let active = -1;
  let typed = false;
  const listId = `${o.id}-list`;
  const listEl = h('ul', { id: listId, class: 'ais-combo-list', role: 'listbox', 'aria-label': o.label ?? t('ai.set.pickModel') });
  listEl.hidden = true;
  const paint = (): void => {
    const one = o.connectionId != null;
    const groups: ModelGroup[] = modelGroups(A.models, s, typed ? input.value : '').filter((g) => !one || g.connectionId === o.connectionId);
    opts = [];
    const items: HTMLElement[] = [];
    const add = (opt: Opt, el: (i: number) => HTMLElement): void => {
      opts.push(opt);
      items.push(el(opts.length - 1));
    };
    const optEl = (i: number, ...kids: Child[]): HTMLElement => h('li', {
      id: `${o.id}-o${i}`, role: 'option', class: 'ais-combo-o' + (i === active ? ' on' : ''), 'aria-selected': String(i === active),
      // mousedown, not click: it comes before the field's blur, which closes the list
      onmousedown: (e: MouseEvent) => {
        e.preventDefault();
        choose(i);
      },
    }, ...kids);
    if (o.follow) add({ kind: 'follow' }, (i) => optEl(i, h('span', { class: 'ais-combo-id' }, t('ai.set.slot.follow'))));
    for (const g of groups) {
      const conn = s.connections.find((c) => c.id === g.connectionId);
      // One connection: its name is in the picker on the left, so the header only stays to show a problem
      if (!one || g.status === 'broken') items.push(h('li', { class: 'ais-combo-g', role: 'presentation' }, g.name,
        g.status === 'broken' && conn ? h('span', { class: 'ais-kind' }, t('ai.set.conn.statusBad', { reason: aiCodeText(conn.errorCode ?? 'ai.keyProblem') })) : null));
      for (const m of g.models) {
        const kind = modelKind(m.id);
        add({ kind: 'model', ref: { connectionId: g.connectionId, model: m.id } }, (i) => optEl(i,
          h('span', { class: 'ais-combo-id' + (kind ? ' other' : '') }, m.id),
          kind ? h('span', { class: 'ais-kind' }, t(KIND_LABEL[kind])) : null,
          m.inPool ? miniIcon('check', 'ais-tick') : null));
      }
    }
    if (!groups.some((g) => g.models.length)) items.push(h('li', { class: 'ais-combo-none', role: 'presentation' }, t('ai.set.noMatch')));
    if (o.onManual) add({ kind: 'manual' }, (i) => optEl(i, h('span', { class: 'ais-combo-id ais-manual' }, t('ai.set.slot.manual'))));
    listEl.replaceChildren(...items);
    if (active >= 0) input.setAttribute('aria-activedescendant', `${o.id}-o${active}`);
    else input.removeAttribute('aria-activedescendant');
    document.getElementById(`${o.id}-o${active}`)?.scrollIntoView({ block: 'nearest' });
  };
  function open(on: boolean): void {
    if (on) {
      // Above the field when less than 200 px are free below it (the on-screen keyboard counts: the visual viewport)
      const vv = window.visualViewport;
      const bottom = vv ? vv.offsetTop + vv.height : window.innerHeight;
      listEl.classList.toggle('up', bottom - input.getBoundingClientRect().bottom < 200);
    }
    listEl.hidden = !on;
    input.setAttribute('aria-expanded', String(on));
    if (on) paint();
  }
  function choose(i: number): void {
    const opt = opts[i];
    if (!opt) return;
    open(false);
    if (opt.kind === 'manual') o.onManual?.();
    else o.onPick(opt.kind === 'follow' ? null : opt.ref);
  }
  const input = h('input', {
    type: 'text', id: o.id, class: 'ais-model', value: o.text, maxlength: '200', role: 'combobox', 'aria-autocomplete': 'list',
    'aria-expanded': 'false', 'aria-controls': listId, placeholder: o.placeholder || t('ai.set.modelFilter'), autocomplete: 'off',
    spellcheck: 'false', disabled: o.disabled, 'aria-label': o.label ?? null,
    onfocus: () => {
      typed = false;
      active = -1;
      input.select();
      // Focused by the page after a pick: no scrolling, no list (a click, a tap or a key opens it)
      if (quietId === o.id) return;
      // On touch the field goes near the top of the visual viewport (below the top bar), so the list and the keyboard fit
      if (matches(TOUCH_MQ)) {
        const nudge = (): void => {
          if (!input.isConnected) return;
          const top = (window.visualViewport?.offsetTop ?? 0) + (parseFloat(getComputedStyle(document.documentElement).getPropertyValue('--top-h')) || 0) + 12;
          const delta = input.getBoundingClientRect().top - top;
          if (Math.abs(delta) > 4) scrollToY(window.scrollY + delta, false);
        };
        nudge();
        window.setTimeout(nudge, 300);
      }
      open(true);
    },
    onblur: () => {
      open(false);
      input.value = o.text;
      o.onLeave?.();
    },
    // A click or tap on the field that already has the focus (its list is closed after a pick or Escape)
    onclick: () => {
      if (listEl.hidden) open(true);
    },
    oninput: () => {
      typed = true;
      active = -1;
      open(true);
    },
    onkeydown: (e: KeyboardEvent) => {
      if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
        e.preventDefault();
        if (listEl.hidden) open(true);
        if (!opts.length) return;
        active = e.key === 'ArrowDown' ? Math.min(opts.length - 1, active + 1) : Math.max(0, active - 1);
        paint();
      } else if (e.key === 'Enter' && !listEl.hidden && opts[active]) {
        e.preventDefault();
        choose(active);
      } else if (e.key === 'Escape' && !listEl.hidden) {
        e.preventDefault();
        open(false);
      }
    },
  });
  return h('div', { class: 'ais-combo' }, input, listEl);
}

// ---------------------------------------------------------------- picking a model for a row

/** The connection chosen on the left of a row whose model is not picked yet (the row itself has not changed) */
const pendingConn: Partial<Record<AiSlot, number>> = {};
/** The pick being tested (null = follow the default): the row shows it until the answer is in */
const picking: Partial<Record<AiSlot, ModelRef | null>> = {};

/** Drops every half-made choice (also called when another settings category is chosen: the rows leave the page) */
export function forgetPending(): void {
  for (const k of SLOTS) delete pendingConn[k];
}
// A half-made choice never outlives the page (entering, leaving, sign-out)
onRouteChange((from, to) => {
  if ((from.name === 'aiSettings') !== (to.name === 'aiSettings')) forgetPending();
});
onClearUserData(forgetPending);

/** The focus left a row whose connection was chosen but no model: the row shows its saved value again */
function dropPending(slot: AiSlot): void {
  const s = A.settings;
  if (pendingConn[slot] == null || A.slotBusy[slot] || A.manualOpen === slot) return;
  const li = document.getElementById(`ais-slot-${slot}`);
  if (li?.contains(document.activeElement)) return;
  delete pendingConn[slot];
  // Only this row is rebuilt: a full redraw could swallow the click that took the focus away
  if (li && s) li.replaceWith(slotRow(s, slot));
}

async function pickSlot(slot: AiSlot, ref: ModelRef | null, manual = false): Promise<void> {
  const s = A.settings;
  if (!s || A.slotBusy[slot]) return;
  if (!ref && slot === 'default') return;
  picking[slot] = ref;
  const was = slotUsing(s, slot);
  A.slotBusy[slot] = true;
  A.slotMsg[slot] = null;
  A.manualOpen = null;
  redraw();
  try {
    const body = ref
      ? { slot, connectionId: ref.connectionId, model: ref.model, manual }
      : { slot: slot as Exclude<AiSlot, 'default'>, clear: true as const };
    const r = await pickModel(body);
    adopt(r.settings);
    if (!ref) A.slotMsg[slot] = null;
    else if (r.ok) A.slotMsg[slot] = { ok: true, text: t('ai.set.slot.switched', { ms: num(r.latencyMs ?? 0) }) };
    else A.slotMsg[slot] = { ok: false, text: t('ai.set.slot.failed', { reason: aiCodeText(r.code ?? 'ai.internal'), model: was?.model ?? t('ai.set.card.model.none') }) };
    void refreshAi(true);
  } catch (e) {
    if (!aiDown(e)) A.slotMsg[slot] = { ok: false, text: errorText(e) };
  }
  A.slotBusy[slot] = false;
  delete picking[slot];
  delete pendingConn[slot];
  redraw();
  // A row that follows the default has no model field to focus (it is disabled): the connection picker takes the focus
  // The list of the model field stays closed (it would cover the rows below on a phone)
  const field = document.getElementById(`ai-slot-${slot}`) as HTMLInputElement | null;
  if (field && !field.disabled) focusQuiet(field.id);
  else document.getElementById(`ai-slot-${slot}-conn`)?.focus();
}

/** "Type a model name" for the connection chosen on the left (for a service that lists no models) */
function manualRow(slot: AiSlot, connId: number, connName: string): HTMLElement {
  const input = h('input', {
    type: 'text', id: `ai-slot-${slot}-name`, class: 'ais-model', maxlength: '200', autocomplete: 'off', spellcheck: 'false',
    placeholder: t('ai.set.slot.manual'), 'aria-label': `${connName} · ${t('ai.set.slot.manual')}`,
  });
  const go = (): void => {
    const model = input.value.trim();
    if (model && connId) void pickSlot(slot, { connectionId: connId, model }, true);
  };
  input.addEventListener('keydown', (e) => {
    if (e.key === 'Enter') {
      e.preventDefault();
      go();
    }
  });
  return h('div', { class: 'ais-inline ais-manual-row' }, input,
    h('button', { class: 'btn primary', type: 'button', onclick: go }, t('ai.set.save')),
    h('button', { class: 'btn', type: 'button', onclick: () => {
      A.manualOpen = null;
      redraw();
      focusQuiet(`ai-slot-${slot}`);
    } }, t('common.cancel')));
}

// ---------------------------------------------------------------- step 2: turn AI+ on

async function enable(on: boolean, def?: ModelRef): Promise<void> {
  const s = A.settings;
  if (!s || A.enableBusy) return;
  A.enableBusy = true;
  A.enableMsg = null;
  redraw();
  try {
    const r = await setEnabledApi(on, def);
    adopt(r.settings);
    if (r.ok) toast(t(on ? 'ai.set.enabled' : 'ai.set.disabled'));
    else A.enableMsg = { ok: false, text: t('ai.set.testFail', { reason: aiCodeText(r.code ?? 'ai.internal') }) };
  } catch (e) {
    if (!aiDown(e)) A.enableMsg = { ok: false, text: errorText(e) };
  }
  A.enableBusy = false;
  await refreshAi(true);
  redraw();
  document.getElementById('ai-enable')?.focus();
}

export function enableSection(s: AiSettings): HTMLElement {
  const def = slotModel(s, 'default');
  const msg = A.enableMsg ? h('p', { class: A.enableMsg.ok ? 'ais-ok' : 'ais-ferr', role: A.enableMsg.ok ? 'status' : 'alert' }, A.enableMsg.text) : null;
  let body: Child;
  if (def) {
    // The default model exists: AI+ is a plain switch
    body = h('div', { class: 'ais-enable' },
      h('span', { class: 'ais-enable-l', id: 'ai-enable-l' }, t('ai.set.enable')),
      miniSwitch({
        id: 'ai-enable', checked: s.enabled, labelledBy: 'ai-enable-l', disabled: A.enableBusy, onToggle: (v) => void enable(v),
      }),
      s.enabled ? h('span', { class: 'ais-enable-v' }, t('ai.set.enable.on', { conn: def.connectionName, model: def.model })) : null);
  } else {
    const none = !s.connections.length;
    if (!A.enableChoice) {
      const first = firstChoice(A.models, s);
      if (first) A.enableChoice = refKey(first);
    }
    const choice = parseRefKey(A.enableChoice);
    body = h('div', null,
      h('div', { class: 'ais-inline' },
        h('div', { class: 'fld ais-enable-pick' },
          h('label', { class: 'fld-l', for: 'ai-enable-model' }, t('ai.set.slot.default')),
          modelPicker(s, {
            id: 'ai-enable-model', text: labelOf(choice, s), follow: false, disabled: none || A.enableBusy, placeholder: '',
            onPick: (r) => {
              A.enableChoice = r ? refKey(r) : '';
              redraw();
              focusQuiet('ai-enable-model');
            },
          })),
        h('button', {
          class: 'btn primary', id: 'ai-enable', type: 'button', disabled: none || !choice || A.enableBusy,
          'aria-busy': A.enableBusy ? 'true' : null, onclick: () => choice && void enable(true, choice),
        }, A.enableBusy ? t('ai.set.testing') : t('ai.set.enable.first'))),
      none ? h('p', { class: 'ais-note' }, t('ai.set.enable.needConn')) : null);
  }
  return h('section', { class: 'panel ais-sec', 'aria-labelledby': 'ais-enable-h' },
    h('h2', { id: 'ais-enable-h', tabindex: '-1' }, t('ai.set.card.main')), body, msg);
}

// ---------------------------------------------------------------- thinking strength (v1.6)

const LEVEL_KEYS = new Set(['none', 'minimal', 'low', 'medium', 'high', 'xhigh', 'max']);

/** Saves one slot's thinking strength at once (null = default, nothing sent); the row is busy meanwhile */
async function saveEffort(slot: AiSlot, effort: AiEffort | null, focusId: string): Promise<void> {
  if (A.slotBusy[slot]) return;
  A.slotBusy[slot] = true;
  A.slotMsg[slot] = null;
  redraw();
  try {
    const r = await setEffort({ slot, effort });
    adopt(r.settings);
  } catch (e) {
    if (!aiDown(e)) A.slotMsg[slot] = { ok: false, text: errorText(e) };
  }
  A.slotBusy[slot] = false;
  redraw();
  document.getElementById(focusId)?.focus();
}

/** The row above a slot's pickers: [Default] [Anthropic] [OpenAI] [Google] [Responses API] [Gemma] and the level list; hidden on an older ai-service */
function effortRow(s: AiSettings, slot: AiSlot, busy: boolean, slotName: string): HTMLElement | null {
  const lists = s.effortLevels;
  if (!lists) return null;
  const cur = s.slots[slot].effort ?? null;
  const fam = cur && lists[cur.family] ? cur.family : null;
  const lvl = fam ? cur?.level ?? '' : '';
  // Google levels show as written (uppercase); Gemma's two get a short hint; the others use the level keys
  const lvlLabel = (x: string, family?: string): string =>
    family === 'gemma' && (x === 'HIGH' || x === 'MINIMAL') ? t(`ai.set.effort.level.gemma.${x}` as MsgKey) : LEVEL_KEYS.has(x) ? t(`ai.set.effort.level.${x}` as MsgKey) : x;
  const id = `ai-slot-${slot}-effort`;
  const dflt = knownDefault(slotUsing(s, slot)?.model);
  const btn = (key: EffortFamily | 'default', pressed: boolean, label: Child[], name: string, title: string, onClick: () => void): HTMLElement =>
    h('button', {
      type: 'button', id: `${id}-${key}`, class: 'ais-eb' + (pressed ? ' on' : ''), 'aria-pressed': pressed ? 'true' : 'false',
      'aria-label': name, title, disabled: busy, onclick: onClick,
    }, ...label);
  const buttons = [
    btn('default', fam === null, [t('ai.set.effort.default')], t('ai.set.effort.default'), t('ai.set.effort.defaultTip'),
      () => void saveEffort(slot, null, `${id}-default`)),
    ...EFFORT_FAMILIES.map((f) => btn(f, fam === f,
      [providerIcon(f)],
      PROVIDER_NAME[f], PROVIDER_NAME[f],
      () => void saveEffort(slot, { family: f, level: levelForFamily(cur?.level, lists[f] ?? []) }, `${id}-${f}`))),
  ];
  const select = h('select', {
    id: `${id}-level`, class: 'ais-select ais-effort-level', 'aria-label': t('ai.set.effort.levelOf', { slot: slotName }),
    disabled: busy || fam === null,
    onchange: (e: Event) => {
      if (fam) void saveEffort(slot, { family: fam, level: (e.target as HTMLSelectElement).value }, `${id}-level`);
    },
  }, fam === null
    ? h('option', { value: '' }, '')
    : (lists[fam] ?? []).map((x) => h('option', { value: x, title: x }, lvlLabel(x, fam))));
  if (fam) (select as HTMLSelectElement).value = lvl;
  return h('div', { class: 'ais-effort' },
    h('div', { class: 'ais-effort-b', role: 'group', 'aria-label': t('ai.set.effort.label') }, buttons),
    dflt ? h('span', { class: 'ais-effort-hint' }, t('ai.set.effort.modelDefault', { level: lvlLabel(dflt, /gemma/i.test(slotUsing(s, slot)?.model ?? '') ? 'gemma' : undefined) })) : null,
    select);
}

// ---------------------------------------------------------------- step 3: the six model rows

function slotRow(s: AiSettings, slot: AiSlot): HTMLElement {
  const own = slotModel(s, slot);
  const busy = A.slotBusy[slot] === true;
  const note = slotNote(s, slot);
  const msg = A.slotMsg[slot];
  const bad = slot === 'default' && defaultUnusable(s);
  const labelId = `ai-slot-${slot}-l`;
  const name = t(`ai.set.slot.${slot}` as MsgKey);
  const canFollow = slot !== 'default';
  // The connection shown on the left: the one being chosen, else the row's own, else "follow" (the default row: none yet)
  const pend = pendingConn[slot];
  const pendOk = pend != null && s.connections.some((c) => c.id === pend);
  // While a pick is tested the row shows that pick (not the saved value), so it does not look dropped
  const pick = busy ? picking[slot] : undefined;
  const connId = pick !== undefined ? pick?.connectionId ?? 0 : pendOk ? pend : own?.connectionId ?? 0;
  const conn = s.connections.find((c) => c.id === connId);
  // The row's own model, unless another connection was just chosen on the left (then: "Choose a model…")
  const text = pick ? pick.model : own && own.connectionId === connId ? own.model : '';
  const def = slotModel(s, 'default');
  const onConn = (): void => {
    const v = Number(sel.value);
    A.manualOpen = null;
    if (!v) {
      // "Follow the default model" switches at once (nothing to test)
      delete pendingConn[slot];
      if (own) void pickSlot(slot, null);
      else redraw();
      return;
    }
    if (own?.connectionId === v) delete pendingConn[slot];
    else pendingConn[slot] = v;
    A.slotMsg[slot] = null;
    redraw();
    // The models of the chosen connection open right away
    document.getElementById(`ai-slot-${slot}`)?.focus();
  };
  const sel = h('select', {
    id: `ai-slot-${slot}-conn`, class: 'ais-select ais-slot-conn', 'aria-label': t('ai.set.slot.connOf', { slot: name }), disabled: busy,
    onchange: onConn,
    // The focus left the row through this picker with no model picked (after the blur has settled, as on the right)
    onblur: () => window.setTimeout(() => dropPending(slot), 0),
  },
    canFollow ? h('option', { value: '0' }, t('ai.set.slot.follow')) : null,
    !canFollow && !connId ? h('option', { value: '0', disabled: true, hidden: true }, t('ai.set.slot.pickConn')) : null,
    s.connections.map((c) => h('option', { value: String(c.id) },
      c.status === 'broken' ? t('ai.set.slot.connBad', { name: c.name }) : c.name)));
  sel.value = String(connId);
  const picker = conn
    ? modelPicker(s, {
      id: `ai-slot-${slot}`, text, follow: false, disabled: busy, placeholder: t('ai.set.slot.pickModel'), connectionId: conn.id,
      label: t('ai.set.slot.modelOf', { slot: name }),
      onPick: (r) => void pickSlot(slot, r),
      // After the blur has settled: the focus may only have moved inside the row
      onLeave: pendOk ? () => window.setTimeout(() => dropPending(slot), 0) : undefined,
      onManual: () => {
        A.manualOpen = slot;
        redraw();
        document.getElementById(`ai-slot-${slot}-name`)?.focus();
      },
    })
    // Following the default (or no connection chosen yet): nothing to pick; the default's model shows, muted
    : h('div', { class: 'ais-combo' }, h('input', {
      type: 'text', id: `ai-slot-${slot}`, class: 'ais-model ais-model-follow', disabled: true,
      value: canFollow ? labelOf(def ? { connectionId: def.connectionId, model: def.model } : null, s) : '',
      placeholder: canFollow ? '' : t('ai.set.slot.pickModel'), 'aria-label': t('ai.set.slot.modelOf', { slot: name }),
    }));
  return h('li', { class: 'ais-slot' + (note?.kind === 'noRoute' || bad ? ' warn' : ''), id: `ais-slot-${slot}` },
    h('span', { class: 'ais-slot-l', id: labelId }, name),
    effortRow(s, slot, busy, name),
    h('div', { class: 'ais-slot-pick', role: 'group', 'aria-labelledby': labelId }, sel, picker),
    busy ? h('p', { class: 'ais-note ais-slot-s', role: 'status' }, t(pick === null ? 'ai.set.slot.switching' : 'ai.set.slot.testing')) : null,
    msg ? h('p', { class: (msg.ok ? 'ais-ok' : 'ais-ferr') + ' ais-slot-s', role: msg.ok ? 'status' : 'alert' }, msg.text) : null,
    A.manualOpen === slot && conn ? manualRow(slot, conn.id, conn.name) : null,
    bad && own ? h('p', { class: 'ais-warn ais-slot-n' }, t('ai.set.card.model.bad', { conn: own.connectionName, model: own.model })) : null,
    note?.kind === 'fallback' ? h('p', { class: 'ais-note ais-slot-n' }, t('ai.set.slot.fallback', { conn: note.conn })) : null,
    note?.kind === 'noRoute' && !bad ? h('p', { class: 'ais-warn ais-slot-n' }, t('ai.set.slot.noRoute')) : null);
}

export function modelsSection(s: AiSettings): HTMLElement {
  let status: Child = null;
  if (A.modelsBusy) status = h('p', { class: 'ais-note', role: 'status' }, t('ai.set.detecting'));
  else if (A.modelsError) status = h('p', { class: 'ais-ferr', role: 'alert' }, A.modelsError);
  else if (A.models) {
    const n = A.models.connections.reduce((a, c) => a + c.models.length, 0);
    status = h('p', { class: 'ais-note', role: 'status' }, n ? tn('ai.set.models', n) : t('ai.set.noList'));
  }
  return h('section', { class: 'panel ais-sec', 'aria-labelledby': 'ais-models-h' },
    h('div', { class: 'ais-sechead' },
      h('h2', { id: 'ais-models-h', tabindex: '-1' }, t('ai.set.sec.model')),
      h('button', {
        class: 'btn', id: 'ai-models-refresh', type: 'button', disabled: A.modelsBusy || !s.connections.length, onclick: () => void loadModels(true),
      }, t('ai.set.models.refresh'))),
    s.connections.length
      ? h('ul', { class: 'ais-slots' }, SLOTS.map((k) => slotRow(s, k)))
      : h('p', { class: 'ais-note' }, t('ai.set.enable.needConn')),
    status);
}
