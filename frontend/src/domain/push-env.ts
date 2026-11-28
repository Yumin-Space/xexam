// The browser environment for installing the app and notifications (v3.1): what kind of device and browser this is, whether
// it can show notifications at all, what state this device is in, and which step the phone setup guide shows. Pure: every
// input is passed in, nothing reads the window, so it is tested for every case (the secure-context branches included).
import type { PushPermission } from '../types/api.ts';

export type Platform = 'ios' | 'android' | 'desktop' | 'other';
export type BrowserKind = 'safari' | 'chrome' | 'firefox' | 'edge' | 'samsung' | 'other';
export interface PushEnv {
  platform: Platform; browser: BrowserKind; iosVersion: number | null;
  /** phone or tablet: ios or android, or touch + small screen (iPadOS: 'Macintosh' UA + maxTouchPoints > 1 counts as ios) */
  mobile: boolean;
  /** display-mode standalone or navigator.standalone */
  standalone: boolean;
  /** window.isSecureContext */
  secure: boolean;
  /** 'serviceWorker' in navigator */
  sw: boolean;
  /** 'PushManager' in window && 'Notification' in window */
  push: boolean;
}
export interface EnvInputs { ua: string; maxTouchPoints: number; standalone: boolean; secure: boolean; sw: boolean; push: boolean; coarse: boolean; phoneWidth: boolean }

/** The first iOS / iPadOS version that can show web push (Home Screen apps only) */
export const IOS_PUSH_MIN = 16.4;

function platformOf(ua: string, maxTouchPoints: number): Platform {
  if (/iPhone|iPad|iPod/.test(ua)) return 'ios';
  // iPadOS 13+ reports a desktop Mac user agent; only the touch screen tells it apart from a Mac
  if (/Macintosh/.test(ua) && maxTouchPoints > 1) return 'ios';
  if (/Android/.test(ua)) return 'android';
  if (/Windows|Macintosh|CrOS|X11|Linux/.test(ua)) return 'desktop';
  return 'other';
}

