// v2.6 preview (owner): "blackboard mode" — the hand-written face (Xiaolai subset) instead of the normal UI font. A per-browser
// convenience kept in localStorage; on by default (owner), so a private window or blocked storage starts hand-written. The switch sits in
// the top bar of every page; Home, Exam & practice, Study records, the exam page and AI Chat follow it (html[data-hand]).

const KEY = 'xexam.handMode';

/** Whether blackboard mode is on in this browser: on by default (owner), off only after "normal text" was chosen */
let mem: boolean | null = null;

export function handMode(): boolean {
  if (mem !== null) return mem;
  try {
    mem = localStorage.getItem(KEY) !== '0';
  } catch {
    mem = true;
  }
  return mem;
}

/** Sets the mode (Settings › Text, or the top-bar switch), saves it when the browser allows, and shows it */
export function setHandMode(on: boolean): void {
  mem = on;
  try {
    localStorage.setItem(KEY, on ? '1' : '0');
  } catch {
    // storage blocked: the mode still applies until the page is reloaded
  }
  applyHandMode(on);
}

const NS = 'http://www.w3.org/2000/svg';

/**
 * The "hand-drawn" filter for icons in blackboard mode: low-frequency noise bends every straight stroke a little, so the
 * line icons look drawn by hand (CSS adds an offset retrace with drop-shadow). One hidden <svg> per page, added on demand.
 */
function ensureWobbleFilter(): void {
  if (document.getElementById('xexam-hand-defs')) return;
  const svg = document.createElementNS(NS, 'svg');
  svg.id = 'xexam-hand-defs';
  svg.setAttribute('aria-hidden', 'true');
  svg.setAttribute('width', '0');
  svg.setAttribute('height', '0');
  svg.setAttribute('style', 'position:absolute;width:0;height:0;overflow:hidden');
  const el = (tag: string, attrs: Record<string, string>, ...kids: Element[]): Element => {
    const e = document.createElementNS(NS, tag);
    for (const [k, v] of Object.entries(attrs)) e.setAttribute(k, v);
    e.append(...kids);
    return e;
  };
  // the ragged edge of red ink (the correct / incorrect marks): fine noise moves the outline by about a pixel
  svg.append(el('filter', { id: 'xexam-crayon', x: '-15%', y: '-15%', width: '130%', height: '130%' },
    el('feTurbulence', { type: 'fractalNoise', baseFrequency: '1.4', numOctaves: '1', seed: '5', result: 'n' }),
    el('feDisplacementMap', { in: 'SourceGraphic', in2: 'n', scale: '1.1', xChannelSelector: 'R', yChannelSelector: 'G' })));
  svg.append(el('filter', { id: 'xexam-hand', x: '-20%', y: '-20%', width: '140%', height: '140%' },
    el('feTurbulence', { type: 'fractalNoise', baseFrequency: '0.06', numOctaves: '2', seed: '3', result: 'n' }),
    el('feDisplacementMap', { in: 'SourceGraphic', in2: 'n', scale: '1.6', xChannelSelector: 'R', yChannelSelector: 'G' })));
  document.body.append(svg);
}

/** Shows the saved mode on the page (html[data-hand]) */
export function applyHandMode(on = handMode()): void {
  if (typeof document === 'undefined') return;
  if (on && document.body) ensureWobbleFilter();
  if (on) document.documentElement.dataset.hand = '1';
  else delete document.documentElement.dataset.hand;
}

/** Switches the mode, saves it when the browser allows, and shows it; returns the new state */
export function toggleHandMode(): boolean {
  const on = !handMode();
  setHandMode(on);
  return on;
}
