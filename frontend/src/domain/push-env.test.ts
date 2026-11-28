import assert from 'node:assert/strict';
import test from 'node:test';
import {
  detectEnv, devicePushState, guideStep, guideStepNumber, installView, notifyActions,
  type EnvInputs, type GuideInputs, type PushEnv,
} from './push-env.ts';

const IPHONE_SAFARI = 'Mozilla/5.0 (iPhone; CPU iPhone OS 17_2 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.2 Mobile/15E148 Safari/604.1';
const IPHONE_OLD = 'Mozilla/5.0 (iPhone; CPU iPhone OS 16_3 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/16.3 Mobile/15E148 Safari/604.1';
const IPHONE_164 = 'Mozilla/5.0 (iPhone; CPU iPhone OS 16_4 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/16.4 Mobile/15E148 Safari/604.1';
const IPHONE_CHROME = 'Mozilla/5.0 (iPhone; CPU iPhone OS 17_2 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) CriOS/120.0.6099.119 Mobile/15E148 Safari/604.1';
const IPHONE_FIREFOX = 'Mozilla/5.0 (iPhone; CPU iPhone OS 17_2 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) FxiOS/121.0 Mobile/15E148 Safari/605.1.15';
const IPHONE_EDGE = 'Mozilla/5.0 (iPhone; CPU iPhone OS 17_2 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.0 EdgiOS/120.0 Mobile/15E148 Safari/605.1.15';
const IPAD_MAC_UA = 'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.2 Safari/605.1.15';
const IPAD_UA = 'Mozilla/5.0 (iPad; CPU OS 16_6 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/16.6 Mobile/15E148 Safari/604.1';
const ANDROID_CHROME = 'Mozilla/5.0 (Linux; Android 14; Pixel 8) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Mobile Safari/537.36';
const ANDROID_FIREFOX = 'Mozilla/5.0 (Android 14; Mobile; rv:121.0) Gecko/121.0 Firefox/121.0';
const ANDROID_SAMSUNG = 'Mozilla/5.0 (Linux; Android 14; SM-S918B) AppleWebKit/537.36 (KHTML, like Gecko) SamsungBrowser/23.0 Chrome/115.0.0.0 Mobile Safari/537.36';
const ANDROID_EDGE = 'Mozilla/5.0 (Linux; Android 14; Pixel 8) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Mobile Safari/537.36 EdgA/120.0.0.0';
const WIN_CHROME = 'Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36';
const WIN_EDGE = 'Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36 Edg/120.0.0.0';
const MAC_SAFARI = IPAD_MAC_UA;
const LINUX_FIREFOX = 'Mozilla/5.0 (X11; Linux x86_64; rv:121.0) Gecko/20100101 Firefox/121.0';

const inputs = (o: Partial<EnvInputs> = {}): EnvInputs => ({
  ua: ANDROID_CHROME, maxTouchPoints: 5, standalone: false, secure: true, sw: true, push: true, coarse: true, phoneWidth: true, ...o,
});
const envOf = (o: Partial<EnvInputs> = {}): PushEnv => detectEnv(inputs(o));

test('detectEnv: platforms', () => {
  assert.equal(envOf({ ua: IPHONE_SAFARI }).platform, 'ios');
  assert.equal(envOf({ ua: IPAD_UA }).platform, 'ios');
  assert.equal(envOf({ ua: ANDROID_CHROME }).platform, 'android');
  assert.equal(envOf({ ua: WIN_CHROME, maxTouchPoints: 0, coarse: false, phoneWidth: false }).platform, 'desktop');
  assert.equal(envOf({ ua: LINUX_FIREFOX, maxTouchPoints: 0, coarse: false, phoneWidth: false }).platform, 'desktop');
  assert.equal(envOf({ ua: 'SomeBot/1.0', maxTouchPoints: 0, coarse: false, phoneWidth: false }).platform, 'other');
});

