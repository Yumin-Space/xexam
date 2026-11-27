// Practice "check answer" effects, made in code (no files, no libraries):
//   · right: small flat pieces are blown up from both bottom corners, then fall calmly, spinning flat (~7 s, canvas, sprites);
//   · wrong: two dry-brush claw swipes crossing (an X), each with a tear and a thud, a red flash, a jolt and a buzz, then a low buzzer.
// The heavy pictures (the claws) are drawn once in idle time and only moved while playing. Nothing blocks the page
// (pointer-events: none: like a filter over the screen, taps go through and do not end it). It ends by itself (confetti
// ~7 s, claws ~3 s) or when the practice moves to another question. Nothing at all when the device asks for reduced motion.

import '../styles/answer-fx.css';

let current: (() => void) | null = null;

const reduced = (): boolean => window.matchMedia('(prefers-reduced-motion: reduce)').matches;

// ---- sound (made with Web Audio, no files). Every effect plays into its own bus, so ending it early silences what it booked ----
let ac: AudioContext | null = null;
function audio(): AudioContext | null {
  try {
    ac ??= new AudioContext();
    if (ac.state === 'suspended') void ac.resume();
    return ac;
  } catch {
    return null;
  }
}
function tone(out: AudioNode, freq: number, at: number, len: number, type: OscillatorType, vol: number, slideTo?: number): void {
  const ctx = out.context;
  const o = ctx.createOscillator();
  const g = ctx.createGain();
  o.type = type;
  o.frequency.setValueAtTime(freq, at);
  if (slideTo) o.frequency.exponentialRampToValueAtTime(slideTo, at + len);
  g.gain.setValueAtTime(0.0001, at);
  g.gain.exponentialRampToValueAtTime(vol, at + 0.012);
  g.gain.exponentialRampToValueAtTime(0.0001, at + len);
  o.connect(g).connect(out);
  o.start(at);
  o.stop(at + len + 0.02);
}
function noise(out: AudioNode, at: number, len: number, vol: number, type: BiquadFilterType, f0: number, f1: number): void {
  const ctx = out.context;
  const n = Math.floor(ctx.sampleRate * len);
  const buf = ctx.createBuffer(1, n, ctx.sampleRate);
  const d = buf.getChannelData(0);
  for (let i = 0; i < n; i++) d[i] = (Math.random() * 2 - 1) * (1 - i / n);
  const src = ctx.createBufferSource();
  src.buffer = buf;
  const f = ctx.createBiquadFilter();
  f.type = type;
  f.frequency.setValueAtTime(f0, at);
  f.frequency.exponentialRampToValueAtTime(f1, at + len);
  f.Q.value = 0.9;
  const g = ctx.createGain();
  g.gain.setValueAtTime(0.0001, at);
  g.gain.exponentialRampToValueAtTime(vol, at + len * 0.3);
  g.gain.exponentialRampToValueAtTime(0.0001, at + len);
  src.connect(f).connect(g).connect(out);
  src.start(at);
}
function buzzer(out: AudioNode): void {
  const t = out.context.currentTime + 0.01;
  tone(out, 196, t, 0.22, 'sawtooth', 0.09, 150);
  tone(out, 147, t + 0.24, 0.38, 'sawtooth', 0.09, 98);
  tone(out, 98, t, 0.6, 'square', 0.035);
  tone(out, 120, t, 0.25, 'sine', 0.5, 35);
}
/** a low growl that swells for `len` s */
function growl(out: AudioNode, delay: number, len: number, vol: number): void {
  const ctx = out.context;
  const t = ctx.currentTime + delay;
  const o = ctx.createOscillator();
  o.type = 'sawtooth';
  o.frequency.setValueAtTime(62, t);
  o.frequency.linearRampToValueAtTime(95, t + len);
  const lfo = ctx.createOscillator();
  lfo.frequency.value = 23;
  const lfoGain = ctx.createGain();
  lfoGain.gain.value = 0.5;
  const amp = ctx.createGain();
  amp.gain.setValueAtTime(0.0001, t);
  amp.gain.exponentialRampToValueAtTime(vol, t + len * 0.85);
  amp.gain.exponentialRampToValueAtTime(0.0001, t + len);
  const lp = ctx.createBiquadFilter();
  lp.type = 'lowpass';
  lp.frequency.setValueAtTime(260, t);
  lp.frequency.exponentialRampToValueAtTime(900, t + len);
  lfo.connect(lfoGain).connect(amp.gain);
  o.connect(lp).connect(amp).connect(out);
  o.start(t); lfo.start(t);
  o.stop(t + len + 0.05); lfo.stop(t + len + 0.05);
  noise(out, t, len, vol * 0.5, 'lowpass', 300, 1200);
}
/** the swipe and the hit when a stroke lands (tear + thud) */
function strokeSound(out: AudioNode, delay: number): void {
  const t = out.context.currentTime + delay;
  noise(out, t, 0.13, 0.5, 'bandpass', 700, 4200);
  noise(out, t + 0.1, 0.16, 0.55, 'highpass', 2500, 900);
  tone(out, 120, t + 0.1, 0.24, 'sine', 0.8, 38);
}
/** the launch: a soft pop and a short rising chime */
function pop(out: AudioNode): void {
  const t = out.context.currentTime + 0.01;
  tone(out, 220, t, 0.12, 'sine', 0.35, 80);
  noise(out, t, 0.08, 0.25, 'highpass', 2500, 900);
  [784, 988, 1319].forEach((f, i) => tone(out, f, t + 0.05 + i * 0.07, 0.35, 'triangle', 0.12));
}
function buzz(pattern: number | number[]): void {
  try { navigator.vibrate?.(pattern); } catch { /* iPhone browsers have no vibration */ }
}

