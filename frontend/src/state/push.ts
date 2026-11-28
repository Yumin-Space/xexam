// Web push and the installed app, client side (v3.1). This is the only module that talks to navigator.serviceWorker,
// PushManager and Notification. It keeps what this device knows (permission, subscription, the server's key, the install
// prompt) and tells the pages when it changes (`onPushChange`). The pure rules (what state a device is in, which guide step
// shows) live in domain/push-env.ts.
import { sayHello } from '../api/devices.ts';
import { ApiError } from '../api/client.ts';
import { fetchPushConfig, sendPushTest, subscribePush } from '../api/push.ts';
import { detectEnv, devicePushState, type DevicePushState, type GuideInputs, type PushEnv } from '../domain/push-env.ts';
import type { Device, PushConfig, PushPermission, PushTestResponse } from '../types/api.ts';
import { PHONE_MQ, STANDALONE_MQ, TOUCH_MQ, matches } from '../utils/media.ts';
import { noteDiscarded } from './saved-marker.ts';
import { loadSaved, storedSessionKey } from './session-storage.ts';
import { guideTestKey, paths, pushConfirmKey, storageOwner } from './storage-keys.ts';
import { S } from './store.ts';

export interface PushView { env: PushEnv; permission: PushPermission; subscribed: boolean; accountOn: boolean;
  courier: boolean | null; serverKey: boolean; device: Device | null; installPrompt: boolean; busy: boolean; installed: boolean }

/** The browser's install prompt event (not in the DOM typings) */
interface InstallPromptEvent extends Event {
  prompt(): Promise<void>;
  userChoice: Promise<{ outcome: 'accepted' | 'dismissed' }>;
}

// ---------------------------------------------------------------- state

const listeners = new Set<() => void>();
/** The server's push key and courier state (null = not read yet, or reading failed) */
let cfg: PushConfig | null = null;
let cfgLoading: Promise<PushConfig | null> | null = null;
/** When reading the config failed last (ms); 0 = it did not. A failed read is tried again after 30 s */
let cfgFailedAt = 0;
let device: Device | null = null;
/** The browser holds a push subscription for this site */
let browserSub = false;
/** The server holds this device's subscription (the answers of hello / subscribe say so) */
let serverSub = false;
let busy = false;
let deferred: InstallPromptEvent | null = null;
/** The first hello and subscription check of this sign-in are done: pages may judge the device now */
let ready = false;
/** The guide card was shown in this page load (rule 8 of guideStep) */
let inFlow = false;
/** The guide was opened from Settings: it shows in full until it is closed */
let reopened = false;
/** Bumped by resetPushState: an answer that arrives after a sign-out is dropped */
let generation = 0;
let started = false;
let helloAt = 0;
let helloPerm: PushPermission | null = null;
let seenPerm: PushPermission | null = null;

function emit(): void {
  for (const fn of [...listeners]) {
    try {
      fn();
    } catch {
      /* a listener must not stop the others */
    }
  }
}

export function onPushChange(fn: () => void): () => void {
  listeners.add(fn);
  return () => void listeners.delete(fn);
}

// ---------------------------------------------------------------- the environment

function permission(): PushPermission {
  if (typeof Notification === 'undefined') return 'unsupported';
  const p = Notification.permission;
  return p === 'granted' || p === 'denied' ? p : 'default';
}

function currentEnv(): PushEnv {
  const nav = typeof navigator !== 'undefined' ? navigator : undefined;
  const win = typeof window !== 'undefined' ? window : undefined;
  return detectEnv({
    ua: nav?.userAgent ?? '',
    maxTouchPoints: nav?.maxTouchPoints ?? 0,
    standalone: matches(STANDALONE_MQ) || (nav as { standalone?: boolean } | undefined)?.standalone === true,
    secure: win?.isSecureContext === true,
    sw: !!nav && 'serviceWorker' in nav,
    push: !!win && 'PushManager' in win && 'Notification' in win,
    coarse: matches(TOUCH_MQ),
    phoneWidth: matches(PHONE_MQ),
  });
}

