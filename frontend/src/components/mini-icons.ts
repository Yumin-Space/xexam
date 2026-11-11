// Small 16×16 line icons (v0.8) for the bank-access controls of User management, the AI+ checkbox and the AI pages: inline
// SVG built with createElementNS, stroke = currentColor, hidden from screen readers (the text next to them names the control).
const SVGNS = 'http://www.w3.org/2000/svg';

type Shape = { d: string } | { cx: number; cy: number; r: number } | { x: number; y: number; w: number; h: number; rx: number };

const MINI = {
  /** A key: "Manage banks" */
  key: [{ cx: 5.2, cy: 10.8, r: 2.7 }, { d: 'M7.2 8.8 13.5 2.5' }, { d: 'M11.2 4.8l1.8 1.8' }, { d: 'M9.6 6.4l1.4 1.4' }],
  /** Two ticks: "Select all" */
  checkAll: [{ d: 'M1.8 8.6l2.6 2.6L9.6 5' }, { d: 'M7.6 10.8l.6.6 6-6.6' }],
  /** A square with a minus: "Deselect all" */
  squareMinus: [{ x: 2.5, y: 2.5, w: 11, h: 11, rx: 2.5 }, { d: 'M5.5 8h5' }],
  /** A clock with a plus: "Include banks imported later" */
  clockPlus: [{ d: 'M13.4 7.2A5.5 5.5 0 1 0 8.6 13.5' }, { d: 'M8 5v3l1.8 1.2' }, { d: 'M12.5 10.5v4' }, { d: 'M10.5 12.5h4' }],
  /** An "i" in a circle: the explanation buttons and the administrator notice */
  info: [{ cx: 8, cy: 8, r: 6 }, { d: 'M8 7.3v3.7' }, { d: 'M8 5v.2' }],
  /** A four-point star and a small one: AI+ */
  sparkle: [{ d: 'M6.5 1.8c.4 2.7 1.6 3.9 4.3 4.3-2.7.4-3.9 1.6-4.3 4.3-.4-2.7-1.6-3.9-4.3-4.3 2.7-.4 3.9-1.6 4.3-4.3Z' }, { d: 'M12 10.2c.2 1.2.7 1.7 1.9 1.9-1.2.2-1.7.7-1.9 1.9-.2-1.2-.7-1.7-1.9-1.9 1.2-.2 1.7-.7 1.9-1.9Z' }],
  /** A speech bubble: a memory saved from a chat */
  chat: [{ d: 'M2.5 3.5h11v7.2H7.2L4.5 13v-2.3h-2z' }],
  /** A pencil: a memory added by hand, Edit */
  pencil: [{ d: 'M10.6 2.6l2.8 2.8-7.6 7.6H3v-2.8z' }, { d: 'M9.2 4l2.8 2.8' }],
  /** A bin: delete */
  trash: [{ d: 'M2.8 4.3h10.4' }, { d: 'M6.2 4.3V2.8h3.6v1.5' }, { d: 'M4.2 4.3l.6 8.9h6.4l.6-8.9' }],
  /** A plus: New chat, Add */
  plus: [{ d: 'M8 3v10' }, { d: 'M3 8h10' }],
  /** An arrow left: Back */
  back: [{ d: 'M9.8 3.5 5.3 8l4.5 4.5' }],
  /** An arrow down: Jump to latest */
  down: [{ d: 'M8 3v9.5' }, { d: 'M4 8.8l4 3.9 4-3.9' }],
  /** Two overlapping sheets: Copy */
  copy: [{ x: 5.5, y: 5.5, w: 8, h: 8, rx: 1.6 }, { d: 'M10.5 3.2V3a1.5 1.5 0 0 0-1.5-1.5H3A1.5 1.5 0 0 0 1.5 3v6A1.5 1.5 0 0 0 3 10.5h.2' }],
  /** A light bulb: AI hint */
  bulb: [{ d: 'M8 1.8a4.3 4.3 0 0 0-2.6 7.7c.5.4.8.9.8 1.5v.5h3.6V11c0-.6.3-1.1.8-1.5A4.3 4.3 0 0 0 8 1.8Z' }, { d: 'M6.3 13.1h3.4' }, { d: 'M6.9 14.6h2.2' }],
  /** An open book: AI explanation */
  book: [{ d: 'M8 4.3C6.6 3.2 4.6 2.8 2 2.9v9.5c2.6-.1 4.6.3 6 1.3 1.4-1 3.4-1.4 6-1.3V2.9c-2.6-.1-4.6.3-6 1.3Z' }, { d: 'M8 4.3v9.4' }],
} satisfies Record<string, Shape[]>;

export type MiniIcon = keyof typeof MINI;

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