/** Ends the effect on screen (if any) */
export function stopAnswerFx(): void {
  current?.();
}

/** Ends the effect and frees the bitmaps (leaving the quiz); the next practice prepares them again */
export function releaseAnswerFx(): void {
  stopAnswerFx();
  rasterCache.clear();
  rasterSize = '';
  rainSprites = null;
}

type Layer = {
  el: HTMLDivElement;
  /** the effect's sound bus (null without Web Audio) */
  out: AudioNode | null;
  /** a timer of the effect: cleared when the effect ends, early or not */
  later: (f: () => void, ms: number) => void;
  /** runs at once when the effect ends (early or not) */
  onStop: (f: () => void) => void;
  /** runs once the layer is gone */
  onEnd: (f: () => void) => void;
};

function layer(duration: number): Layer {
  stopAnswerFx();
  const el = document.createElement('div');
  el.className = 'fx-layer';
  el.setAttribute('aria-hidden', 'true');
  document.body.append(el);
  const ctx = audio();
  let bus: GainNode | null = null;
  if (ctx) {
    bus = ctx.createGain();
    bus.connect(ctx.destination);
  }
  const timers: number[] = [];
  const stops: (() => void)[] = [];
  const cleanups: (() => void)[] = [];
  let done = false;
  const end = (fast = false): void => {
    if (done) return;
    done = true;
    current = null;
    timers.forEach((id) => window.clearTimeout(id));
    // silence whatever the effect booked on the audio clock (a short fade, no click)
    if (ctx && bus) {
      bus.gain.cancelScheduledValues(0);
      bus.gain.setTargetAtTime(0, ctx.currentTime, 0.015);
    }
    stops.forEach((f) => f());
    el.classList.add('fx-out');
    window.setTimeout(() => {
      el.remove();
      bus?.disconnect();
      cleanups.forEach((f) => f());
      // no effect playing: let the audio thread sleep (audio() wakes it for the next one)
      if (!current && ac && ac.state === 'running') void ac.suspend();
    }, fast ? 200 : 380);
  };
  const later = (f: () => void, ms: number): void => { timers.push(window.setTimeout(f, ms)); };
  later(() => end(), duration - 350);
  current = () => end(true);
  return { el, out: bus, later, onStop: (f) => stops.push(f), onEnd: (f) => cleanups.push(f) };
}

