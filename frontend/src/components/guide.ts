// The phone setup guide card (v3.1): install the app, allow notifications, check a test arrives. It shows on Home to phones
// and tablets that are not set up yet (and is never closed for them by accident: the close button remembers the choice in
// this browser), and again from Settings → "Show the setup guide". Which step shows is decided by domain/push-env.ts
// (`guideStep`); this file only draws it and wires the buttons to state/push.ts.
import '../styles/guide.css';
import { guideStep, guideStepNumber, type GuideStep } from '../domain/push-env.ts';
import { errorText } from '../i18n/errors.ts';
import { t, uiLang } from '../i18n/index.ts';
import { render } from '../state/actions.ts';
import { S } from '../state/store.ts';
import { updateSettings } from '../state/preferences.ts';
import {
  confirmGuideTest, dismissGuide, enablePush, guideInputs, guideTestSent, markGuideInFlow, markGuideTestSent, promptInstall,
  pushReady, pushView, reconfirmPush, sendTestPush, skipInstall,
} from '../state/push.ts';
import { h, type Child } from '../utils/dom.ts';
import { toast } from '../utils/toast.ts';
import { artAddButton, artAddToHome, artShare } from './guide-art.ts';
import { miniIcon } from './mini-icons.ts';

/** The last thing that went wrong in the card (shown under the buttons until the next tap) */
let problem: string | null = null;
/** A test notification is on its way (the "Send again" button waits) */
let testing = false;
/** The step drawn last (a change of step moves focus to the title) */
let shownStep: GuideStep | null = null;

const INSTALL_STEPS: ReadonlySet<GuideStep> = new Set<GuideStep>(['installAndroid', 'installAndroidManual', 'installIosSafari', 'installIosChrome', 'installIosOther']);

function failWith(key: 'guide.notify.failed' | 'guide.test.failed', e: unknown): void {
  problem = t(key, { reason: errorText(e) });
  render();
}

async function sendTest(): Promise<void> {
  if (testing) return;
  testing = true;
  try {
    const r = await sendTestPush();
    problem = null;
    const sep = uiLang() === 'en' ? ' ' : '';
    const note = r.courier ? '' : sep + t(S.user?.role === 'admin' ? 'settings.notify.noCourier.admin' : 'settings.notify.noCourier.user');
    toast(t('guide.test.sent') + note, note ? 6000 : 3500);
  } catch (e) {
    failWith('guide.test.failed', e);
  } finally {
    testing = false;
    render();
  }
}

function turnAccountOn(): void {
  updateSettings({ push: true }, render).then(() => reconfirmPush(), (e: unknown) => toast(errorText(e), 5000));
}

// ---------------------------------------------------------------- pieces

function progress(step: GuideStep): HTMLElement | null {
  const n = guideStepNumber(step);
  const done = step === 'done';
  // Only the three real steps have a progress row; explanations ("insecure", "unsupported", …) have none
  if (n == null && !done) return null;
  const labels: Array<[1 | 2 | 3, string]> = [[1, t('guide.s1')], [2, t('guide.s2')], [3, t('guide.s3')]];
  return h('ol', { class: 'gd-steps', 'aria-hidden': 'true' }, labels.map(([i, label]) =>
    h('li', { class: `gd-step${done || (n != null && i < n) ? ' is-done' : ''}${n === i ? ' is-on' : ''}` },
      h('span', { class: 'gd-dot' }), h('span', { class: 'gd-step-l' }, label))));
}

/** A numbered instruction with the drawing under it */
function item(n: number, text: string, art: SVGSVGElement): HTMLElement {
  return h('li', { class: 'gd-item' },
    h('span', { class: 'gd-item-n', 'aria-hidden': 'true' }, n),
    h('div', { class: 'gd-item-body' }, h('p', { class: 'gd-item-t' }, text), h('div', { class: 'gd-art-box' }, art)));
}

function iosList(kind: 'safari' | 'chrome'): HTMLElement {
  const k = kind === 'safari' ? 'guide.install.iosSafari' : 'guide.install.iosChrome';
  const texts = [t(`${k}.1`), t(`${k}.2`), t(`${k}.3`)];
  return h('ol', { class: 'gd-list' },
    item(1, texts[0]!, artShare(kind === 'safari' ? 'bottom' : 'top', texts[0])),
    item(2, texts[1]!, artAddToHome(texts[1])),
    item(3, texts[2]!, artAddButton(texts[2])));
}

const btn = (id: string, cls: string, label: string, onclick: () => void, disabled = false): HTMLButtonElement =>
  h('button', { class: `btn ${cls}`.trim(), type: 'button', id, disabled, onclick }, label);

