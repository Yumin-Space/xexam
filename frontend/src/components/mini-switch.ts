// A small on / off switch (v0.8): `button[role=switch][aria-checked]` with a 44×26 track and a knob (styles: .mini-switch in
// main.css). Used for the AI+ service switch of User management, the global "AI+ service" of AI settings
// (administrators). Unlike Settings' full-width .set-switch row it has no row of its own; its accessible
// name comes from `label` or `labelledBy`.
// `blocked`: it cannot be switched now (aria-disabled, dimmed, still focusable so its reason can be found; the click is
// refused); `disabled`: a request runs (really disabled).
import { h } from '../utils/dom.ts';

export interface MiniSwitchOptions {
  id: string;
  checked: boolean;
  /** The accessible name (or use labelledBy) */
  label?: string;
  labelledBy?: string;
  describedBy?: string;
  disabled?: boolean;
  blocked?: boolean;
  onToggle: (next: boolean) => void;
}

export function miniSwitch(o: MiniSwitchOptions): HTMLButtonElement {
  return h('button', {
    class: 'mini-switch', id: o.id, type: 'button', role: 'switch', 'aria-checked': String(o.checked),
    'aria-label': o.label ?? null, 'aria-labelledby': o.labelledBy ?? null, 'aria-describedby': o.describedBy ?? null,
    'aria-disabled': o.blocked ? 'true' : null, disabled: !!o.disabled,
    onclick: (e: MouseEvent) => {
      e.preventDefault();
      if (o.blocked || o.disabled) return;
      o.onToggle(!o.checked);
    },
  }, h('span', { class: 'mini-track', 'aria-hidden': 'true' }, h('span', { class: 'mini-knob' })));
}