function svgEl(tag: string, attrs: Record<string, string>): SVGElement {
  const e = document.createElementNS('http://www.w3.org/2000/svg', tag);
  for (const [k, v] of Object.entries(attrs)) e.setAttribute(k, v);
  return e;
}

// ---- dry-brush strokes (SVG, drawn once into a bitmap): every stroke is a torn core band plus dozens of "bristles" of
//      different lengths and inks, ragged ends, torn edges (turbulence displacement), scratches and flecks ----
type Pt = [number, number];
let uid = 0;

/** points every `step` along a polyline, keeping its corners sharp */
function resample(pts: Pt[], step: number): Pt[] {
  const out: Pt[] = [];
  for (let i = 0; i < pts.length - 1; i++) {
    const a = pts[i]!, b = pts[i + 1]!;
    const n = Math.max(2, Math.ceil(Math.hypot(b[0] - a[0], b[1] - a[1]) / step));
    for (let j = i ? 1 : 0; j <= n; j++) out.push([a[0] + ((b[0] - a[0]) * j) / n, a[1] + ((b[1] - a[1]) * j) / n]);
  }
  return out;
}
/** normals at every point and the fraction of the length reached there */
function frames(p: Pt[]): { n: Pt[]; t: number[] } {
  const n: Pt[] = [];
  const cum = [0];
  for (let i = 1; i < p.length; i++) cum.push(cum[i - 1]! + Math.hypot(p[i]![0] - p[i - 1]![0], p[i]![1] - p[i - 1]![1]));
  const len = cum[cum.length - 1] || 1;
  for (let i = 0; i < p.length; i++) {
    const a = p[Math.max(0, i - 1)]!, b = p[Math.min(p.length - 1, i + 1)]!;
    const dx = b[0] - a[0], dy = b[1] - a[1], l = Math.hypot(dx, dy) || 1;
    n.push([-dy / l, dx / l]);
  }
  return { n, t: cum.map((c) => c / len) };
}
const line = (pts: Pt[]): string => pts.map((q, i) => (i ? 'L' : 'M') + q[0].toFixed(2) + ' ' + q[1].toFixed(2)).join('');

function roughFilter(defs: SVGElement, freq: number, scale: number): string {
  const id = 'fxr' + ++uid;
  const f = svgEl('filter', { id, x: '-30%', y: '-30%', width: '160%', height: '160%' });
  f.append(
    svgEl('feTurbulence', { type: 'fractalNoise', baseFrequency: String(freq), numOctaves: '2', seed: String((Math.random() * 99) | 0) }),
    svgEl('feDisplacementMap', { in: 'SourceGraphic', scale: String(scale), xChannelSelector: 'R', yChannelSelector: 'G' }),
  );
  defs.append(f);
  return id;
}

