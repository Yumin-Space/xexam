// Draws an AI answer (v0.8) from the block tree of domain/ai-markdown.ts with elements and text nodes only, never innerHTML:
// nothing in an answer is ever interpreted as HTML. Shared by AI Chat and the administrators' viewer in AI settings.
import '../styles/ai-md.css';
import { parseMarkdown, type Block, type Inline } from '../domain/ai-markdown.ts';
import { h } from '../utils/dom.ts';

function inlineNodes(c: readonly Inline[]): Node[] {
  return c.map((x) => {
    switch (x.t) {
      case 'text':
        return document.createTextNode(x.v);
      case 'code':
        return h('code', null, x.v);
      case 'b':
        return h('strong', null, inlineNodes(x.c));
      default:
        return h('em', null, inlineNodes(x.c));
    }
  });
}

function blockEl(b: Block): HTMLElement {
  switch (b.t) {
    case 'p':
      return h('p', null, inlineNodes(b.c));
    case 'h':
      return h('p', { class: 'ai-h' }, h('strong', null, inlineNodes(b.c)));
    case 'ul':
      return h('ul', null, b.items.map((it) => h('li', null, inlineNodes(it))));
    case 'ol':
      return h('ol', { start: b.start !== 1 ? String(b.start) : null }, b.items.map((it) => h('li', null, inlineNodes(it))));
    case 'pre':
      return h('pre', null, h('code', null, b.v));
    default:
      return h('hr');
  }
}

/** Puts the answer `src` into `el` (replacing what it held) */
export function fillMarkdown(el: HTMLElement, src: string): void {
  el.replaceChildren(...parseMarkdown(src).map(blockEl));
}

/** A new `div.ai-md` holding the answer `src` */
export function markdownEl(src: string, id?: string): HTMLDivElement {
  const el = h('div', { class: 'ai-md', id: id ?? null });
  fillMarkdown(el, src);
  return el;
}