/** The server's key is there, or not read yet (judged optimistically), but not when reading it failed */
function serverKeyNow(): boolean {
  return cfg ? cfg.publicKey != null : cfgFailedAt === 0;
}

function stateNow(env: PushEnv = currentEnv()): DevicePushState {
  return devicePushState(env, permission(), browserSub && serverSub, S.settings.push, serverKeyNow());
}

/** Reads the server's push key. `refresh` forgets what was read before. Never throws: null when it could not be read. */
function getConfig(refresh: boolean): Promise<PushConfig | null> {
  if (!refresh && cfg) return Promise.resolve(cfg);
  if (cfgLoading) return cfgLoading;
  const gen = generation;
  const p: Promise<PushConfig | null> = fetchPushConfig().then((c) => {
    if (gen === generation) {
      cfg = c;
      cfgFailedAt = 0;
      emit();
    }
    return c;
  }, () => {
    if (gen === generation) {
      cfgFailedAt = Date.now();
      emit();
    }
    return null;
  }).finally(() => {
    if (cfgLoading === p) cfgLoading = null;
  });
  cfgLoading = p;
  return p;
}

export function pushView(): PushView {
  const env = currentEnv();
  // The answer of the server is prefetched when something asks for the view (the permission prompt must be the first thing a
  // tap does, so nothing may have to be fetched before it)
  if (S.user && env.secure && env.sw && !cfg && !cfgLoading && Date.now() - cfgFailedAt > 30_000) void getConfig(false);
  return {
    env,
    permission: permission(),
    subscribed: browserSub && serverSub,
    accountOn: S.settings.push,
    courier: cfg ? cfg.courier : null,
    serverKey: serverKeyNow(),
    device,
    installPrompt: deferred != null,
    busy,
    installed: readGuide().installed,
  };
}

/** The first hello and subscription check of this sign-in are done (a page must not judge the device before) */
export function pushReady(): boolean {
  return ready;
}

// ---------------------------------------------------------------- install prompt, service worker

export function captureInstallPrompt(): void {
  if (typeof window === 'undefined') return;
  window.addEventListener('beforeinstallprompt', (e) => {
    e.preventDefault();
    deferred = e as InstallPromptEvent;
    emit();
  });
  window.addEventListener('appinstalled', () => {
    deferred = null;
    writeGuide({ installed: true });
    emit();
  });
}

export function registerServiceWorker(): void {
  if (typeof navigator === 'undefined' || typeof window === 'undefined') return;
  if (!window.isSecureContext || !('serviceWorker' in navigator)) return;
  navigator.serviceWorker.register('/sw.js', { scope: '/', updateViaCache: 'none' }).catch(() => {
    /* push stays unavailable; the app itself does not need the worker */
  });
  // A tapped notification asks the open window to go to its page
  navigator.serviceWorker.addEventListener('message', (e: MessageEvent) => {
    const d = e.data as { type?: unknown; url?: unknown } | null;
    if (!d || d.type !== 'xexam.open' || typeof d.url !== 'string') return;
    window.location.hash = d.url.startsWith('/#/') ? d.url.slice(1) : '#/';
  });
}

/** The registration with an active worker, or null (no worker support, or it did not become ready in time) */
async function registration(): Promise<ServiceWorkerRegistration | null> {
  if (typeof navigator === 'undefined' || !('serviceWorker' in navigator)) return null;
  try {
    const timeout = new Promise<null>((resolve) => window.setTimeout(() => resolve(null), 8000));
    return await Promise.race([navigator.serviceWorker.ready, timeout]);
  } catch {
    return null;
  }
}

export async function promptInstall(): Promise<'accepted' | 'dismissed' | 'unavailable'> {
  const ev = deferred;
  if (!ev) return 'unavailable';
  // The browser allows the prompt once per captured event
  deferred = null;
  try {
    await ev.prompt();
    const choice = await ev.userChoice;
    if (choice.outcome === 'accepted') writeGuide({ installed: true });
    emit();
    return choice.outcome === 'accepted' ? 'accepted' : 'dismissed';
  } catch {
    emit();
    return 'unavailable';
  }
}

