// 一个很小的 DOM 构造函数：所有文字都用 createTextNode 写入页面，永远不拼 HTML（Excel 里的内容不可信）。

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
    // style 属性（setAttribute）会被 CSP 的 style-src 'self' 拦掉；通过 CSSOM（cssText）写入则不受限制。
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

/** 按行拆成多个 <p>（空行丢掉）。 */
export function paras(text: string | null | undefined, extra?: string, lang?: string): HTMLParagraphElement[] {
  return String(text ?? '')
    .split('\n')
    .filter((l) => l.trim())
    .map((l) => h('p', { lang: lang ?? 'ja', class: extra }, l.trim()));
}

const SVGNS = 'http://www.w3.org/2000/svg';

/** 对 / 错的小图标 */
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

/** 必须存在的元素；找不到就抛错，而不是返回 null 后在别处崩。 */
export function must<T extends Element>(sel: string, root: ParentNode = document): T {
  const el = root.querySelector<T>(sel);
  if (!el) throw new Error('页面缺少元素：' + sel);
  return el;
}
