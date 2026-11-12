// Small 16×16 line icons (v0.8; the v1.0 inventory of spec 4.2): inline SVG built with createElementNS, stroke = currentColor,
// hidden from screen readers (the text or the aria-label of the control names it). The shapes are in domain/mini-icon-shapes.ts.
import { MINI, type MiniIcon, type Shape } from '../domain/mini-icon-shapes.ts';

export type { MiniIcon };

const SVGNS = 'http://www.w3.org/2000/svg';

/** A 16×16 icon; `cls` defaults to the shared `us-mi` look (16px, --ink3) */
export function miniIcon(name: MiniIcon, cls = 'us-mi'): SVGSVGElement {
  const s = document.createElementNS(SVGNS, 'svg');
  s.setAttribute('viewBox', '0 0 16 16');
  s.setAttribute('width', '16');
  s.setAttribute('height', '16');
  s.setAttribute('fill', 'none');
  s.setAttribute('stroke', 'currentColor');
  s.setAttribute('stroke-width', '1.5');
  s.setAttribute('stroke-linecap', 'round');
  s.setAttribute('stroke-linejoin', 'round');
  s.setAttribute('aria-hidden', 'true');
  s.setAttribute('focusable', 'false');
  s.setAttribute('class', cls);
  for (const sh of MINI[name] as Shape[]) {
    let el: SVGElement;
    if ('d' in sh) {
      el = document.createElementNS(SVGNS, 'path');
      el.setAttribute('d', sh.d);
    } else if ('cx' in sh) {
      el = document.createElementNS(SVGNS, 'circle');
      el.setAttribute('cx', String(sh.cx));
      el.setAttribute('cy', String(sh.cy));
      el.setAttribute('r', String(sh.r));
    } else {
      el = document.createElementNS(SVGNS, 'rect');
      el.setAttribute('x', String(sh.x));
      el.setAttribute('y', String(sh.y));
      el.setAttribute('width', String(sh.w));
      el.setAttribute('height', String(sh.h));
      el.setAttribute('rx', String(sh.rx));
    }
    s.append(el);
  }
  return s;
}