// ---------------------------------------------------------------- keys and subscription

function b64urlToBytes(s: string): Uint8Array {
  const t = s.replace(/-/g, '+').replace(/_/g, '/');
  const bin = atob(t + '='.repeat((4 - (t.length % 4)) % 4));
  const out = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) out[i] = bin.charCodeAt(i);
  return out;
}

function bytesToB64url(buf: ArrayBuffer | null): string {
  if (!buf) return '';
  const bytes = new Uint8Array(buf);
  let s = '';
  for (const b of bytes) s += String.fromCharCode(b);
  return btoa(s).replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/, '');
}

function sameKey(sub: PushSubscription, key: Uint8Array): boolean {
  const k = sub.options?.applicationServerKey;
  if (!k) return false;
  const a = new Uint8Array(k);
  return a.length === key.length && a.every((v, i) => v === key[i]);
}

interface Confirmed { e: string; t: number }

function readConfirmed(): Confirmed | null {
  const uid = storageOwner();
  if (uid == null) return null;
  try {
    const d: unknown = JSON.parse(localStorage.getItem(pushConfirmKey(uid)) ?? 'null');
    if (d && typeof d === 'object' && typeof (d as Confirmed).e === 'string' && typeof (d as Confirmed).t === 'number') return d as Confirmed;
  } catch {
    /* none */
  }
  return null;
}

function writeConfirmed(endpoint: string): void {
  const uid = storageOwner();
  if (uid == null) return;
  try {
    localStorage.setItem(pushConfirmKey(uid), JSON.stringify({ e: endpoint, t: Math.floor(Date.now() / 1000) }));
  } catch {
    /* the next start confirms again */
  }
}

const DAY_S = 86400;

/**
 * Makes sure the browser holds a subscription for the server's current key and the server knows it. Needs the permission.
 * `force` confirms to the server even when it was confirmed recently. Throws on failure (callers decide what to show).
 */
async function ensureSubscription(force: boolean, retry = true): Promise<void> {
  const gen = generation;
  const config = await getConfig(false);
  if (!config?.publicKey) throw new Error('no push key');
  const reg = await registration();
  if (!reg) throw new Error('no service worker');
  const key = b64urlToBytes(config.publicKey);
  let sub = await reg.pushManager.getSubscription();
  let fresh = false;
  // A subscription made for another server key can never receive anything: replace it
  if (sub && !sameKey(sub, key)) {
    await sub.unsubscribe();
    sub = null;
  }
  if (!sub) {
    sub = await reg.pushManager.subscribe({ userVisibleOnly: true, applicationServerKey: key as BufferSource });
    fresh = true;
  }
  if (gen !== generation) return;
  browserSub = true;
  // At most one confirmation a day per user, unless the subscription is new or the caller insists
  const c = readConfirmed();
  if (!force && !fresh && serverSub && c && c.e === sub.endpoint && Date.now() / 1000 - c.t < DAY_S) {
    emit();
    return;
  }
  try {
    const r = await subscribePush({
      endpoint: sub.endpoint,
      keys: { p256dh: bytesToB64url(sub.getKey('p256dh')), auth: bytesToB64url(sub.getKey('auth')) },
      expirationTime: sub.expirationTime ?? null,
      keyId: config.keyId ?? '',
    });
    if (gen !== generation) return;
    device = r.device;
    serverSub = r.device.push || serverSub;
    writeConfirmed(sub.endpoint);
    emit();
  } catch (e) {
    // The server's key changed: read it again once and start over with the new one
    if (e instanceof ApiError && e.code === 'push.staleKey' && retry) {
      cfg = null;
      await getConfig(true);
      return ensureSubscription(true, false);
    }
    throw e;
  }
}