function browserOf(ua: string): BrowserKind {
  if (/EdgA|EdgiOS|Edg\//.test(ua)) return 'edge';
  if (/SamsungBrowser/.test(ua)) return 'samsung';
  // Browsers that look like Chrome or Safari but are neither
  if (/OPR\/|OPT\/|OPiOS|Opera|DuckDuckGo|Vivaldi|UCBrowser|YaBrowser/.test(ua)) return 'other';
  if (/Firefox|FxiOS/.test(ua)) return 'firefox';
  if (/Chrome|Chromium|CriOS/.test(ua)) return 'chrome';
  if (/Safari/.test(ua) && /Version\//.test(ua)) return 'safari';
  return 'other';
}

/** The iOS version as a number (16.4), or null when the user agent does not say (iPadOS reports a frozen Mac version) */
function iosVersionOf(ua: string): number | null {
  const m = /(?:iPhone |CPU )OS (\d+)[_.](\d+)/.exec(ua);
  if (!m) return null;
  const major = Number(m[1]);
  const minor = Number(m[2]);
  if (!Number.isFinite(major) || !Number.isFinite(minor)) return null;
  return minor >= 10 ? major + 0.9 : major + minor / 10;
}

export function detectEnv(i: EnvInputs): PushEnv {
  const platform = platformOf(i.ua, i.maxTouchPoints);
  return {
    platform,
    browser: browserOf(i.ua),
    iosVersion: platform === 'ios' ? iosVersionOf(i.ua) : null,
    mobile: platform === 'ios' || platform === 'android' || (i.coarse && i.phoneWidth),
    standalone: i.standalone,
    secure: i.secure,
    sw: i.sw,
    push: i.push,
  };
}

const iosOld = (env: PushEnv): boolean => env.platform === 'ios' && env.iosVersion != null && env.iosVersion < IOS_PUSH_MIN;

export type DevicePushState = 'on' | 'off' | 'blocked' | 'needsInstall' | 'insecure' | 'unsupported' | 'iosOld' | 'accountOff' | 'serverOff';

/**
 * What notifications are like on this device. First match wins; the iOS checks come before the feature checks (the same
 * order as the guide's rules 3-4) because a desktop browser with an iPhone user agent still has `PushManager`.
 */
export function devicePushState(env: PushEnv, permission: PushPermission, subscribed: boolean, accountOn: boolean, serverKey: boolean): DevicePushState {
  if (!env.secure) return 'insecure';
  if (iosOld(env)) return 'iosOld';
  if (env.platform === 'ios' && !env.standalone) return 'needsInstall';
  if (!env.sw || !env.push) return 'unsupported';
  if (!serverKey) return 'serverOff';
  if (!accountOn) return 'accountOff';
  if (permission === 'denied') return 'blocked';
  if (subscribed && permission === 'granted') return 'on';
  return 'off';
}

export type GuideStep =
  | 'hidden' | 'installAndroid' | 'installAndroidManual' | 'installIosSafari' | 'installIosChrome' | 'installIosOther'
  | 'notify' | 'notifyDenied' | 'notifyAccountOff' | 'serverOff' | 'test' | 'done' | 'insecure' | 'unsupported' | 'iosOld';
export interface GuideInputs { env: PushEnv; permission: PushPermission; subscribed: boolean; accountOn: boolean;
  installPrompt: boolean; dismissed: boolean; skippedInstall: boolean;
  /** the app was installed from this browser (the install prompt was accepted, or `appinstalled` fired) */
  installed: boolean;
  /** the server has a push key and its config loaded */
  serverKey: boolean; testConfirmed: boolean; fromSettings: boolean;
  /** true once the card was shown in this page load (kept in memory by state/push.ts) */
  inFlow: boolean }

/** The step of the phone setup guide (rules in order, first match wins), or `hidden` */
export function guideStep(g: GuideInputs): GuideStep {
  const { env } = g;
  // 1. Auto mode shows the card only to a phone or tablet in a secure context that did not close it
  if (!g.fromSettings && (g.dismissed || !env.mobile || !env.secure)) return 'hidden';
  // 2.-3. Opened from Settings: say why nothing can be set up
  if (!env.secure) return 'insecure';
  if (iosOld(env)) return g.fromSettings ? 'iosOld' : 'hidden'; // D17: explanations only from Settings
  // 4. iPhone / iPad: notifications work only in the Home Screen app
  if (env.platform === 'ios' && !env.standalone) {
    return env.browser === 'safari' ? 'installIosSafari' : env.browser === 'chrome' ? 'installIosChrome' : 'installIosOther';
  }
  // 5. Android: installing is optional (it can be skipped)
  if (env.platform === 'android' && !env.standalone && !g.skippedInstall && !g.installed) return g.installPrompt ? 'installAndroid' : 'installAndroidManual';
  // 6.
  if (!env.sw || !env.push) return g.fromSettings ? 'unsupported' : 'hidden'; // D17
  // 6b. The server cannot send push: explain, nothing to enable
  if (!g.serverKey) return 'serverOff';
  // 7.
  if (!g.accountOn) return 'notifyAccountOff';
  if (g.permission === 'denied') return 'notifyDenied';
  const set = g.permission === 'granted' && g.subscribed;
  if (!set) return 'notify';
  // 8. A device that is set up never shows the guide again on a later page load (it stays within the load that set it up)
  if (!g.fromSettings && !g.inFlow) return 'hidden';
  // 9.
  return g.testConfirmed ? 'done' : 'test';
}

/** The step counter: install = 1, notifications = 2, test = 3, everything else none */
export function guideStepNumber(s: GuideStep): 1 | 2 | 3 | null {
  switch (s) {
    case 'installAndroid': case 'installAndroidManual': case 'installIosSafari': case 'installIosChrome': case 'installIosOther':
      return 1;
    case 'notify': case 'notifyDenied': case 'notifyAccountOff':
      return 2;
    case 'test':
      return 3;
    default:
      return null;
  }
}

/** What the Settings → Notifications buttons do for a state: Enable is hidden only when `on`, enabled only when `off`,
 *  otherwise shown disabled (the state text explains why); Test only when `on`. */
export function notifyActions(s: DevicePushState): { enable: 'hidden' | 'enabled' | 'disabled'; test: boolean } {
  return { enable: s === 'on' ? 'hidden' : s === 'off' ? 'enabled' : 'disabled', test: s === 'on' };
}

export type InstallView = 'insecure' | 'done' | 'prompt' | 'android' | 'ios' | 'iosOther' | 'other';

/** Settings → Install: no secure context → insecure; already an app → done; Android with a captured prompt → prompt; Android →
 *  android; iPhone / iPad in Safari or Chrome → ios; another iOS browser → iosOther; everything else → other. */
export function installView(env: PushEnv, installPrompt: boolean, installed = false): InstallView {
  if (!env.secure) return 'insecure';
  if (env.standalone || (installed && env.platform === 'android')) return 'done';
  if (env.platform === 'android') return installPrompt ? 'prompt' : 'android';
  if (env.platform === 'ios') return env.browser === 'safari' || env.browser === 'chrome' ? 'ios' : 'iosOther';
  return 'other';
}