/** one torn brush stroke along `pts` (user units), width `W`, painted from `inks` with `bristles` bristles */
function brushStroke(svg: SVGElement, pts: Pt[], W: number, inks: string[], bristles: number, rough: string): void {
  const p = resample(pts, W / 5);
  const { n, t } = frames(p);
  const at = (q: number, off: number, wob = 0): Pt => {
    // point at fraction q of the stroke, `off` across it
    let i = 0;
    while (i < t.length - 2 && t[i + 1]! < q) i++;
    const k = (q - t[i]!) / ((t[i + 1]! - t[i]!) || 1);
    const a = p[i]!, b = p[i + 1]!, na = n[i]!;
    const o = off + wob;
    return [a[0] + (b[0] - a[0]) * k + na[0] * o, a[1] + (b[1] - a[1]) * k + na[1] * o];
  };
  const pick = (): string => inks[(Math.random() * inks.length) | 0] ?? '#f00';
  const g = svgEl('g', { filter: `url(#${rough})` });
  // core band: the solid body, narrower than the bristles and much shorter, so edges and ends stay ragged
  const steps = 48;
  const c0 = 0.08, c1 = 0.86;
  const side = (sgn: number): Pt[] => Array.from({ length: steps + 1 }, (_, i) => {
    const q = c0 + ((c1 - c0) * i) / steps;
    const ends = Math.min(1, (q - c0) / 0.1, (c1 - q) / 0.12);
    return at(q, sgn * W * 0.47 * (0.5 + 0.5 * Math.max(0, ends)) * (1 + 0.015 * Math.sin(q * 29)));
  });
  g.append(svgEl('path', { d: line([...side(1), ...side(-1).reverse()]) + 'Z', fill: inks[0] ?? '#f00' }));
  /** a bristle as a tapered sliver (needle-sharp ends), from q0 to q1 at offset `off` */
  const sliver = (q0: number, q1: number, off: number, sw: number, ink: string, alpha: number, ph: number): void => {
    const L: Pt[] = [], R: Pt[] = [];
    const k = 20;
    for (let i = 0; i <= k; i++) {
      const q = q0 + ((q1 - q0) * i) / k;
      const u = i / k;
      const taper = Math.pow(Math.max(0, Math.min(1, u / 0.12, (1 - u) / 0.35)), 0.7);
      const wob = Math.sin(q * 13 + ph) * W * 0.006;
      const hw = (sw / 2) * taper;
      L.push(at(q, off + hw, wob));
      R.push(at(q, off - hw, wob));
    }
    g.append(svgEl('path', { d: line([...L, ...R.reverse()]) + 'Z', fill: ink, opacity: alpha.toFixed(2) }));
  };
  for (let b = 0; b < bristles; b++) {
    const u = (b + 0.5) / bristles - 0.5; // -0.5 .. 0.5 across
    const edge = Math.abs(u) * 2; // 0 middle .. 1 edge
    let t0 = Math.random() * (0.05 + 0.16 * edge) * 1.8 - 0.02;
    let t1 = 1 - Math.random() * (0.05 + 0.22 * edge) * 1.8 + 0.02;
    // some bristles shoot past the ends as long splinters
    if (Math.random() < 0.3) t1 += 0.05 + Math.random() * 0.12;
    if (Math.random() < 0.2) t0 -= 0.04 + Math.random() * 0.08;
    const off = u * W * (0.98 + Math.random() * 0.14);
    const sw = (W / bristles) * (1.4 + Math.random() * 1.6);
    const ph = Math.random() * 9;
    const ink = pick(), alpha = 0.75 + Math.random() * 0.25;
    if (Math.random() < 0.35 && edge > 0.55) {
      // a broken bristle: dry brush gaps along it
      const gap = t0 + (t1 - t0) * (0.3 + Math.random() * 0.45);
      sliver(t0, gap - 0.03 - Math.random() * 0.1, off, sw, ink, alpha, ph);
      sliver(gap + 0.02 + Math.random() * 0.05, t1, off, sw, ink, alpha, ph);
    } else sliver(t0, t1, off, sw, ink, alpha, ph);
  }
  // scratches: hairlines in dark and light along the stroke (the dragged-paint texture)
  for (let k = 0; k < 46; k++) {
    const q0 = Math.random() * 0.75, q1 = q0 + 0.12 + Math.random() * 0.5;
    const dark = Math.random() < 0.6;
    sliver(q0, Math.min(1.05, q1), (Math.random() - 0.5) * W * 0.95, W * (0.01 + Math.random() * 0.025),
      dark ? (Math.random() < 0.5 ? '#6E0008' : '#8F040E') : '#FF4A55', dark ? 0.3 + Math.random() * 0.3 : 0.25, Math.random() * 9);
  }
  svg.append(g);
  // flecks: small sharp shards thrown off the ends and edges
  const bits = svgEl('g', { filter: `url(#${rough})` });
  for (let k = 0; k < 26; k++) {
    const q = Math.random() < 0.5 ? Math.random() * 0.15 : 0.85 + Math.random() * 0.15;
    const c = at(q, (Math.random() < 0.5 ? -1 : 1) * W * (0.55 + Math.random() * 0.5));
    const r = W * (0.03 + Math.random() * 0.07);
    const sh: Pt[] = Array.from({ length: 4 }, (_, i): Pt => {
      const a = (i / 4) * Math.PI * 2 + Math.random();
      const rr = r * (i % 2 ? 0.5 : 1.6);
      return [c[0] + Math.cos(a) * rr, c[1] + Math.sin(a) * rr];
    });
    bits.append(svgEl('path', { d: line(sh) + 'Z', fill: pick() }));
  }
  svg.append(bits);
}

