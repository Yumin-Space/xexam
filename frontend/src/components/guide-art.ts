// Original drawings for the iPhone / iPad setup steps (v3.1): a simplified phone with its toolbar and the generic share glyph,
// the "Add to Home Screen" row of the share sheet, and the "Add" button of its next screen. Built with the DOM (no markup
// strings); colours come from classes in styles/guide.css, so light and dark follow the theme. The drawings show where to tap,
// nothing more: no screenshots and no marks of any other product. Each is a `role="img"` with the step text as its label.
import { t } from '../i18n/index.ts';

const NS = 'http://www.w3.org/2000/svg';

type Attrs = Record<string, string | number>;

function node<K extends keyof SVGElementTagNameMap>(tag: K, attrs: Attrs, ...kids: Array<SVGElement | string>): SVGElementTagNameMap[K] {
  const el = document.createElementNS(NS, tag);
  for (const [k, v] of Object.entries(attrs)) el.setAttribute(k, String(v));
  for (const kid of kids) el.append(typeof kid === 'string' ? document.createTextNode(kid) : kid);
  return el;
}

function canvas(w: number, h: number, label: string | undefined, ...kids: SVGElement[]): SVGSVGElement {
  const attrs: Attrs = { viewBox: `0 0 ${w} ${h}`, class: 'gd-art', focusable: 'false' };
  if (label) {
    attrs.role = 'img';
    attrs['aria-label'] = label;
  } else {
    attrs['aria-hidden'] = 'true';
  }
  return node('svg', attrs, ...kids);
}

/** A grey line standing in for text */
const bar = (x: number, y: number, w: number, cls = 'ga-bar'): SVGElement => node('rect', { x, y, width: w, height: 5, rx: 2.5, class: cls });

/** The generic share glyph: an open box with an arrow leaving it upwards, centred on (cx, cy) */
function shareGlyph(cx: number, cy: number, cls: string): SVGElement {
  return node('g', { class: cls, transform: `translate(${cx} ${cy})` },
    node('path', { d: 'M-5 -2v9h10v-9' }),
    node('path', { d: 'M0 5.5V-9' }),
    node('path', { d: 'M-3.6 -5.4L0 -9l3.6 3.6' }));
}

/** A square with a plus inside, centred on (cx, cy) */
function plusSquare(cx: number, cy: number, cls: string): SVGElement {
  return node('g', { class: cls, transform: `translate(${cx} ${cy})` },
    node('rect', { x: -7, y: -7, width: 14, height: 14, rx: 3.5 }),
    node('path', { d: 'M0 -3.5v7M-3.5 0h7' }));
}

/**
 * A phone with the Share button highlighted: at the bottom of the screen (Safari), or inside the address bar at the top
 * (Chrome). `label` is the step text, read out for the picture.
 */
export function artShare(where: 'bottom' | 'top', label?: string): SVGSVGElement {
  const body: SVGElement[] = [
    node('rect', { x: 56, y: 4, width: 108, height: 132, rx: 15, class: 'ga-frame' }),
    // page content
    bar(70, where === 'top' ? 48 : 22, 52), bar(70, where === 'top' ? 60 : 34, 80), bar(70, where === 'top' ? 72 : 46, 66),
    node('rect', { x: 70, y: where === 'top' ? 86 : 60, width: 80, height: 22, rx: 5, class: 'ga-fill' }),
  ];
  if (where === 'bottom') {
    body.push(
      node('rect', { x: 61, y: 108, width: 98, height: 23, rx: 11, class: 'ga-toolbar' }),
      // The other toolbar buttons keep clear of the highlight ring around Share (r 14 at x 110)
      node('circle', { cx: 72, cy: 119.5, r: 3.2, class: 'ga-dot' }),
      node('circle', { cx: 86, cy: 119.5, r: 3.2, class: 'ga-dot' }),
      node('circle', { cx: 134, cy: 119.5, r: 3.2, class: 'ga-dot' }),
      node('circle', { cx: 148, cy: 119.5, r: 3.2, class: 'ga-dot' }),
      node('circle', { cx: 110, cy: 119.5, r: 14, class: 'ga-hi' }),
      shareGlyph(110, 119.5, 'ga-glyph ga-glyph-hi'));
  } else {
    body.push(
      node('rect', { x: 62, y: 16, width: 96, height: 24, rx: 12, class: 'ga-toolbar' }),
      bar(74, 26, 40),
      node('circle', { cx: 140, cy: 28, r: 14, class: 'ga-hi' }),
      shareGlyph(140, 28, 'ga-glyph ga-glyph-hi'));
  }
  return canvas(220, 140, label, ...body);
}

/** The share sheet with its "Add to Home Screen" row highlighted */
export function artAddToHome(label?: string): SVGSVGElement {
  const text = node('text', { x: 24, y: 60, class: 'ga-text', 'font-size': 12.5, 'font-weight': 600, 'dominant-baseline': 'central' }, t('guide.art.addToHome'));
  return canvas(240, 120, label,
    node('rect', { x: 8, y: 6, width: 224, height: 108, rx: 14, class: 'ga-frame' }),
    node('rect', { x: 16, y: 14, width: 208, height: 24, rx: 8, class: 'ga-fill' }),
    bar(26, 24, 60), plusSquare(206, 26, 'ga-glyph ga-glyph-dim'),
    node('rect', { x: 16, y: 44, width: 208, height: 32, rx: 8, class: 'ga-hi ga-hi-box' }),
    text,
    plusSquare(206, 60, 'ga-glyph ga-glyph-hi'),
    node('rect', { x: 16, y: 82, width: 208, height: 24, rx: 8, class: 'ga-fill' }),
    bar(26, 92, 76), plusSquare(206, 94, 'ga-glyph ga-glyph-dim'));
}

/** The screen after it: the "Add" button in its top right corner highlighted */
export function artAddButton(label?: string): SVGSVGElement {
  return canvas(240, 120, label,
    node('rect', { x: 8, y: 6, width: 224, height: 108, rx: 14, class: 'ga-frame' }),
    // the header: a quiet left action, a title, and the highlighted button
    bar(22, 20, 34), bar(98, 20, 44),
    node('rect', { x: 170, y: 11, width: 52, height: 24, rx: 12, class: 'ga-hi ga-hi-box' }),
    node('text', { x: 196, y: 27.5, class: 'ga-text', 'font-size': 12.5, 'font-weight': 600, 'text-anchor': 'middle' }, t('guide.art.add')),
    // the app icon and its name
    node('rect', { x: 24, y: 52, width: 40, height: 40, rx: 10, class: 'ga-icon' }),
    node('rect', { x: 76, y: 52, width: 140, height: 18, rx: 6, class: 'ga-fill' }),
    bar(84, 59, 70),
    bar(76, 82, 110), bar(76, 94, 80));
}
