// The list of signed-in devices (v3.1): Settings (own devices) and User management (one user's devices, administrators).
// One row per device: name, state tag, notifications tag, the facts, a "full device information" toggle and, for a device that
// can be signed out, a 44 px icon button. An `expired` row's action is the owner's "Stop" (D20): the same sign-out, other words.
// The row asks first; `onSignOut` runs only after the confirmation. Administrators see the notification state read-only.
import '../styles/devices.css';
import { iconButton } from './icon-button.ts';
import { deviceAction, deviceName, deviceStateKey, sortDevices } from '../domain/devices.ts';
import { countryNameIn, placeText } from '../domain/geo.ts';
import { fullAt } from '../domain/user-admin.ts';
import { t, uiLang } from '../i18n/index.ts';
import type { Device } from '../types/api.ts';
import { ask } from '../utils/dialog.ts';
import { h } from '../utils/dom.ts';

export interface DeviceListOptions {
  devices: Device[];
  /** Whether the IP and place lines are shown (the administrator's "sign-in information" switch) */
  signinInfo: boolean;
  /** `self`: the person's own list; `admin`: one user's list seen by an administrator */
  mode: 'self' | 'admin';
  /** The row whose sign-out runs now (its button is disabled); default null */
  busyId?: number | null;
  /** Called after the confirmation */
  onSignOut: (d: Device) => void;
}

/** Rows whose "Full device information" is open (kept across redraws of the page) */
const uaOpen = new Set<number>();

function nameOf(d: Device): string {
  return deviceName(d, t('devices.app'), t('devices.unknown'));
}

async function confirmSignOut(d: Device, onSignOut: (d: Device) => void): Promise<void> {
  const stop = deviceAction(d) === 'stop';
  const name = nameOf(d);
  const yes = await ask({
    title: t(stop ? 'devices.stop.confirmTitle' : 'devices.signOut.confirmTitle'),
    body: t(stop ? 'devices.stop.confirmBody' : 'devices.signOut.confirmBody', { name }),
    ok: t(stop ? 'devices.stop.ok' : 'devices.signOut.ok'),
    cancel: t('common.cancel'),
    danger: true,
  });
  if (yes) onSignOut(d);
}

function meta(...parts: Array<string | HTMLElement>): HTMLElement {
  return h('p', { class: 'dv-meta' }, parts);
}

function facts(d: Device, signinInfo: boolean): HTMLElement[] {
  const country = countryNameIn(uiLang());
  const lines: HTMLElement[] = [meta(t('devices.activeAt', { at: fullAt(d.activeAt) }))];
  if (d.signinAt) lines.push(meta(t('devices.signinAt', { at: fullAt(d.signinAt) })));
  if (signinInfo) {
    lines.push(
      meta(t('devices.signinIp'), h('span', { class: 'dv-ip' }, d.signinIp ?? '—')),
      meta(t('devices.signinPlace'), placeText(d.signinLocation, country) ?? t('devices.placeUnknown')),
      meta(t('devices.activeIp'), h('span', { class: 'dv-ip' }, d.activeIp ?? '—')),
      meta(t('devices.activePlace'), placeText(d.activeLocation, country) ?? t('devices.placeUnknown')),
    );
  }
  return lines;
}

function uaToggle(d: Device): HTMLElement[] {
  const id = `dv-ua-${d.id}`;
  const detail = h('dl', { class: 'dv-ua', id, hidden: !uaOpen.has(d.id) }, h('dt', null, t('act.detail.ua')), h('dd', null, d.ua ?? '—'));
  const btn = h('button', {
    type: 'button', class: 'dv-ua-btn', 'aria-expanded': String(uaOpen.has(d.id)), 'aria-controls': id,
    onclick: () => {
      const open = !uaOpen.has(d.id);
      if (open) uaOpen.add(d.id);
      else uaOpen.delete(d.id);
      btn.setAttribute('aria-expanded', String(open));
      detail.hidden = !open;
    },
  }, t('devices.full'));
  return [btn, detail];
}

function row(d: Device, o: DeviceListOptions): HTMLElement {
  const action = deviceAction(d);
  const name = nameOf(d);
  const out = action === 'none'
    ? null
    : iconButton({
      icon: 'exit', tone: 'danger',
      label: t(action === 'stop' ? 'devices.stopAria' : 'devices.signOutAria', { name }),
      onClick: () => void confirmSignOut(d, o.onSignOut),
    });
  if (out) {
    out.classList.add('dv-out');
    out.dataset.deviceId = String(d.id);
    out.disabled = o.busyId === d.id;
  }
  return h('li', { class: 'dv-row' + (d.state === 'signedOut' ? ' off' : ''), 'data-device': String(d.id), 'aria-busy': o.busyId === d.id ? 'true' : null },
    h('div', { class: 'dv-main' },
      h('div', { class: 'dv-line' },
        h('span', { class: 'dv-name' }, name),
        h('span', { class: 'dv-tags' },
          d.current ? h('span', { class: 'tag k' }, t('devices.thisDevice')) : null,
          h('span', { class: 'tag' + (d.state === 'expired' ? ' r' : '') }, t(deviceStateKey(d))),
          h('span', { class: 'tag' + (d.push ? ' g' : '') }, t(d.push ? 'devices.push.on' : 'devices.push.off')))),
      facts(d, o.signinInfo),
      uaToggle(d)),
    out ? h('div', { class: 'dv-act' }, out) : null);
}

/** `ul.dv-list` with one `li.dv-row` per device (see the file header); an empty list says so */
export function deviceList(o: DeviceListOptions): HTMLElement {
  if (o.devices.length === 0) return h('p', { class: 'dv-empty set-note' }, t('devices.empty'));
  return h('ul', { class: 'dv-list', 'data-mode': o.mode }, sortDevices(o.devices).map((d) => row(d, o)));
}