// ---- the claw bitmaps: an svg drawn into a canvas once (filters and shadows computed a single time), then only shown ----
// Only the bitmaps of the current screen size are kept: a new size (rotation, a phone's address bar, a resized window) drops the old ones.
const rasterCache = new Map<string, HTMLCanvasElement | Promise<HTMLCanvasElement>>();
let rasterSize = '';

/** the bitmap of an svg (w x h px), built and drawn once per key. Full-screen pictures stay at plain resolution: the smallest load */
function rasterSource(key: string, w: number, h: number, build: () => SVGElement): HTMLCanvasElement | Promise<HTMLCanvasElement> {
  const size = `${w}x${h}`;
  if (size !== rasterSize) {
    rasterCache.clear();
    rasterSize = size;
  }
  const hit = rasterCache.get(key);
  if (hit) return hit;
  const src = document.createElement('canvas');
  src.width = Math.max(1, w);
  src.height = Math.max(1, h);
  const job = new Promise<HTMLCanvasElement>((resolve) => {
    const svg = build();
    svg.setAttribute('xmlns', 'http://www.w3.org/2000/svg');
    svg.setAttribute('width', String(w));
    svg.setAttribute('height', String(h));
    const img = new Image();
    // kept only while this job is still the cached one (not dropped by a new size or by leaving the quiz)
    const mine = (): boolean => rasterCache.get(key) === job;
    img.onload = () => {
      try { src.getContext('2d')?.drawImage(img, 0, 0, src.width, src.height); } catch { /* not drawable */ }
      if (mine()) rasterCache.set(key, src);
      resolve(src);
    };
    img.onerror = () => {
      // not cached: the next effect tries again
      if (mine()) rasterCache.delete(key);
      resolve(src);
    };
    // a data: URL (the CSP allows data: images, not blob: ones)
    img.src = 'data:image/svg+xml;charset=utf-8,' + encodeURIComponent(new XMLSerializer().serializeToString(svg));
  });
  rasterCache.set(key, job);
  return job;
}

type Gash = [number, number, number, number];
type Claw = 'A' | 'B';

// the claw swipes of the wrong-answer timeline (fractions of the screen: x0, y0 -> x1, y1); the second is the first mirrored
const CLAW_A: Gash[] = [[0.79, 0.04, 0.10, 0.63], [0.89, 0.065, 0.105, 0.76], [0.905, 0.14, 0.26, 0.79], [0.965, 0.19, 0.19, 0.93]];
const CLAW_B: Gash[] = CLAW_A.map(([a, b, c, d]) => [1 - a, b, 1 - c, d]);

/** the cached bitmap of a claw swipe at the current screen size */
function clawSource(k: Claw): HTMLCanvasElement | Promise<HTMLCanvasElement> {
  const CW = window.innerWidth, CH = window.innerHeight;
  return rasterSource(`claw:${k}`, CW, CH, () => clawSvg(k === 'A' ? CLAW_A : CLAW_B, CW, CH));
}

/** a full-screen canvas showing claw swipe `k`, shown at once from `at` s */
function clawSwipe(k: Claw, at: number): HTMLCanvasElement {
  const w = window.innerWidth, h = window.innerHeight;
  const cv = document.createElement('canvas');
  cv.width = Math.max(1, w);
  cv.height = Math.max(1, h);
  cv.className = 'fx-clawfull fx-wipe';
  cv.style.setProperty('--fx-at', `${at}s`);
  const paint = (src: HTMLCanvasElement): void => { cv.getContext('2d')?.drawImage(src, 0, 0, cv.width, cv.height); };
  const s = clawSource(k);
  if (s instanceof HTMLCanvasElement) paint(s); else void s.then(paint);
  return cv;
}

/** one claw swipe: long torn gashes (fractions of the screen: x0, y0 -> x1, y1), each hooking more steeply at its end; the dark
 *  edge and the red glow are part of the picture (no CSS filter on the moving layer) */
