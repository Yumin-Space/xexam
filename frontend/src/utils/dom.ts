// A tiny DOM builder: all text is written into the page with createTextNode and HTML is never concatenated (content from Excel is untrusted).

export type Child = Node | string | number | false | null | undefined | Child[];
export type Props = Record<string, unknown> | null | undefined;

const PROPS = new Set(['checked', 'disabled', 'value', 'open']);

export function h<K extends keyof HTMLElementTagNameMap>(tag: K, props?: Props, ...kids: Child[]): HTMLElementTagNameMap[K] {
  const el = document.createElement(tag);
  for (const [k, v] of Object.entries(props ?? {})) {
    if (v == null || v === false) continue;
    if (k === 'class') el.className = String(v);
    else if (k.startsWith('on') && typeof v === 'function') el.addEventListener(k.slice(2), v as EventListener);
    else if (PROPS.has(k)) (el as unknown as Record<string, unknown>)[k] = v;
    // The style attribute (setAttribute) is blocked by the CSP style-src 'self'; writing through the CSSOM (cssText) is not restricted.
    else if (k === 'style') el.style.cssText = String(v);
    else el.setAttribute(k, v === true ? '' : String(v));
  }
  appendAll(el, kids);
  return el;
}

function appendAll(el: HTMLElement, kids: readonly Child[]): void {
  for (const kid of kids) {
    if (kid == null || kid === false) continue;
    if (Array.isArray(kid)) appendAll(el, kid);
    else el.append(kid instanceof Node ? kid : document.createTextNode(String(kid)));
  }
}

/** Split by line into multiple <p> elements (empty lines are dropped). */
export function paras(text: string | null | undefined, extra?: string, lang?: string): HTMLParagraphElement[] {
  return String(text ?? '')
    .split('\n')
    .filter((l) => l.trim())
    .map((l) => h('p', { lang: lang ?? 'ja', class: extra }, l.trim()));
}

const SVGNS = 'http://www.w3.org/2000/svg';

/** Small correct / incorrect icons */
export function svgMark(ok: boolean): SVGSVGElement {
  const s = document.createElementNS(SVGNS, 'svg');
  s.setAttribute('viewBox', '0 0 20 20');
  s.setAttribute('fill', 'none');
  s.setAttribute('stroke-width', '2.4');
  s.setAttribute('stroke-linecap', 'round');
  s.setAttribute('stroke-linejoin', 'round');
  s.setAttribute('aria-hidden', 'true');
  const p = document.createElementNS(SVGNS, 'path');
  p.setAttribute('d', ok ? 'M4 10.5l4 4 8-9' : 'M5 5l10 10M15 5L5 15');
  s.append(p);
  return s;
}

/** An element that must exist; throws if it is not found, instead of returning null and crashing somewhere else. */
export function must<T extends Element>(sel: string, root: ParentNode = document): T {
  const el = root.querySelector<T>(sel);
  if (!el) throw new Error('Missing page element: ' + sel);
  return el;
}
