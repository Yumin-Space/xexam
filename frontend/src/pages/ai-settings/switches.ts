// AI settings page: the global feature switches (study: chat, hint + analysis, summary; banks: AI questions, AI import). AI hint
// and AI analysis are one switch that writes both keys (`hint` and `explain`).
// Turning one off applies to every NORMAL user at once; administrators are not affected and keep using the feature. The per-user
// switches stay on the Users tab. Each switch is saved when it is flipped.
import { setFeatureSwitches } from '../../api/ai-admin.ts';
import { miniSwitch } from '../../components/mini-switch.ts';
import { FEATURE_GROUPS, featureLabelKey, itemOn, withItem, type FeatureKey } from '../../domain/ai-features.ts';
import { errorText } from '../../i18n/errors.ts';
import { t, uiLang, type MsgKey } from '../../i18n/index.ts';
import { refreshAi } from '../../state/actions.ts';
import type { AiSettings } from '../../types/api.ts';
import { h } from '../../utils/dom.ts';
import { aiDown } from './status.ts';
import { A, adopt, redraw } from './state.ts';

async function flip(k: FeatureKey, on: boolean): Promise<void> {
  if (A.switchBusy) return;
  A.switchBusy = true;
  A.switchMsg = null;
  redraw();
  try {
    adopt(await setFeatureSwitches(withItem({}, k, on)));
    void refreshAi(true);
    // The Users tab reads the global switches from the same settings, nothing else to reload
  } catch (e) {
    if (!aiDown(e)) A.switchMsg = { ok: false, text: errorText(e) };
  }
  A.switchBusy = false;
  redraw();
  document.getElementById(`ai-sw-${k}`)?.focus();
}

export function switchesSection(s: AiSettings): HTMLElement {
  const row = (k: FeatureKey): HTMLElement => {
    const labelId = `ai-sw-${k}-l`;
    return h('li', { class: 'ais-sw' },
      h('span', { class: 'ais-sw-l', id: labelId }, t(featureLabelKey(k) as MsgKey)),
      miniSwitch({ id: `ai-sw-${k}`, checked: itemOn(s.featureSwitches, k), labelledBy: labelId, disabled: A.switchBusy, onToggle: (v) => void flip(k, v) }));
  };
  const colon = uiLang() === 'en' ? ':' : '：';
  return h('section', { class: 'panel ais-sec', 'aria-labelledby': 'ais-switches-h' },
    h('h2', { id: 'ais-switches-h', tabindex: '-1' }, t('ai.set.switches.title')),
    // owner correction: the switches apply to normal users only; administrators are not affected
    h('p', { class: 'ais-note' }, t('ai.set.switches.note')),
    h('ul', { class: 'ais-sws' }, FEATURE_GROUPS.main.map(row)),
    h('p', { class: 'ais-fg-l ais-sw-group' }, t('ai.feature.group.banks') + colon),
    h('ul', { class: 'ais-sws' }, FEATURE_GROUPS.banks.map(row)),
    A.switchMsg ? h('p', { class: 'ais-ferr', role: 'alert' }, A.switchMsg.text) : null);
}