test('detectEnv: iPadOS with a Mac user agent counts as ios only with a touch screen', () => {
  assert.equal(envOf({ ua: IPAD_MAC_UA, maxTouchPoints: 5 }).platform, 'ios');
  assert.equal(envOf({ ua: IPAD_MAC_UA, maxTouchPoints: 0, coarse: false, phoneWidth: false }).platform, 'desktop');
  assert.equal(envOf({ ua: IPAD_MAC_UA, maxTouchPoints: 1, coarse: false, phoneWidth: false }).platform, 'desktop');
  // the version is unknown there
  assert.equal(envOf({ ua: IPAD_MAC_UA, maxTouchPoints: 5 }).iosVersion, null);
});

test('detectEnv: iOS version', () => {
  assert.equal(envOf({ ua: IPHONE_SAFARI }).iosVersion, 17.2);
  assert.equal(envOf({ ua: IPHONE_OLD }).iosVersion, 16.3);
  assert.equal(envOf({ ua: IPHONE_164 }).iosVersion, 16.4);
  assert.equal(envOf({ ua: IPAD_UA }).iosVersion, 16.6);
  assert.equal(envOf({ ua: ANDROID_CHROME }).iosVersion, null);
});

test('detectEnv: browsers', () => {
  assert.equal(envOf({ ua: IPHONE_SAFARI }).browser, 'safari');
  assert.equal(envOf({ ua: IPHONE_CHROME }).browser, 'chrome');
  assert.equal(envOf({ ua: IPHONE_FIREFOX }).browser, 'firefox');
  assert.equal(envOf({ ua: IPHONE_EDGE }).browser, 'edge');
  assert.equal(envOf({ ua: ANDROID_CHROME }).browser, 'chrome');
  assert.equal(envOf({ ua: ANDROID_FIREFOX }).browser, 'firefox');
  assert.equal(envOf({ ua: ANDROID_SAMSUNG }).browser, 'samsung');
  assert.equal(envOf({ ua: ANDROID_EDGE }).browser, 'edge');
  assert.equal(envOf({ ua: WIN_EDGE }).browser, 'edge');
  assert.equal(envOf({ ua: WIN_CHROME }).browser, 'chrome');
  assert.equal(envOf({ ua: MAC_SAFARI }).browser, 'safari');
  assert.equal(envOf({ ua: 'curl/8' }).browser, 'other');
});

test('detectEnv: mobile = ios, android, or touch with a small screen', () => {
  assert.equal(envOf({ ua: IPHONE_SAFARI, coarse: false, phoneWidth: false }).mobile, true);
  assert.equal(envOf({ ua: ANDROID_CHROME, coarse: false, phoneWidth: false }).mobile, true);
  assert.equal(envOf({ ua: WIN_CHROME, coarse: true, phoneWidth: true }).mobile, true);
  assert.equal(envOf({ ua: WIN_CHROME, coarse: true, phoneWidth: false }).mobile, false);
  assert.equal(envOf({ ua: WIN_CHROME, coarse: false, phoneWidth: true }).mobile, false);
});

test('detectEnv: passes the flags through', () => {
  const e = envOf({ standalone: true, secure: false, sw: false, push: false });
  assert.deepEqual([e.standalone, e.secure, e.sw, e.push], [true, false, false, false]);
});

test('devicePushState: order of the checks', () => {
  const android = envOf();
  const iphone = envOf({ ua: IPHONE_SAFARI });
  const app = envOf({ ua: IPHONE_SAFARI, standalone: true });
  assert.equal(devicePushState(envOf({ secure: false }), 'default', false, true, true), 'insecure');
  // insecure beats everything, also an old iPhone
  assert.equal(devicePushState(envOf({ ua: IPHONE_OLD, secure: false }), 'default', false, true, true), 'insecure');
  assert.equal(devicePushState(envOf({ ua: IPHONE_OLD, standalone: true }), 'default', false, true, true), 'iosOld');
  assert.equal(devicePushState(iphone, 'default', false, true, true), 'needsInstall');
  // the iOS check wins over the feature check (headless Chrome with an iPhone UA has PushManager)
  assert.equal(devicePushState(envOf({ ua: IPHONE_SAFARI, sw: false, push: false }), 'default', false, true, true), 'needsInstall');
  assert.equal(devicePushState(envOf({ sw: false }), 'default', false, true, true), 'unsupported');
  assert.equal(devicePushState(envOf({ push: false }), 'default', false, true, true), 'unsupported');
  assert.equal(devicePushState(android, 'default', false, true, false), 'serverOff');
  assert.equal(devicePushState(android, 'default', false, false, true), 'accountOff');
  assert.equal(devicePushState(android, 'default', false, false, false), 'serverOff');
  assert.equal(devicePushState(android, 'denied', false, true, true), 'blocked');
  assert.equal(devicePushState(android, 'denied', true, true, true), 'blocked');
  assert.equal(devicePushState(android, 'granted', true, true, true), 'on');
  assert.equal(devicePushState(app, 'granted', true, true, true), 'on');
  assert.equal(devicePushState(android, 'granted', false, true, true), 'off');
  assert.equal(devicePushState(android, 'default', false, true, true), 'off');
  assert.equal(devicePushState(android, 'default', true, true, true), 'off');
  // a 16.4 phone is new enough
  assert.equal(devicePushState(envOf({ ua: IPHONE_164, standalone: true }), 'default', false, true, true), 'off');
});

