// Copying text (moved out of pages/ai.ts in v0.9 so AI Chat, the AI popover and the AI summary overlay share it): the async
// clipboard where it is allowed, else a hidden textarea and execCommand (older browsers, plain-http hosts). The fallback
// textarea goes into the open modal dialog when there is one: a modal <dialog> makes everything outside it inert, so a
// textarea appended to the body could not be selected there.
import { h } from './dom.ts';

export async function copyText(text: string): Promise<boolean> {
  try {
    if (navigator.clipboard && window.isSecureContext) {
      await navigator.clipboard.writeText(text);
      return true;
    }
  } catch {
    // fall through to the old way
  }
  const area = h('textarea', { class: 'sr-only', readonly: true, 'aria-hidden': 'true', tabindex: '-1', 'data-aip-copy': '' });
  area.value = text;
  const prev = document.activeElement instanceof HTMLElement ? document.activeElement : null;
  (document.querySelector('dialog[open]') ?? document.body).append(area);
  area.select();
  let ok = false;
  try {
    ok = document.execCommand('copy');
  } catch {
    ok = false;
  }
  area.remove();
  // select() moved the focus into the textarea: give it back (the popover's Copy button keeps it)
  if (prev && prev.isConnected) prev.focus({ preventScroll: true });
  return ok;
}