function clawSvg(lines: Gash[], CW: number, CH: number): SVGElement {
  const svg = svgEl('svg', { viewBox: `0 0 ${CW} ${CH}` });
  const defs = svgEl('defs', {});
  svg.append(defs);
  const rough = roughFilter(defs, 0.05, 3);
  const shadowId = 'fxs' + ++uid;
  const shadow = svgEl('filter', { id: shadowId, x: '-10%', y: '-10%', width: '120%', height: '120%' });
  shadow.append(
    svgEl('feDropShadow', { dx: '2', dy: '3', stdDeviation: '0', 'flood-color': '#000', 'flood-opacity': '.8' }),
    svgEl('feDropShadow', { dx: '0', dy: '0', stdDeviation: '9', 'flood-color': '#ff0a0a', 'flood-opacity': '.45' }),
  );
  defs.append(shadow);
  const body = svgEl('g', { filter: `url(#${shadowId})` });
  svg.append(body);
  const blood = ['#C00A16', '#9A0410', '#D8121E', '#7A0008', '#E01A26', '#B00010'];
  const gw = Math.max(13, Math.min(CW, CH) * 0.045);
  lines.forEach(([x0, y0, x1, y1], k) => {
    const a: Pt = [x0 * CW, y0 * CH], e: Pt = [x1 * CW, y1 * CH];
    const m: Pt = [a[0] + (e[0] - a[0]) * 0.8 - Math.sign(e[0] - a[0]) * CW * 0.035, a[1] + (e[1] - a[1]) * 0.8];
    const outer = k === 0 || k === lines.length - 1;
    brushStroke(body, [a, m, e], gw * (outer ? 0.85 : 1), blood, 16, rough);
  });
  return svg;
}

function pulse(el: HTMLElement | null | undefined, cls: string, ms: number): void {
  if (!el) return;
  el.classList.remove(cls);
  void el.offsetWidth;
  el.classList.add(cls);
  window.setTimeout(() => el.classList.remove(cls), ms);
}

/** shakes the effect layer and the question card, stronger with `px`. Not the body: a transformed body would move every
 *  fixed bar of the page while it shakes */
function jolt(els: (HTMLElement | null | undefined)[], px: number, ms: number): void {
  for (const b of els) {
    if (!b) continue;
    b.style.setProperty('--fx-j', px + 'px');
    b.style.setProperty('--fx-jt', ms + 'ms');
    b.classList.remove('fx-jolt');
    void b.offsetWidth;
    b.classList.add('fx-jolt');
    window.setTimeout(() => b.classList.remove('fx-jolt'), ms + 50);
  }
}

// ---- right answer: blown up, then a calm rain of small flat pieces ----
// Dots, squares, triangles and hexagons are blown up from both bottom corners (a pop), slow down near the top, then fall at a
// calm, constant speed, drifting a little sideways and spinning flat (spin speeding up to one turn a second). About 150 pieces,
// gone in ~7 s. Light green shades and amber.
const RAIN_COLORS = ['#7CD35C', '#6CC24A', '#3F8F2E', '#2E7A22', '#F2A93B', '#F0A02A'];
const RAIN_SIZE = 6; // px
const RAIN_TOTAL = 7000; // ms
type Rain = { x: number; y: number; vx: number; vy: number; dx: number; fall: number; a: number; va: number; aa: number; spr: HTMLCanvasElement; born: number; sz: number; fl: number; vf: number; round: boolean };
let rainSprites: HTMLCanvasElement[] | null = null;