function body(step: GuideStep): Child[] {
  const v = pushView();
  const p = (text: string, cls = 'gd-text'): HTMLElement => h('p', { class: cls }, text);
  switch (step) {
    case 'installAndroid':
      return [
        p(t('guide.install.android')),
        h('div', { class: 'gd-acts' },
          btn('gd-install', 'primary', t('guide.install.androidBtn'), () => void promptInstall()),
          btn('gd-skip', 'ghost', t('guide.skip'), skipInstall)),
      ];
    case 'installAndroidManual':
      return [p(t('guide.install.androidManual')), h('div', { class: 'gd-acts' }, btn('gd-skip', 'ghost', t('guide.skip'), skipInstall))];
    case 'installIosSafari':
      return [iosList('safari')];
    case 'installIosChrome':
      return [iosList('chrome')];
    case 'installIosOther':
      return [p(t('guide.install.iosOther'))];
    case 'notify':
      return [
        p(t('guide.notify.lead')),
        v.env.platform === 'ios' ? p(t('guide.notify.iosNote'), 'gd-text gd-note') : null,
        h('div', { class: 'gd-acts' }, btn('gd-enable', 'primary', t(v.busy ? 'guide.notify.busy' : 'guide.notify.btn'), () => {
          problem = null;
          // The permission prompt must be the first call of the tap (state/push.ts), so nothing else runs before it
          enablePush().then(() => render(), (e: unknown) => failWith('guide.notify.failed', e));
        }, v.busy)),
      ];
    case 'serverOff':
      // Its own sentence: the Settings state text is a lower-case continuation of "This device:"
      return [p(t('guide.serverOff'))];
    case 'notifyDenied':
      return [p(t('guide.notify.denied'))];
    case 'notifyAccountOff':
      return [p(t('guide.notify.accountOff')), h('div', { class: 'gd-acts' }, btn('gd-turnon', 'primary', t('guide.notify.turnOn'), turnAccountOn))];
    case 'test':
      // One test goes out by itself the first time this step shows for this user on this browser
      if (!guideTestSent()) {
        markGuideTestSent();
        void sendTest();
      }
      return [
        p(t('guide.test.lead')),
        h('div', { class: 'gd-acts' },
          btn('gd-yes', 'primary', t('guide.test.yes'), confirmGuideTest),
          btn('gd-again', 'ghost', t('guide.test.again'), () => void sendTest(), testing)),
      ];
    case 'done':
      return [p(t('guide.done'))];
    case 'insecure':
      return [p(t('guide.insecure'))];
    case 'unsupported':
      return [p(t('guide.unsupported'))];
    case 'iosOld':
      return [p(t('guide.iosOld'))];
    default:
      return [];
  }
}

/**
 * The guide card, or null when there is nothing to show (not a phone, closed, set up already, or the device is still being
 * checked). `fromSettings` is the Settings entry; the Home mount passes false and the "Show the setup guide" button of Settings
 * makes the card show in full anyway (state/push.ts `reopenGuide`).
 */
export function guideCard(opts: { fromSettings: boolean }): HTMLElement | null {
  // Not before the first hello and subscription check: a set-up device must not flash the "allow notifications" step
  if (!pushReady()) return null;
  const step = guideStep(guideInputs(opts.fromSettings));
  if (step === 'hidden') {
    problem = null;
    shownStep = null;
    return null;
  }
  markGuideInFlow();
  const n = guideStepNumber(step);
  // A new step replaces the pressed button: move focus to the card title so the change is announced (not on the first draw)
  if (shownStep !== null && shownStep !== step) queueMicrotask(() => document.getElementById('gd-title')?.focus({ preventScroll: true }));
  shownStep = step;
  return h('section', { class: 'panel gd-card', id: 'gd-card', 'aria-labelledby': 'gd-title' },
    h('div', { class: 'gd-head' },
      h('div', { class: 'gd-headtext' },
        h('h2', { class: 'gd-title', id: 'gd-title', tabindex: -1 }, t('guide.title')),
        n != null ? h('p', { class: 'gd-count' }, t('guide.step', { n })) : null),
      h('button', { class: 'gd-close', type: 'button', id: 'gd-close', 'aria-label': t('guide.close'), onclick: dismissGuide },
        miniIcon('x', 'gd-close-ico'))),
    progress(step),
    h('div', { class: `gd-body${INSTALL_STEPS.has(step) ? ' gd-install' : ''}` }, ...body(step)),
    problem ? h('p', { class: 'gd-err', role: 'alert' }, problem) : null);
}