test('notifyActions: every state', () => {
  assert.deepEqual(notifyActions('on'), { enable: 'hidden', test: true });
  assert.deepEqual(notifyActions('off'), { enable: 'enabled', test: false });
  for (const s of ['blocked', 'needsInstall', 'insecure', 'unsupported', 'iosOld', 'accountOff', 'serverOff'] as const) {
    assert.deepEqual(notifyActions(s), { enable: 'disabled', test: false }, s);
  }
});

test('installView: every branch', () => {
  assert.equal(installView(envOf({ secure: false }), true), 'insecure');
  assert.equal(installView(envOf({ secure: false, standalone: true }), false), 'insecure');
  assert.equal(installView(envOf({ standalone: true }), true), 'done');
  assert.equal(installView(envOf(), true), 'prompt');
  assert.equal(installView(envOf(), false), 'android');
  assert.equal(installView(envOf(), false, true), 'done');
  assert.equal(installView(envOf({ ua: IPHONE_SAFARI }), true), 'ios');
  assert.equal(installView(envOf({ ua: IPHONE_CHROME }), false), 'ios');
  assert.equal(installView(envOf({ ua: IPHONE_FIREFOX }), false), 'iosOther');
  assert.equal(installView(envOf({ ua: IPHONE_EDGE }), false), 'iosOther');
  assert.equal(installView(envOf({ ua: WIN_CHROME, coarse: false, phoneWidth: false }), true), 'other');
});

const guide = (env: PushEnv, o: Partial<GuideInputs> = {}): GuideInputs => ({
  env, permission: 'default', subscribed: false, accountOn: true, installPrompt: false, dismissed: false,
  skippedInstall: false, installed: false, serverKey: true, testConfirmed: false, fromSettings: false, inFlow: false, ...o,
});

test('guideStep rule 1: auto mode hides for dismissed, desktop and insecure; Settings ignores that', () => {
  const android = envOf();
  assert.equal(guideStep(guide(android, { dismissed: true })), 'hidden');
  const desktop = envOf({ ua: WIN_CHROME, coarse: false, phoneWidth: false, maxTouchPoints: 0 });
  assert.equal(guideStep(guide(desktop)), 'hidden');
  assert.equal(guideStep(guide(envOf({ secure: false }))), 'hidden');
  // from Settings the dismissed flag is ignored, the desktop gets the notification step, an insecure page says so
  assert.equal(guideStep(guide(android, { dismissed: true, fromSettings: true })), 'installAndroidManual');
  assert.equal(guideStep(guide(desktop, { fromSettings: true })), 'notify');
  assert.equal(guideStep(guide(envOf({ secure: false }), { fromSettings: true })), 'insecure');
});