/** the pieces, each colour x shape drawn once (dot, square, triangle, hexagon) */
function rainSpriteSet(): HTMLCanvasElement[] {
  if (rainSprites) return rainSprites;
  const k = 3; // sprite resolution: sharp at any screen ratio
  const n = RAIN_SIZE;
  const out: HTMLCanvasElement[] = [];
  for (const c of RAIN_COLORS) {
    for (let shape = 0; shape < 4; shape++) {
      const cv = document.createElement('canvas');
      cv.width = cv.height = n * k;
      const g = cv.getContext('2d');
      if (g) {
        g.scale(k, k);
        g.fillStyle = c;
        if (shape === 0) { g.beginPath(); g.arc(n / 2, n / 2, n / 2, 0, Math.PI * 2); g.fill(); }
        else if (shape === 1) g.fillRect(0, 0, n, n);
        else if (shape === 2) { g.beginPath(); g.moveTo(n / 2, 0); g.lineTo(n, n * 0.87); g.lineTo(0, n * 0.87); g.closePath(); g.fill(); }
        else {
          g.beginPath();
          for (let i = 0; i < 6; i++) { const a = (i / 6) * Math.PI * 2; g.lineTo(n / 2 + Math.cos(a) * n / 2, n / 2 + Math.sin(a) * n / 2); }
          g.closePath();
          g.fill();
        }
      }
      out.push(cv);
    }
  }
  rainSprites = out;
  return out;
}

function confetti(): void {
  const { el, out, onEnd } = layer(RAIN_TOTAL);
  const W = window.innerWidth, H = window.innerHeight;
  const dpr = Math.min(window.devicePixelRatio || 1, 2);
  const cv = document.createElement('canvas');
  cv.width = Math.round(W * dpr);
  cv.height = Math.round(H * dpr);
  el.append(cv);
  const g = cv.getContext('2d');
  if (!g) return;
  const sprites = rainSpriteSet();
  const r = (lo: number, hi: number): number => lo + Math.random() * (hi - lo);
  const deg = Math.PI / 180;
  // sprayed out of the bottom corners like a confetti cannon: a very fast jet that the air brakes hard (reach ~ v0 / AIR),
  // light gravity turns the pieces over at the top, then the calm fall
  const AIR = 3.2; // 1/s
  const GLIGHT = 260; // px/s²
  const v0 = AIR * H * 1.2; // the fastest go past the top edge
  const ps: Rain[] = [];
  const blow = (x: number, angle: number, spread: number, speed: number, n: number, born: number): void => {
    for (let i = 0; i < n; i++) {
      const an = angle + (Math.random() - 0.5) * spread;
      const sp = speed * r(0.72, 1.05);
      const si = (Math.random() * sprites.length) | 0;
      ps.push({
        x, y: H + RAIN_SIZE, vx: Math.cos(an) * sp, vy: Math.sin(an) * sp,
        dx: r(-50, 50), fall: r(50, 150), // the calm rain afterwards: drift and fall speed (px/s)
        a: r(0, 360) * deg, va: r(0, 180) * deg, aa: r(180, 540) * deg, // rad, rad/s, rad/s²
        spr: sprites[si]!, born: born + Math.random() * 0.25, // a spray over a quarter second
        sz: r(6, 12) / RAIN_SIZE, fl: r(0, 6.28), vf: r(1.5, 4.5), round: si % 4 === 0, // size, slow flip (not for dots)
      });
    }
  };
  // aimed at the middle of the screen, high up: the pieces meet above the centre, then spread and fall
  const aim = (x: number): number => Math.atan2(-H * 0.92, W * 0.5 - x);
  blow(W * 0.03, aim(W * 0.03), 0.26, v0, 60, 0);
  blow(W * 0.97, aim(W * 0.97), 0.26, v0, 60, 0);
  blow(W * 0.3, aim(W * 0.3), 0.2, v0 * 0.97, 15, 0.35);
  blow(W * 0.7, aim(W * 0.7), 0.2, v0 * 0.97, 15, 0.35);
  if (out) pop(out);
  const maxSpin = 360 * deg;
  const half = (RAIN_SIZE * 3) / 2;
  const t0 = performance.now();
  let last = t0, raf = 0;
  const step = (now: number): void => {
    const t = (now - t0) / 1000;
    const dt = Math.min(0.05, (now - last) / 1000);
    last = now;
    g.setTransform(1, 0, 0, 1, 0, 0);
    g.clearRect(0, 0, cv.width, cv.height);
    const left = RAIN_TOTAL / 1000 - t;
    g.globalAlpha = left < 1 ? Math.max(0, left) : 1;
    let w = 0;
    for (let i = 0; i < ps.length; i++) {
      const p = ps[i]!;
      if (t < p.born) { ps[w++] = p; continue; }
      p.va = Math.min(maxSpin, p.va + p.aa * dt);
      p.a += p.va * dt;
      // the jet: the air brakes it hard, light gravity turns it over; then it settles into its own calm fall and drift
      const brake = Math.exp(-AIR * dt);
      if (p.vy < p.fall) p.vy = Math.min(p.fall, p.vy * brake + GLIGHT * dt);
      p.vx = p.dx + (p.vx - p.dx) * brake;
      p.x += p.vx * dt;
      p.y += p.vy * dt;
      if (p.y > H + RAIN_SIZE) continue; // gone
      ps[w++] = p;
      p.fl += p.vf * dt;
      const k = (p.sz * dpr) / 3, sq = p.round ? 1 : Math.cos(p.fl); // a flat piece turning over looks like a thin diamond
      const c = Math.cos(p.a) * k, sn = Math.sin(p.a) * k;
      g.setTransform(c, sn, -sn * sq, c * sq, p.x * dpr, p.y * dpr);
      g.drawImage(p.spr, -half, -half);
    }
    ps.length = w;
    if (ps.length && left > 0) raf = requestAnimationFrame(step);
  };
  raf = requestAnimationFrame(step);
  onEnd(() => cancelAnimationFrame(raf));
}