/** Reads the permission and the subscription of this browser (no network); tells the pages when something changed */
async function refreshLocal(): Promise<void> {
  const gen = generation;
  const perm = permission();
  let has = false;
  if (perm === 'granted') {
    const reg = await registration();
    try {
      has = reg ? (await reg.pushManager.getSubscription()) != null : false;
    } catch {
      has = false;
    }
  }
  if (gen !== generation) return;
  const changed = has !== browserSub || perm !== seenPerm;
  browserSub = has;
  seenPerm = perm;
  // The user changed the permission in the browser's settings: the server hears about it
  if (helloPerm != null && perm !== helloPerm) await hello();
  if (changed) emit();
}

/** Subscribes silently when everything allows it (permission already given, the account switch on); failures are silent */
async function maybeSubscribe(force: boolean): Promise<void> {
  if (stateNow() !== 'off' && stateNow() !== 'on') return;
  if (permission() !== 'granted') return;
  try {
    await ensureSubscription(force);
  } catch {
    /* the next start tries again */
  }
}

// ---------------------------------------------------------------- hello and start

function zoneName(): string | null {
  try {
    const z = Intl.DateTimeFormat().resolvedOptions().timeZone;
    return typeof z === 'string' && z.length > 0 && z.length <= 64 && /^[A-Za-z0-9_+\-/]+$/.test(z) ? z : null;
  } catch {
    return null;
  }
}

async function hello(): Promise<void> {
  const gen = generation;
  const perm = permission();
  try {
    const r = await sayHello({ tz: zoneName(), app: currentEnv().standalone, permission: perm });
    if (gen !== generation) return;
    device = r.device;
    helloAt = Date.now();
    helloPerm = perm;
    if (r.device.push) serverSub = true;
    else if (S.settings.push) serverSub = false;
    emit();
  } catch {
    /* the next start says hello again */
  }
}

function installWatchers(): void {
  if (started || typeof document === 'undefined') return;
  started = true;
  document.addEventListener('visibilitychange', () => {
    if (document.visibilityState !== 'visible' || !ready) return;
    // The permission may have been changed in the browser's settings; say hello again after an hour
    void refreshLocal();
    if (Date.now() - helloAt >= 3_600_000) void hello();
  });
}

/** The saved session that the question bank no longer holds is invisible in the UI: its reminders must stop */
function noteOrphanSaved(): void {
  const key = storedSessionKey();
  if (key && S.bank && !loadSaved(S.bank)) noteDiscarded(key);
}

export async function initDevice(): Promise<void> {
  const gen = generation;
  ready = false;
  installWatchers();
  try {
    void getConfig(false);
    await hello();
    if (gen !== generation) return;
    noteOrphanSaved();
    await refreshLocal();
    if (gen !== generation) return;
    await maybeSubscribe(false);
  } finally {
    if (gen === generation) {
      ready = true;
      emit();
    }
  }
}

/** The account switch was turned on (Settings, guide): subscribe at once when the browser already allows notifications */
export async function reconfirmPush(): Promise<void> {
  await maybeSubscribe(true);
}

// ---------------------------------------------------------------- enabling and testing

function askPermission(): Promise<NotificationPermission> {
  return new Promise((resolve, reject) => {
    try {
      // Older browsers take a callback, newer ones return a promise
      const r = Notification.requestPermission(resolve);
      if (r && typeof r.then === 'function') r.then(resolve, reject);
    } catch (e) {
      reject(e);
    }
  });
}

/**
 * A tap on "Enable on this device". WebKit grants the prompt only inside the tap, so `Notification.requestPermission()` is the
 * first call of this function and nothing is awaited before it. Resolves to the device's state afterwards; a real failure
 * (the server or the browser refused the subscription) rejects, and the caller shows it.
 */
export function enablePush(): Promise<DevicePushState> {
  const env = currentEnv();
  const before = stateNow(env);
  // Only a device that can be enabled and is not yet: everything else is explained by its state
  if (before !== 'off') return Promise.resolve(before);
  const asked = askPermission();
  busy = true;
  emit();
  const gen = generation;
  return (async (): Promise<DevicePushState> => {
    try {
      const perm = await asked;
      if (perm !== 'granted') {
        await hello();
        return stateNow(env);
      }
      await ensureSubscription(true);
      return stateNow(env);
    } finally {
      if (gen === generation) {
        busy = false;
        seenPerm = permission();
        emit();
      }
    }
  })();
}