test('guideStep rules 3-4: iOS', () => {
  assert.equal(guideStep(guide(envOf({ ua: IPHONE_OLD }), { fromSettings: true })), 'iosOld');
  // D17: the explanation shows only from Settings
  assert.equal(guideStep(guide(envOf({ ua: IPHONE_OLD }))), 'hidden');
  assert.equal(guideStep(guide(envOf({ ua: IPHONE_SAFARI }))), 'installIosSafari');
  assert.equal(guideStep(guide(envOf({ ua: IPHONE_CHROME }))), 'installIosChrome');
  assert.equal(guideStep(guide(envOf({ ua: IPHONE_FIREFOX }))), 'installIosOther');
  assert.equal(guideStep(guide(envOf({ ua: IPAD_MAC_UA }))), 'installIosSafari');
  // skippedInstall does not apply to iOS (notifications need the Home Screen app)
  assert.equal(guideStep(guide(envOf({ ua: IPHONE_SAFARI }), { skippedInstall: true })), 'installIosSafari');
  // opened as the Home Screen app: straight to the notification step
  assert.equal(guideStep(guide(envOf({ ua: IPHONE_SAFARI, standalone: true }))), 'notify');
});

test('guideStep rule 5: Android install, skippable', () => {
  assert.equal(guideStep(guide(envOf(), { installPrompt: true })), 'installAndroid');
  assert.equal(guideStep(guide(envOf())), 'installAndroidManual');
  assert.equal(guideStep(guide(envOf(), { skippedInstall: true })), 'notify');
  assert.equal(guideStep(guide(envOf({ standalone: true }))), 'notify');
  // installed from this browser: the browser tab moves on to the notification step
  assert.equal(guideStep(guide(envOf(), { installed: true })), 'notify');
  assert.equal(guideStep(guide(envOf(), { installed: true, installPrompt: true })), 'notify');
});

test('guideStep rules 6-7: features, account, permission', () => {
  const app = envOf({ standalone: true });
  assert.equal(guideStep(guide(envOf({ standalone: true, sw: false }))), 'hidden');
  assert.equal(guideStep(guide(envOf({ standalone: true, push: false }))), 'hidden');
  assert.equal(guideStep(guide(envOf({ standalone: true, sw: false }), { fromSettings: true })), 'unsupported');
  assert.equal(guideStep(guide(envOf({ standalone: true, push: false }), { fromSettings: true })), 'unsupported');
  // no push key on the server: an explanation, never the Enable step
  assert.equal(guideStep(guide(app, { serverKey: false })), 'serverOff');
  assert.equal(guideStep(guide(app, { accountOn: false })), 'notifyAccountOff');
  assert.equal(guideStep(guide(app, { permission: 'denied' })), 'notifyDenied');
  assert.equal(guideStep(guide(app, { permission: 'default' })), 'notify');
  assert.equal(guideStep(guide(app, { permission: 'granted', subscribed: false })), 'notify');
  assert.equal(guideStep(guide(app, { permission: 'default', subscribed: true })), 'notify');
  // the account switch is checked before the permission
  assert.equal(guideStep(guide(app, { accountOn: false, permission: 'denied' })), 'notifyAccountOff');
});

test('guideStep rules 8-9: set up devices, test, done', () => {
  const app = envOf({ standalone: true });
  const set = { permission: 'granted', subscribed: true } as const;
  // a device that is already set up never shows the card on a later load
  assert.equal(guideStep(guide(app, { ...set })), 'hidden');
  assert.equal(guideStep(guide(app, { ...set, testConfirmed: true })), 'hidden');
  // within the load that set it up the card moves on
  assert.equal(guideStep(guide(app, { ...set, inFlow: true })), 'test');
  assert.equal(guideStep(guide(app, { ...set, inFlow: true, testConfirmed: true })), 'done');
  // from Settings it always shows the last steps
  assert.equal(guideStep(guide(app, { ...set, fromSettings: true })), 'test');
  assert.equal(guideStep(guide(app, { ...set, fromSettings: true, testConfirmed: true })), 'done');
});

test('guideStepNumber', () => {
  for (const s of ['installAndroid', 'installAndroidManual', 'installIosSafari', 'installIosChrome', 'installIosOther'] as const) {
    assert.equal(guideStepNumber(s), 1, s);
  }
  for (const s of ['notify', 'notifyDenied', 'notifyAccountOff'] as const) assert.equal(guideStepNumber(s), 2, s);
  assert.equal(guideStepNumber('serverOff'), null);
  assert.equal(guideStepNumber('test'), 3);
  for (const s of ['hidden', 'done', 'insecure', 'unsupported', 'iosOld'] as const) assert.equal(guideStepNumber(s), null, s);
});