/** Wrong answer: a claw swipes across the whole screen (four gashes, top right to bottom left), then a second one the other way
 *  (an X), each landing like a hit (tear and thud with a low growl, red flash, jolt), then a low buzzer and a shake of the card.
 *  ~3 s; ending it early also cancels its sounds, buzzes and shakes still to come. */
function claws(card?: HTMLElement | null): void {
  const { el, out, later, onStop } = layer(3000);
  const T1 = 0.04, T2 = 0.42;
  // each claw appears with its hit, 0.1 s after its swipe sound starts
  el.append(clawSwipe('A', T1 + 0.1), clawSwipe('B', T2 + 0.1));
  if (out) {
    growl(out, 0, 0.35, 0.4);
    growl(out, T2 - 0.04, 0.3, 0.45);
  }
  for (const h of [T1, T2]) {
    const flash = Object.assign(document.createElement('div'), { className: 'fx-redhit' });
    flash.style.setProperty('--fx-at', `${h + 0.1}s`);
    el.append(flash);
    if (out) strokeSound(out, h);
    later(() => { buzz([60]); jolt([el, card], 9, 300); }, (h + 0.1) * 1000);
  }
  later(() => { if (out) buzzer(out); pulse(card, 'fx-shake', 600); }, (T2 + 0.18) * 1000);
  onStop(() => {
    buzz(0);
    card?.classList.remove('fx-jolt', 'fx-shake');
  });
}

/**
 * The feedback of a checked practice answer: confetti for a right one, two claw swipes for a wrong one. `card` is the question
 * card (it shakes); `key` names the practice (the effects do not depend on earlier answers). One effect at a time; nothing
 * when the device asks for reduced motion.
 */
export function answerFx(ok: boolean, opts: { card?: HTMLElement | null; key?: unknown }): void {
  if (reduced()) return;
  if (ok) confetti(); else claws(opts.card);
}

/** builds the heavy pictures (the claws, the pieces) in idle time, so the first effect does not stutter; nothing to do when they
 *  are ready for the current screen size */
export function prewarmAnswerFx(): void {
  if (reduced()) return;
  if (rainSprites && rasterSize === `${window.innerWidth}x${window.innerHeight}` && rasterCache.has('claw:A') && rasterCache.has('claw:B')) return;
  const idle = (f: () => void): void => {
    const ric = (window as unknown as { requestIdleCallback?: (cb: () => void, o?: object) => void }).requestIdleCallback;
    if (ric) ric(f, { timeout: 1500 }); else window.setTimeout(f, 200);
  };
  const jobs: (() => void)[] = [
    () => { rainSpriteSet(); },
    () => { void clawSource('A'); },
    () => { void clawSource('B'); },
  ];
  let i = 0;
  const next = (): void => { const j = jobs[i++]; if (j) { j(); idle(next); } };
  idle(next);
}