export async function sendTestPush(): Promise<PushTestResponse> {
  const r = await sendPushTest();
  if (cfg) cfg = { ...cfg, courier: r.courier };
  emit();
  return r;
}

// ---------------------------------------------------------------- the guide (browser-local bookkeeping)

interface GuideStore { dismissed: boolean; skippedInstall: boolean; installed: boolean }
let guideMem: GuideStore = { dismissed: false, skippedInstall: false, installed: false };
const testMem = new Map<number, string>();

function readGuide(): GuideStore {
  try {
    const d: unknown = JSON.parse(localStorage.getItem(paths.guide) ?? 'null');
    if (d && typeof d === 'object') {
      const o = d as Partial<GuideStore>;
      guideMem = { dismissed: o.dismissed === true, skippedInstall: o.skippedInstall === true, installed: o.installed === true };
    }
  } catch {
    /* the in-memory copy stands in */
  }
  return guideMem;
}

function writeGuide(patch: Partial<GuideStore>): void {
  guideMem = { ...readGuide(), ...patch };
  try {
    localStorage.setItem(paths.guide, JSON.stringify(guideMem));
  } catch {
    /* kept in memory for this page load */
  }
}

export function guideDismissed(): boolean {
  return readGuide().dismissed;
}

export function dismissGuide(): void {
  writeGuide({ dismissed: true });
  reopened = false;
  inFlow = false;
  emit();
}

/** "Show the setup guide" in Settings: the card shows in full on Home until it is closed */
export function reopenGuide(): void {
  writeGuide({ dismissed: false });
  reopened = true;
  emit();
}

export function skipInstall(): void {
  writeGuide({ skippedInstall: true });
  emit();
}

function testState(): string {
  const uid = storageOwner();
  if (uid == null) return '';
  try {
    return localStorage.getItem(guideTestKey(uid)) ?? testMem.get(uid) ?? '';
  } catch {
    return testMem.get(uid) ?? '';
  }
}

function setTestState(v: '1' | '2'): void {
  const uid = storageOwner();
  if (uid == null) return;
  testMem.set(uid, v);
  try {
    localStorage.setItem(guideTestKey(uid), v);
  } catch {
    /* kept in memory */
  }
}

/** The guide already sent its automatic test notification for this user on this browser */
export function guideTestSent(): boolean {
  return testState() !== '';
}

export function markGuideTestSent(): void {
  if (testState() === '') setTestState('1');
}

export function confirmGuideTest(): void {
  setTestState('2');
  emit();
}

/** The guide card was shown in this page load (a device that is set up shows it only for the load that set it up) */
export function markGuideInFlow(): void {
  inFlow = true;
}

export function guideInputs(fromSettings: boolean): GuideInputs {
  const env = currentEnv();
  const g = readGuide();
  return {
    env,
    permission: permission(),
    subscribed: browserSub && serverSub,
    accountOn: S.settings.push,
    installPrompt: deferred != null,
    dismissed: g.dismissed,
    skippedInstall: g.skippedInstall,
    installed: g.installed || currentEnv().standalone,
    serverKey: serverKeyNow(),
    testConfirmed: testState() === '2',
    fromSettings: fromSettings || reopened,
    inFlow,
  };
}

// ---------------------------------------------------------------- sign-out

/** Sign-out: forget everything that belongs to the signed-in user (the browser keeps its subscription, see D.4) */
export function resetPushState(): void {
  generation += 1;
  cfg = null;
  cfgLoading = null;
  cfgFailedAt = 0;
  device = null;
  browserSub = false;
  serverSub = false;
  busy = false;
  ready = false;
  inFlow = false;
  reopened = false;
  helloAt = 0;
  helloPerm = null;
  seenPerm = null;
  emit();
}
