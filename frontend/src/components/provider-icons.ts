// Provider marks for the thinking-strength row of the AI settings (v1.6): inline SVG built with createElementNS, one
// colour (fill = currentColor, so light / dark follow the theme), hidden from screen readers (the button carries the name).
// Anthropic and Google use the Simple Icons paths; OpenAI has no logo here: a small neutral spark of our own.
import type { AiEffortFamily } from '../types/api.ts';

export type EffortFamily = AiEffortFamily;
const SVGNS = 'http://www.w3.org/2000/svg';

/** The provider names are proper nouns and stay the same in every language */
export const PROVIDER_NAME: Record<EffortFamily, string> = { anthropic: 'Anthropic', openai: 'OpenAI', google: 'Google' };

// Simple Icons 16.38.0 (CC0): the anthropic and google paths, viewBox 0 0 24 24
export const ANTHROPIC_PATH = 'M17.3041 3.541h-3.6718l6.696 16.918H24Zm-10.6082 0L0 20.459h3.7442l1.3693-3.5527h7.0052l1.3693 3.5528h3.7442L10.5363 3.5409Zm-.3712 10.2232 2.2914-5.9456 2.2914 5.9456Z';
export const GOOGLE_PATH = 'M12.48 10.92v3.28h7.84c-.24 1.84-.853 3.187-1.787 4.133-1.147 1.147-2.933 2.4-6.053 2.4-4.827 0-8.6-3.893-8.6-8.72s3.773-8.72 8.6-8.72c2.6 0 4.507 1.027 5.907 2.347l2.307-2.307C18.747 1.44 16.133 0 12.48 0 5.867 0 .307 5.387.307 12s5.56 12 12.173 12c3.573 0 6.267-1.173 8.373-3.36 2.16-2.16 2.84-5.213 2.84-7.667 0-.76-.053-1.467-.173-2.053H12.48z';
// Our own neutral four-point spark (viewBox 0 0 24 24); it is not the OpenAI logo
export const SPARK_PATH = 'M12 2c.7 5.6 4.4 9.3 10 10-5.6.7-9.3 4.4-10 10-.7-5.6-4.4-9.3-10-10 5.6-.7 9.3-4.4 10-10Z';

const PATH: Record<EffortFamily, string> = { anthropic: ANTHROPIC_PATH, openai: SPARK_PATH, google: GOOGLE_PATH };

/** A 16×16 monochrome mark (fill = currentColor), aria-hidden */
export function providerIcon(f: EffortFamily, cls = 'ais-pi'): SVGSVGElement {
  const s = document.createElementNS(SVGNS, 'svg');
  s.setAttribute('viewBox', '0 0 24 24');
  s.setAttribute('width', '16');
  s.setAttribute('height', '16');
  s.setAttribute('fill', 'currentColor');
  s.setAttribute('aria-hidden', 'true');
  s.setAttribute('focusable', 'false');
  s.setAttribute('class', cls);
  const p = document.createElementNS(SVGNS, 'path');
  p.setAttribute('d', PATH[f]);
  s.appendChild(p);
  return s;
}
